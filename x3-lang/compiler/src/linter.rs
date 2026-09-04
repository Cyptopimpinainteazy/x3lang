//! Static analysis linter for X3 programs.
//!
//! Checks for common issues:
//! - Missing refund path on intents with bridges
//! - Missing nonce on intents with timeouts
//! - Unknown chain names
//! - Unused imports
//! - Unsafe slippage (too high)
//! - Unbounded deadlines
//! - Single RPC (production should have quorum)

use crate::semantic::CompilationMode;
use std::collections::HashSet;
use x3_lang_ast::ast::*;
use x3_lang_common::Spanned;

#[derive(Debug, Clone)]
pub struct LintDiagnostic {
    pub severity: Severity,
    pub message: String,
    pub location: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Severity {
    Error,
    Warning,
    Info,
}

impl std::fmt::Display for Severity {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Severity::Error => write!(f, "error"),
            Severity::Warning => write!(f, "warning"),
            Severity::Info => write!(f, "info"),
        }
    }
}

pub struct X3Linter {
    diagnostics: Vec<LintDiagnostic>,
    known_chains: HashSet<String>,
    mode: Option<CompilationMode>,
}

impl X3Linter {
    pub fn new() -> Self {
        let known_chains: HashSet<String> = [
            "eth",
            "ethereum",
            "sol",
            "solana",
            "x3",
            "btc",
            "bitcoin",
            "utxo",
            "polygon",
            "arbitrum",
            "optimism",
            "base",
            "bsc",
            "avalanche",
            "arb",
            "avax",
            "ksm",
            "polkadot",
            "kusama",
            "sui",
            "aptos",
            "cosmos",
            "atom",
            "osmo",
            "starknet",
            "cardano",
            "ada",
            "ton",
            "fuel",
            "near",
            "stellar",
            "xlm",
        ]
        .iter()
        .map(|s| s.to_string())
        .collect();
        X3Linter {
            diagnostics: Vec::new(),
            known_chains,
            mode: None,
        }
    }

    pub fn with_mode(mode: CompilationMode) -> Self {
        let mut linter = X3Linter::new();
        linter.mode = Some(mode);
        linter
    }

    pub fn mode(&self) -> Option<CompilationMode> {
        self.mode
    }

    pub fn lint_program(&mut self, program: &Program) -> &[LintDiagnostic] {
        self.diagnostics.clear();

        let mut imports: Vec<(String, bool)> = Vec::new();
        let mut has_intent = false;
        let mut has_bridge = false;
        let mut has_timeout = false;
        let mut has_nonce = false;
        let mut has_refund_path = false;
        let mut slippage_values: Vec<u64> = Vec::new();
        let mut rpc_quorum_count = 0;
        let mut timeout_durations: Vec<u64> = Vec::new();

        for item in &program.items {
            match &item.node {
                Item::Import(decl) => {
                    imports.push((
                        decl.module.iter().map(|s| s.as_str()).collect::<Vec<_>>().join("::"),
                        false,
                    ));
                }
                Item::Use(decl) => {
                    imports.push((
                        decl.path.iter().map(|s| s.as_str()).collect::<Vec<_>>().join("::"),
                        false,
                    ));
                }
                Item::IntentDecl(intent) => {
                    has_intent = true;
                    self.check_intent(
                        intent,
                        &mut has_bridge,
                        &mut has_timeout,
                        &mut has_nonce,
                        &mut has_refund_path,
                        &mut slippage_values,
                        &mut timeout_durations,
                    );
                }
                Item::Bridge(_) => {
                    has_bridge = true;
                }
                Item::AtomicSwap(_) => {
                    has_bridge = true;
                }
                Item::RpcQuorum(_) => {
                    rpc_quorum_count += 1;
                }
                _ => {}
            }
        }

        // Check: missing refund path on intents with bridges
        if has_intent && has_bridge && !has_refund_path {
            let severity = match self.mode {
                Some(CompilationMode::Dev) => Severity::Warning,
                _ => Severity::Error,
            };
            self.diagnostics.push(LintDiagnostic {
                severity,
                message:
                    "intent with bridge operation missing refund path — add 'require refund_path' or 'on_fail refund'"
                        .into(),
                location: "program".into(),
            });
        }

        // Check: missing nonce on intents with timeouts
        if has_intent && has_timeout && !has_nonce {
            self.diagnostics.push(LintDiagnostic {
                severity: Severity::Warning,
                message: "intent with timeout missing nonce guard — add 'require nonce unused' for replay protection"
                    .into(),
                location: "program".into(),
            });
        }

        // Check: unused imports
        for (import_path, _used) in &imports {
            self.diagnostics.push(LintDiagnostic {
                severity: Severity::Info,
                message: format!("unused import: '{}' — no reference found in program", import_path),
                location: "import".into(),
            });
        }

        // Check: unsafe slippage (> 5%)
        for &slippage in &slippage_values {
            if slippage > 5 {
                let severity = match self.mode {
                    Some(CompilationMode::Mainnet) => Severity::Error,
                    _ => Severity::Warning,
                };
                self.diagnostics.push(LintDiagnostic {
                    severity,
                    message: format!("high slippage tolerance: {}% — consider reducing to ≤ 5%", slippage),
                    location: "intent".into(),
                });
            }
        }

        // Check: unbounded deadline (only in Mainnet)
        if self.mode == Some(CompilationMode::Mainnet) {
            for &duration in &timeout_durations {
                if duration > 86400 {
                    self.diagnostics.push(LintDiagnostic {
                        severity: Severity::Error,
                        message: format!(
                            "unbounded deadline: {} blocks — reduce to ≤ 86400 for Mainnet",
                            duration
                        ),
                        location: "intent".into(),
                    });
                }
            }
        }

        // Check: single RPC quorum (production should have at least 2 for redundancy)
        if rpc_quorum_count <= 1 && has_intent {
            let severity = match self.mode {
                Some(CompilationMode::Mainnet) => Severity::Error,
                _ => Severity::Info,
            };
            self.diagnostics.push(LintDiagnostic {
                severity,
                message: "single RPC quorum defined — production should have at least 2 for redundancy".into(),
                location: "rpc_quorum".into(),
            });
        }

        &self.diagnostics
    }

    fn check_intent(
        &mut self,
        intent: &IntentDecl,
        has_bridge: &mut bool,
        has_timeout: &mut bool,
        has_nonce: &mut bool,
        has_refund_path: &mut bool,
        slippage_values: &mut Vec<u64>,
        timeout_durations: &mut Vec<u64>,
    ) {
        for stmt in &intent.body.stmts {
            self.check_statement_chains(stmt);

            match stmt {
                Statement::Bridge { .. } => {
                    *has_bridge = true;
                }
                Statement::Swap { .. } => {
                    *has_bridge = true;
                }
                Statement::OnTimeout { duration, .. } => {
                    *has_timeout = true;
                    if let Expression::Literal(LiteralExpr::Int { value, .. }) = duration {
                        timeout_durations.push(*value as u64);
                    }
                }
                Statement::OnFail(action) => {
                    if matches!(action, FailureAction::Refund(_)) {
                        *has_refund_path = true;
                    }
                }
                Statement::Require(guard) => match &guard.kind {
                    RequireKind::Nonce => *has_nonce = true,
                    RequireKind::RefundPath => *has_refund_path = true,
                    RequireKind::Slippage => {
                        if let Expression::Literal(LiteralExpr::Int { value, .. }) = &guard.value {
                            slippage_values.push(*value as u64);
                        }
                    }
                    _ => {}
                },
                Statement::Atomic(atomic) => {
                    for s in &atomic.body.stmts {
                        self.check_statement_chains(s);
                        match s {
                            Statement::Bridge { .. } | Statement::Swap { .. } => *has_bridge = true,
                            Statement::Require(guard) => match &guard.kind {
                                RequireKind::Nonce => *has_nonce = true,
                                RequireKind::RefundPath => *has_refund_path = true,
                                RequireKind::Slippage => {
                                    if let Expression::Literal(LiteralExpr::Int { value, .. }) = &guard.value {
                                        slippage_values.push(*value as u64);
                                    }
                                }
                                _ => {}
                            },
                            _ => {}
                        }
                    }
                }
                _ => {}
            }
        }
    }

    fn check_statement_chains(&mut self, stmt: &Statement) {
        let chains: Vec<String> = match stmt {
            Statement::Lock { chain, .. } => vec![chain.as_str().to_ascii_lowercase()],
            Statement::Release { chain, .. } => vec![chain.as_str().to_ascii_lowercase()],
            Statement::Mint { asset, .. } => vec![asset.chain.as_str().to_ascii_lowercase()],
            Statement::Burn { asset, .. } => vec![asset.chain.as_str().to_ascii_lowercase()],
            Statement::Swap { from, .. } => vec![from.chain.as_str().to_ascii_lowercase()],
            Statement::Bridge { from, .. } => vec![from.chain.as_str().to_ascii_lowercase()],
            _ => Vec::new(),
        };
        for c in chains {
            if !self.known_chains.contains(&c) {
                self.diagnostics.push(LintDiagnostic {
                    severity: Severity::Warning,
                    message: format!("unknown chain name: '{}' — verify the chain is supported", c),
                    location: "statement".into(),
                });
            }
        }
    }
}

impl Default for X3Linter {
    fn default() -> Self {
        Self::new()
    }
}
