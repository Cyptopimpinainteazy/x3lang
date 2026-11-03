//! Compiler cost model — spec PHASE 35.
//!
//! The phase asks the compiler to estimate gas, compute, network calls, latency,
//! proof cost, verification cost and expected fees, and to include those costs in
//! economic assertions "when appropriate".
//!
//! This module estimates what the compiler can *measure* and refuses to invent the
//! rest. Every figure it reports is derived by walking the artifact the compiler
//! just wrote, with the same framing rules and the same weight table the VM uses,
//! so an estimate cannot disagree with a charge:
//!
//! | component | basis |
//! |---|---|
//! | instructions | one per frame in the emitted stream |
//! | base weight | `spec::opcodes::base_gas_cost`, the table `vm/src/executor.rs` charges from |
//! | payload bytes | the length-prefixed frames, excluding their alignment padding |
//! | host-facing instructions | the opcodes that leave the VM for a host adapter, named in `HOST_FACING` |
//! | proof bytes carried | the source-finality and transfer proofs inside bridge payloads |
//! | artifact bytes | the file itself |
//!
//! What it does **not** estimate, and says so in the report rather than printing a
//! plausible number: EVM gas, SVM compute units, cross-domain latency and expected
//! fees. Each needs a table or a quote the compiler does not have, and a figure
//! nobody can check is worse than an admitted gap (the same rule that keeps the
//! mainnet checks from accepting unbacked quantities).

use crate::emitter::instructions;
use crate::spec::opcodes::{is_payload_opcode, opcode_name, BASE_WEIGHT_TABLE_IS_THE_CHARGE};
use x3_lang_common::X3Error;

/// One opcode's share of an artifact.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OpcodeCost {
    pub name: &'static str,
    pub opcode: u8,
    pub count: usize,
    pub weight: u128,
}

/// What a program is expected to cost, and what could not be estimated.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CostEstimate {
    pub instruction_count: usize,
    /// Sum of `base_gas_cost` over the instructions — the VM's own charge for them.
    pub base_weight: u128,
    /// Payload bytes carried by the frames, without their padding.
    pub payload_bytes: usize,
    /// Instructions that leave the VM for a host adapter.
    pub host_facing: usize,
    /// Proof bytes the artifact carries in its bridge payloads.
    pub proof_bytes: usize,
    pub artifact_bytes: usize,
    /// Per opcode, heaviest first, then by name: deterministic ordering.
    pub per_opcode: Vec<OpcodeCost>,
    /// Components the compiler will not guess, with the reason.
    pub not_estimated: Vec<(&'static str, &'static str)>,
}

impl CostEstimate {
    /// The PHASE 35 report, with the basis of every figure in brackets.
    pub fn report(&self) -> String {
        let mut out = String::new();
        out.push_str("Estimated execution (each figure with its basis):\n");
        out.push_str(&format!(
            "  instructions          {:>8}   [frames in the emitted stream]\n",
            self.instruction_count
        ));
        out.push_str(&format!(
            "  base weight           {:>8}   [sum of base_gas_cost — {}]\n",
            self.base_weight, BASE_WEIGHT_TABLE_IS_THE_CHARGE
        ));
        out.push_str(&format!(
            "  payload bytes         {:>8}   [length-prefixed frames, padding excluded]\n",
            self.payload_bytes
        ));
        out.push_str(&format!(
            "  host-facing           {:>8}   [instructions that leave the VM: {}]\n",
            self.host_facing,
            HOST_FACING_NAMES.join(", ")
        ));
        out.push_str(&format!(
            "  proof bytes carried   {:>8}   [finality and transfer proofs in bridge payloads]\n",
            self.proof_bytes
        ));
        out.push_str(&format!(
            "  artifact bytes        {:>8}   [the file this compiler writes]\n",
            self.artifact_bytes
        ));
        out.push_str("  heaviest instructions:\n");
        for entry in self.per_opcode.iter().take(5) {
            out.push_str(&format!(
                "    0x{:02x}  {:<20} {:>4} ×  = {:>7}\n",
                entry.opcode, entry.name, entry.count, entry.weight
            ));
        }
        out.push_str("Not estimated, and why:\n");
        for (component, reason) in &self.not_estimated {
            out.push_str(&format!("  {component:<22} {reason}\n"));
        }
        out
    }
}

/// Instructions whose execution asks a host adapter to do something — the phase's
/// "network calls". The set is named rather than derived, because it is a statement
/// about which opcodes leave the VM, and a reader has to be able to argue with it.
pub const HOST_FACING: &[u8] = &[
    crate::spec::opcodes::BRIDGE,
    crate::spec::opcodes::CALL_HOST,
    crate::spec::opcodes::ORACLE_REQUEST,
    crate::spec::opcodes::GPU_DISPATCH,
    crate::spec::opcodes::PROOF_VERIFY,
    crate::spec::opcodes::STORAGE_OP,
    crate::spec::opcodes::MEMPOOL_SCAN,
    crate::spec::opcodes::PATHFIND,
    crate::spec::opcodes::TRADING_BRIDGE,
];

/// The names of `HOST_FACING`, for the report.
pub const HOST_FACING_NAMES: &[&str] = &[
    "BRIDGE",
    "CALL_HOST",
    "ORACLE_REQUEST",
    "GPU_DISPATCH",
    "PROOF_VERIFY",
    "STORAGE_OP",
    "MEMPOOL_SCAN",
    "PATHFIND",
    "TRADING_BRIDGE",
];

/// Estimate what the emitted artifact will cost.
pub fn estimate_artifact(bytecode: &[u8]) -> Result<CostEstimate, X3Error> {
    let walked = instructions(bytecode)?;
    let mut counted: Vec<(u8, usize)> = Vec::new();
    let mut base_weight: u128 = 0;
    let mut payload_bytes = 0usize;
    let mut host_facing = 0usize;
    let mut proof_bytes = 0usize;

    for instruction in &walked {
        base_weight = base_weight.saturating_add(crate::spec::opcodes::base_gas_cost(instruction.opcode));
        if is_payload_opcode(instruction.opcode, true) {
            payload_bytes += instruction.payload.len();
        }
        if HOST_FACING.contains(&instruction.opcode) {
            host_facing += 1;
        }
        // Proof bytes the artifact *carries*: the two proof fields inside a bridge
        // payload. What a chain will demand at settlement is a different number,
        // and one this compiler does not know.
        if instruction.opcode == crate::spec::opcodes::BRIDGE {
            if let Ok(payload) = x3_lang_common::decode_bridge_payload(instruction.payload) {
                proof_bytes += payload.source_finality_proof.len() + payload.transfer_proof.len();
            }
        }
        match counted.iter_mut().find(|(opcode, _)| *opcode == instruction.opcode) {
            Some((_, count)) => *count += 1,
            None => counted.push((instruction.opcode, 1)),
        }
    }

    let mut per_opcode: Vec<OpcodeCost> = counted
        .into_iter()
        .map(|(opcode, count)| OpcodeCost {
            name: opcode_name(opcode),
            opcode,
            count,
            weight: crate::spec::opcodes::base_gas_cost(opcode).saturating_mul(count as u128),
        })
        .collect();
    per_opcode.sort_by(|left, right| right.weight.cmp(&left.weight).then_with(|| left.name.cmp(right.name)));

    Ok(CostEstimate {
        instruction_count: walked.len(),
        base_weight,
        payload_bytes,
        host_facing,
        proof_bytes,
        artifact_bytes: bytecode.len(),
        per_opcode,
        not_estimated: vec![
            (
                "EVM gas",
                "no per-instruction table for a foreign VM; an invented figure is a claim nobody can check",
            ),
            ("SVM compute", "same: X3IR instructions are not Solana instructions"),
            ("cross-domain latency", "a network fact, not a compiler fact"),
            (
                "expected fees",
                "needs a quote; a declared venue `fee_bps` is a ceiling the program states, not a fee",
            ),
        ],
    })
}
