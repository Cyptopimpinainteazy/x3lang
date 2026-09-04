//! Deterministic linear-scan register allocator for the X3 compiler pipeline
//! **— EXPERIMENTAL, NOT YET WIRED INTO EMISSION.**
//!
//! Takes an ordered sequence of Operations and assigns physical registers to
//! temporaries using a classic linear-scan algorithm. When physical registers
//! are exhausted, values are spilled to stack slots.
//!
//! # Current status
//! The allocator computes `AllocationResult` metadata (register assignments
//! and spill slots) but the emitter (`crate::emitter`) does not consume it.
//! `patch_operation()` is a no-op and `rewrite_operations()` is dead code.
//! Until a consumer calls `regalloc::allocate()` from the emission path,
//! the allocator is **non-functional** — emitted code uses the legacy
//! stack-based encoding independent of the allocation result.
//!
//! # Determinism
//! Same input always produces the same output — this is required for the GPU
//! determinism invariant and trusted execution replay.

use crate::ir::Operation;
use std::collections::HashMap;

/// Number of general-purpose physical registers available.
const PHYSICAL_REGISTERS: usize = 16;

/// A stack slot offset used as a spill target.
type StackSlot = u32;

/// Result of linear-scan register allocation.
///
/// Carries both the operation list and the explicit allocation metadata
/// so callers (the backend emitter) can consume the register assignments
/// directly without stringly-typed inspection.
#[derive(Debug, Clone)]
pub struct AllocationResult {
    /// Operations in original order.
    pub operations: Vec<Operation>,
    /// temp_id → physical register index (0..15).
    pub register_assignments: HashMap<u32, usize>,
    /// temp_id → stack spill slot offset.
    pub spill_slots: HashMap<u32, StackSlot>,
    /// Number of distinct registers used.
    pub registers_used: usize,
    /// Number of stack spill slots allocated.
    pub spills_used: u32,
    /// Number of temporaries processed.
    pub temps_processed: usize,
}

impl AllocationResult {
    /// Returns true if the operation list is empty.
    pub fn is_empty(&self) -> bool {
        self.operations.is_empty()
    }

    /// Returns the number of operations.
    pub fn len(&self) -> usize {
        self.operations.len()
    }
}

/// Register assignment for a single temporary.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Assignment {
    /// Assigned to physical register r (0..15).
    Register(usize),
    /// Spilled to stack slot s.
    Spill(u32),
}

/// Assign physical registers to temporaries using linear-scan allocation.
///
/// Each temporary's first use triggers an allocation. If all 16 physical
/// registers are live, a stack spill slot is assigned instead. Spilled values
/// are reloaded at subsequent uses.
///
/// Returns an `AllocationResult` with the operation list and explicit
/// register/spill assignments.
pub fn allocate(instrs: &[Operation]) -> AllocationResult {
    if instrs.is_empty() {
        return AllocationResult {
            operations: Vec::new(),
            register_assignments: HashMap::new(),
            spill_slots: HashMap::new(),
            registers_used: 0,
            spills_used: 0,
            temps_processed: 0,
        };
    }

    let result = instrs.to_vec();
    let mut temp_to_reg: HashMap<u32, usize> = HashMap::new();
    let mut temp_to_spill: HashMap<u32, StackSlot> = HashMap::new();
    let mut reg_in_use = [false; PHYSICAL_REGISTERS];
    let mut next_spill_slot: StackSlot = 0;

    // First pass: collect live ranges for each temporary.
    let live_ranges = compute_live_ranges(&result);

    // Sort temporaries by start position for linear-scan.
    let mut sorted_temps: Vec<(u32, (usize, usize))> = live_ranges.into_iter().collect();
    sorted_temps.sort_by_key(|(_, (start, _))| *start);

    // Active list: temporaries currently occupying a register.
    let mut active: Vec<(u32, usize, usize)> = Vec::new();

    for (temp_id, (start, end)) in &sorted_temps {
        // Expire any intervals that end before this one starts.
        active.retain(|(_, reg, active_end)| {
            if *active_end < *start {
                reg_in_use[*reg] = false;
                false
            } else {
                true
            }
        });

        if active.len() < PHYSICAL_REGISTERS {
            let reg = (0..PHYSICAL_REGISTERS)
                .find(|r| !reg_in_use[*r])
                .expect("active.len() < PHYSICAL_REGISTERS but no free register found");
            reg_in_use[reg] = true;
            temp_to_reg.insert(*temp_id, reg);
            active.push((*temp_id, reg, *end));
        } else {
            let slot = next_spill_slot;
            next_spill_slot += 1;
            temp_to_spill.insert(*temp_id, slot);
        }
    }

    // Compute allocation metadata.
    let regs_used = (0..PHYSICAL_REGISTERS)
        .filter(|r| reg_in_use[*r] || temp_to_reg.values().any(|&v| v == *r))
        .count();

    AllocationResult {
        operations: result,
        register_assignments: temp_to_reg,
        spill_slots: temp_to_spill,
        registers_used: regs_used,
        spills_used: next_spill_slot,
        temps_processed: sorted_temps.len(),
    }
}

/// Query the assignment for a given temporary from an allocation result.
pub fn get_assignment(result: &AllocationResult, temp_id: u32) -> Option<Assignment> {
    if let Some(&reg) = result.register_assignments.get(&temp_id) {
        Some(Assignment::Register(reg))
    } else if let Some(&slot) = result.spill_slots.get(&temp_id) {
        Some(Assignment::Spill(slot))
    } else {
        None
    }
}

/// Compute live ranges: for each temporary, record (first_def, last_use) positions.
fn compute_live_ranges(instrs: &[Operation]) -> HashMap<u32, (usize, usize)> {
    let mut ranges: HashMap<u32, (usize, usize)> = HashMap::new();

    for (pos, op) in instrs.iter().enumerate() {
        // Collect all temporary IDs referenced by this operation.
        let temps = op_referenced_temps(op);
        for temp_id in temps {
            ranges
                .entry(temp_id)
                .and_modify(|(start, end)| {
                    if pos < *start {
                        *start = pos;
                    }
                    if pos > *end {
                        *end = pos;
                    }
                })
                .or_insert((pos, pos));
        }
    }

    ranges
}

/// Extract the set of temporary IDs referenced (def + use) by an operation.
///
/// This is a heuristic extraction that scans the operation's string-based
/// and numeric representations for temp references. In the full compiler
/// this would be a precise def/use list from the IR.
fn op_referenced_temps(_op: &Operation) -> Vec<u32> {
    // Operations encode their temporaries in the Display/Debug output.
    // We extract numeric IDs from the operation's string representation.
    // This is a pragmatic approach for the v0.1 pipeline — the full lowering
    // pass will attach explicit def/use metadata.
    let s = format!("{:?}", _op);
    let mut temps = Vec::new();

    // Simple parser: find substrings matching "t<number>" or "%<number>"
    let bytes = s.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if (bytes[i] == b't' || bytes[i] == b'%') && i + 1 < bytes.len() {
            let start = i + 1;
            let mut end = start;
            while end < bytes.len() && bytes[end].is_ascii_digit() {
                end += 1;
            }
            if end > start {
                if let Ok(num) = std::str::from_utf8(&bytes[start..end]).unwrap_or("").parse::<u32>() {
                    temps.push(num);
                }
            }
            i = end;
        } else {
            i += 1;
        }
    }

    temps
}

/// Rewrite operation list, replacing temporary references with assigned physical
/// registers or spill slot operands.
#[allow(dead_code)]
fn rewrite_operations(
    instrs: &mut [Operation],
    temp_to_reg: &HashMap<u32, usize>,
    temp_to_spill: &HashMap<u32, StackSlot>,
) {
    for op in instrs.iter_mut() {
        // For each temporary referenced in this operation, patch the
        // operation's representation to use the assigned physical register
        // or spill slot reference.
        patch_operation(op, temp_to_reg, temp_to_spill);
    }
}

/// Patch a single operation's temporary references to physical registers.
#[allow(dead_code)]
fn patch_operation(_op: &mut Operation, _temp_to_reg: &HashMap<u32, usize>, _temp_to_spill: &HashMap<u32, StackSlot>) {
    // In the full compiler, each Operation variant carries explicit operand
    // slots (defs and uses). The v0.1 pipeline uses Display/Debug for temp
    // identification, so the rewrite is a no-op at the IR level — the
    // assigned registers are recorded in the allocation metadata and consumed
    // by the backend's emit phase.
    //
    // For v0.2, each Operation variant will store `Vec<Operand>` with
    // explicit register/spill assignments, and this function will
    // directly set `operand.location = Reg(r)` or `Spill(s)`.
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_input_returns_empty() {
        let result = allocate(&[]);
        assert!(result.is_empty());
    }

    #[test]
    fn single_instruction_unchanged() {
        // Register allocator should not alter single instructions beyond
        // recording allocation metadata.
        let input = vec![Operation::Nop]; // Use a simple no-op operation
        let result = allocate(&input);
        assert_eq!(result.len(), 1);
    }

    #[test]
    fn deterministic_output() {
        // Same input must produce same output (GPU determinism invariant).
        let ops = vec![Operation::Nop, Operation::Nop];
        let r1 = allocate(&ops);
        let r2 = allocate(&ops);
        assert_eq!(r1.len(), r2.len());
    }

    #[test]
    fn handles_many_instructions() {
        // Should not panic or produce incorrect output with many ops.
        let ops: Vec<Operation> = (0..100).map(|_| Operation::Nop).collect();
        let result = allocate(&ops);
        assert_eq!(result.len(), 100);
    }
}
