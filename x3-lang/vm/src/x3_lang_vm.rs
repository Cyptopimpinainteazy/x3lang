//! X3 VM core data structures and helper functions.

use crate::executor::{ExecError, ExecResult};
use std::sync::Arc;
use x3_lang_common::{AssetOpPayload, BridgePayload};

pub type Register = u128; // 128-bit to match u256-like operations; adjust as needed

#[derive(Clone, Debug)]
pub struct InstructionStream(Arc<Vec<u8>>);

impl InstructionStream {
    pub fn new(bytes: Vec<u8>) -> Self {
        InstructionStream(Arc::new(bytes))
    }
    pub fn len(&self) -> usize {
        self.0.len()
    }
    pub fn as_slice(&self) -> &[u8] {
        &self.0
    }
}

#[derive(Clone, Debug)]
pub struct VMConfig {
    pub max_registers: usize,
    pub max_stack: usize,
    pub max_memory_pages: usize,
}

impl Default for VMConfig {
    fn default() -> Self {
        VMConfig {
            max_registers: 32,
            max_stack: 65536,
            max_memory_pages: 256,
        }
    }
}

/// A snapshot of VM state taken at ATOMIC_BEGIN, used to restore state
/// on ATOMIC_ROLLBACK.
#[derive(Clone, Debug)]
pub struct VmSnapshot {
    pub registers: Vec<Register>,
    pub memory: Vec<u8>,
    pub asset_ops_len: usize,
    pub bridge_receipts_len: usize,
    pub pc: usize,
    pub call_stack: Vec<usize>,
    pub instruction_count: u128,
}

#[derive(Clone, Debug)]
pub struct VMState {
    pub registers: Vec<Register>,
    pub vector_registers: Vec<[u8; 16]>,
    pub pc: usize,
    pub sp: usize,
    pub fp: usize,
    pub gas: u128,
    pub memory: Vec<u8>,
    pub atomic_stack: Vec<usize>, // stack of pc for atomic begin
    pub call_stack: Vec<usize>,
    pub asset_ops: Vec<AssetOpPayload>,
    pub bridge_ops: Vec<BridgePayload>,
    pub bridge_receipts: Vec<Vec<u8>>,
    pub paused: bool,
    /// Atomic scope rollback snapshot (set by ATOMIC_BEGIN, consumed by ATOMIC_ROLLBACK).
    pub atomic_snapshot: Option<VmSnapshot>,
    /// Failure handler PC targets registered by ON_FAIL.
    pub failure_handlers: Vec<usize>,
    /// If set, instruction_count exceeding this causes a timeout panic.
    pub timeout_deadline: Option<u128>,
    /// Monotonically increasing instruction counter for timeout enforcement.
    pub instruction_count: u128,
}

impl VMState {
    pub fn new(config: &VMConfig, initial_gas: u128) -> Self {
        VMState {
            registers: vec![0u128; config.max_registers],
            vector_registers: vec![[0u8; 16]; 8],
            pc: 0,
            sp: 0,
            fp: 0,
            gas: initial_gas,
            memory: vec![0u8; config.max_memory_pages * 64 * 1024],
            atomic_stack: Vec::new(),
            call_stack: Vec::new(),
            asset_ops: Vec::new(),
            bridge_ops: Vec::new(),
            bridge_receipts: Vec::new(),
            paused: false,
            atomic_snapshot: None,
            failure_handlers: Vec::new(),
            timeout_deadline: None,
            instruction_count: 0,
        }
    }
}

pub struct VM {
    pub config: VMConfig,
    pub state: VMState,
    pub code: InstructionStream,
    pub bridge: Box<dyn crate::bridge::BridgeAdapter>,
}

impl VM {
    pub fn new(code: Vec<u8>, cfg: VMConfig, initial_gas: u128) -> Self {
        let code_deposit = code.len() as u128;
        let gas = initial_gas.saturating_sub(code_deposit);
        VM {
            config: cfg.clone(),
            state: VMState::new(&cfg, gas),
            code: InstructionStream::new(code),
            bridge: Box::new(crate::bridge::DryRunBridge),
        }
    }

    /// Build a VM with an explicit bridge backend selection.
    ///
    /// `BackendMode::DryRun` is functionally equivalent to
    /// [`VM::new`] — it just constructs the same `DryRunBridge` but
    /// makes the choice explicit. `BackendMode::Production` requires
    /// a fully-wired production adapter; the call fails closed if the
    /// adapter is `None` rather than silently falling back to dry-run.
    ///
    /// A **code-deposit cost** equal to `bytecode.len()` is deducted
    /// from `initial_gas` at construction time, same as [`VM::new`].
    pub fn with_bridge(
        code: Vec<u8>,
        cfg: VMConfig,
        initial_gas: u128,
        bridge_config: BridgeConfig,
    ) -> ExecResult<Self> {
        let bridge: Box<dyn crate::bridge::BridgeAdapter> = match bridge_config.mode {
            BackendMode::DryRun => Box::new(crate::bridge::DryRunBridge),
            BackendMode::Production => bridge_config.adapter.ok_or_else(|| {
                ExecError::Panic(
                    "X3_BRIDGE_BACKEND_REQUIRED: BackendMode::Production requires a wired \
                     production adapter (ProductionBridgeAdapter wrapping an EVM or SVM \
                     backend); refusing to silently fall back to dry-run"
                        .to_string(),
                )
            })?,
        };
        let code_deposit = code.len() as u128;
        let gas = initial_gas.saturating_sub(code_deposit);
        Ok(VM {
            config: cfg.clone(),
            state: VMState::new(&cfg, gas),
            code: InstructionStream::new(code),
            bridge,
        })
    }

    pub fn execute(&mut self) -> ExecResult<()> {
        self.verify_and_execute()
    }

    pub fn verify_and_execute(&mut self) -> ExecResult<()> {
        crate::verifier::verify(&self.code).map_err(|err| ExecError::Panic(format!("X3_VERIFY_FAILED: {err:?}")))?;
        self.execute_unverified()
    }

    pub(crate) fn execute_unverified(&mut self) -> ExecResult<()> {
        crate::executor::execute(self)
    }
}

/// Bridge backend selection.
///
/// `DryRun` is the default and the only mode the CLI sandbox and unit
/// tests exercise today. `Production` requires a real
/// `BridgeAdapter` (an EVM or SVM backend wired with a verifier and a
/// receipt store). The two are kept separate so a typo or a missing
/// config cannot quietly turn a production call into a dry-run.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum BackendMode {
    /// In-process dry-run bridge. No RPC, no light client, no real
    /// finality. Bridge operations are recorded in `state.bridge_ops`
    /// and acknowledged with `dry-run-bridge_transfer:...` receipts.
    DryRun,
    /// Production bridge. The `BridgeConfig` must carry a wired
    /// `BridgeAdapter` (an EVM or SVM backend); otherwise
    /// [`VM::with_bridge`] fails closed.
    Production,
}

impl Default for BackendMode {
    fn default() -> Self {
        BackendMode::DryRun
    }
}

/// Configuration passed to [`VM::with_bridge`].
///
/// `mode` selects the backend family; `adapter` is the actual
/// `BridgeAdapter` to install on the VM (typically a
/// `ProductionBridgeAdapter<EthereumLightClientVerifier, _>` or
/// `ProductionBridgeAdapter<SolanaLightClientVerifier, _>`). When
/// `mode == DryRun` the adapter is ignored.
///
/// `BridgeConfig` is not `Clone` because the production adapter is a
/// boxed trait object; clone is unnecessary in the call paths we
/// support (constructed once at startup, consumed by `with_bridge`).
pub struct BridgeConfig {
    /// Backend mode.
    pub mode: BackendMode,
    /// Production adapter. Required iff `mode == Production`.
    pub adapter: Option<Box<dyn crate::bridge::BridgeAdapter>>,
}

impl std::fmt::Debug for BridgeConfig {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("BridgeConfig")
            .field("mode", &self.mode)
            .field("adapter", &self.adapter.as_ref().map(|_| "<BridgeAdapter>"))
            .finish()
    }
}

impl Default for BridgeConfig {
    fn default() -> Self {
        Self {
            mode: BackendMode::DryRun,
            adapter: None,
        }
    }
}

impl BridgeConfig {
    /// Build a dry-run config (default).
    pub fn dry_run() -> Self {
        Self::default()
    }

    /// Build a production config that wraps the supplied production
    /// adapter. The adapter is `Box`-erased to a `BridgeAdapter`
    /// trait object; callers in production code usually pass a
    /// `ProductionBridgeAdapter<...>`.
    pub fn with_production_adapter<A>(adapter: A) -> Self
    where
        A: crate::bridge::BridgeAdapter + 'static,
    {
        Self {
            mode: BackendMode::Production,
            adapter: Some(Box::new(adapter)),
        }
    }
}
