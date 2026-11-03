//! JIT skeleton for X3
//!
//! This module houses structures and glue for recording hot paths and compiling
//! them to native code. For v0.1 we provide hints and a simple interface used by
//! runtime to mark hot basic blocks.

use std::collections::HashMap;

pub struct JitCompiler {
    pub threshold: u32,
    hit_counts: HashMap<usize, u32>,
}

impl JitCompiler {
    pub fn new(threshold: u32) -> Self {
        JitCompiler {
            threshold,
            hit_counts: HashMap::new(),
        }
    }
    pub fn maybe_compile(&mut self, _code: &[u8], pc: usize) -> bool {
        let hits = self.hit_counts.entry(pc).or_insert(0);
        *hits = hits.saturating_add(1);
        *hits >= self.threshold
    }
}

/// Representation of a compiled function.
pub struct CompiledFn {
    pub entry_pc: usize,
    pub code: Vec<u8>,
}
