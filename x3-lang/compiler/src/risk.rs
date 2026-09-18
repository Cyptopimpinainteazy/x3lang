//! Risk scoring for X3 intents.
//!
//! Computes a numeric risk score from intent constraints,
//! chain risk, bridge risk, solver risk, and liquidity risk.

use crate::semantic::CompilationMode;
use crate::trading_semantic::analyze_trading;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use x3_lang_ast::ast::*;
use x3_lang_ast::trading::{InvariantKind, TradeStmt};
use x3_lang_common::Spanned;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RiskReport {
    pub overall_score: u32,
    pub max_score: u32,
    pub categories: HashMap<String, u32>,
    pub details: Vec<String>,
}

pub struct RiskScorer {
    chain_risk: HashMap<String, u32>,
    bridge_risk: HashMap<String, u32>,
    mode: Option<CompilationMode>,
}

impl RiskScorer {
    pub fn new() -> Self {
        let mut chain_risk = HashMap::new();
        chain_risk.insert("eth".into(), 10);
        chain_risk.insert("ethereum".into(), 10);
        chain_risk.insert("sol".into(), 25);
        chain_risk.insert("solana".into(), 25);
        chain_risk.insert("btc".into(), 5);
        chain_risk.insert("bitcoin".into(), 5);
        chain_risk.insert("x3".into(), 15);
        chain_risk.insert("polygon".into(), 30);
        chain_risk.insert("arbitrum".into(), 20);
        chain_risk.insert("optimism".into(), 25);
        chain_risk.insert("base".into(), 30);
        chain_risk.insert("bsc".into(), 35);
        chain_risk.insert("avalanche".into(), 30);
        chain_risk.insert("sui".into(), 40);
        chain_risk.insert("aptos".into(), 40);
        chain_risk.insert("starknet".into(), 35);
        chain_risk.insert("near".into(), 35);
        chain_risk.insert("cosmos".into(), 25);

        let mut bridge_risk = HashMap::new();
        bridge_risk.insert("x3".into(), 5);
        bridge_risk.insert("wormhole".into(), 20);
        bridge_risk.insert("layerzero".into(), 15);
        bridge_risk.insert("axelar".into(), 10);
        bridge_risk.insert("native".into(), 5);
        bridge_risk.insert("btc-relay".into(), 5);

        RiskScorer {
            chain_risk,
            bridge_risk,
            mode: None,
        }
    }

    pub fn with_mode(mode: CompilationMode) -> Self {
        let mut scorer = RiskScorer::new();
        scorer.mode = Some(mode);
        scorer
    }

    pub fn mode(&self) -> Option<CompilationMode> {
        self.mode
    }

    pub fn score_program(&self, program: &Program) -> RiskReport {
        let mut categories: HashMap<String, u32> = HashMap::new();
        let mut details: Vec<String> = Vec::new();

        let mut chains_used: Vec<String> = Vec::new();
        let mut bridges_used: Vec<String> = Vec::new();
        let mut has_liquidity_check = false;
        let mut has_profit_check = false;
        let mut timeout_secs: u64 = 0;
        let mut slippage_bps: u64 = 0;
        let mut has_refund = false;
        let mut has_nonce = false;
        let mut has_route_score = false;

        let mut has_intent_decl = false;
        for item in &program.items {
            if let Item::IntentDecl(intent) = &item.node {
                has_intent_decl = true;
                for stmt in &intent.body.stmts {
                    match stmt {
                        Statement::Lock { chain, .. } => {
                            let c = chain.as_str().to_ascii_lowercase();
                            if !chains_used.contains(&c) {
                                chains_used.push(c);
                            }
                        }
                        Statement::Release { chain, .. } => {
                            let c = chain.as_str().to_ascii_lowercase();
                            if !chains_used.contains(&c) {
                                chains_used.push(c);
                            }
                        }
                        Statement::Bridge { via, from, to, .. } => {
                            let b = via.as_str().to_ascii_lowercase();
                            if !bridges_used.contains(&b) {
                                bridges_used.push(b);
                            }
                            for c in [from.chain.as_str(), to.chain.as_str()] {
                                let c = c.to_ascii_lowercase();
                                if !chains_used.contains(&c) {
                                    chains_used.push(c);
                                }
                            }
                        }
                        Statement::Swap { from, .. } => {
                            let c = from.chain.as_str().to_ascii_lowercase();
                            if !chains_used.contains(&c) {
                                chains_used.push(c);
                            }
                        }
                        Statement::Require(guard) => match &guard.kind {
                            RequireKind::BridgeLiquidity => has_liquidity_check = true,
                            RequireKind::Profit => has_profit_check = true,
                            RequireKind::RefundPath => has_refund = true,
                            RequireKind::Nonce => has_nonce = true,
                            RequireKind::RouteScore => has_route_score = true,
                            RequireKind::Slippage => {
                                if let Expression::Literal(LiteralExpr::Int { value, .. }) = &guard.value {
                                    // This guard's literal is a whole percent (e.g. `require
                                    // slippage < 5` means 5%); normalize to basis points so it
                                    // shares a scale with trading-core-v1's native bps policy.
                                    slippage_bps = (*value as u64).saturating_mul(100);
                                }
                            }
                            _ => {}
                        },
                        Statement::OnTimeout { duration, .. } => {
                            if let Expression::Literal(LiteralExpr::Int { value, .. }) = duration {
                                timeout_secs = *value as u64;
                            }
                            if let Expression::Literal(LiteralExpr::Duration { value, unit }) = duration {
                                match unit {
                                    x3_lang_common::DurationUnit::Seconds => timeout_secs = *value,
                                    x3_lang_common::DurationUnit::Minutes => timeout_secs = value * 60,
                                    x3_lang_common::DurationUnit::Hours => timeout_secs = value * 3600,
                                    _ => {}
                                }
                            }
                        }
                        Statement::OnFail(action) => {
                            if matches!(action, FailureAction::Refund(_)) {
                                has_refund = true;
                            }
                        }
                        _ => {}
                    }
                }
            }
        }

        // Trading Core v1 risk scoring. This is a structurally different AST
        // (Item::AtomicTrade / Item::TradeRiskPolicy, not Item::IntentDecl),
        // so it feeds the same shared categories above through its own
        // signals rather than being silently invisible to every check in
        // this function (see PR #216's description: this was a known,
        // unfixed gap — a well-formed trading-core-v1 program used to score
        // as if it had no chains, no profit check, no refund path, no
        // replay protection, and no liquidity check, regardless of what it
        // actually declared).
        let mut has_trading_core = false;
        let mut trading_gas_declared = false;
        let mut trading_oracle_firewall = false;
        let mut trading_cumulative_loss_ceiling = false;
        if let Ok(symbols) = analyze_trading(program, self.mode.unwrap_or(CompilationMode::Dev)) {
            for item in &program.items {
                let Item::AtomicTrade(trade) = &item.node else {
                    continue;
                };
                has_trading_core = true;

                let policy = symbols.policies.get(&trade.risk_policy);

                for stmt in &trade.body {
                    match stmt {
                        TradeStmt::Swap {
                            from_asset, to_asset, ..
                        } => {
                            has_liquidity_check = true; // min_output is a required field, not optional
                            for asset_name in [from_asset, to_asset] {
                                if let Some(asset) = symbols.assets.get(asset_name) {
                                    let c = asset.chain.as_str().to_ascii_lowercase();
                                    if !chains_used.contains(&c) {
                                        chains_used.push(c);
                                    }
                                }
                            }
                        }
                        TradeStmt::Bridge {
                            from_asset,
                            to_asset,
                            via,
                            ..
                        } => {
                            let b = via.as_str().to_ascii_lowercase();
                            if !bridges_used.contains(&b) {
                                bridges_used.push(b);
                            }
                            for asset_name in [from_asset, to_asset] {
                                if let Some(asset) = symbols.assets.get(asset_name) {
                                    let c = asset.chain.as_str().to_ascii_lowercase();
                                    if !chains_used.contains(&c) {
                                        chains_used.push(c);
                                    }
                                }
                            }
                        }
                        TradeStmt::RequireMinNetProfit { .. } => has_profit_check = true,
                        TradeStmt::AssertInvariant {
                            kind: InvariantKind::Solvent,
                        } => {
                            has_profit_check = true; // stronger than a single-asset profit floor
                        }
                        TradeStmt::EmitReceipt => has_nonce = true, // replay protection is receipt-based, not a nonce guard
                        _ => {}
                    }
                }

                // Atomic execution always rolls back every leg on any guard
                // failure (verified: trading_execution.rs's
                // `failure_restores_the_pre_execution_state`) — strictly
                // stronger than the intent-DSL's optional refund path.
                has_refund = true;

                if let Some(policy) = policy {
                    trading_gas_declared = true; // max_gas is a required policy field
                    if policy.min_profit.is_some() {
                        has_profit_check = true;
                    }
                    if timeout_secs == 0 {
                        timeout_secs = 1; // deadline is a required policy field; not statically evaluated here
                    }
                    slippage_bps = slippage_bps.max(policy.max_slippage_bps as u64);
                    if policy.max_oracle_deviation_bps.is_some() {
                        trading_oracle_firewall = true;
                    }
                    if policy.max_cumulative_loss.is_some() {
                        trading_cumulative_loss_ceiling = true;
                    }
                } else {
                    details.push(format!(
                        "trading risk: atomic trade '{}' references unresolved risk policy '{}'",
                        trade.name.as_str(),
                        trade.risk_policy.as_str()
                    ));
                }
            }
        }

        if has_trading_core {
            let gas_score: u32 = if trading_gas_declared { 0 } else { 25 };
            categories.insert("gas_risk".into(), gas_score);
            if !trading_gas_declared {
                details.push("gas risk: atomic trade has no resolved risk policy declaring max_gas".into());
            }

            let oracle_score: u32 = if trading_oracle_firewall { 0 } else { 20 };
            categories.insert("oracle_risk".into(), oracle_score);
            if !trading_oracle_firewall {
                details.push(
                    "oracle risk: no max_oracle_deviation_bps ceiling — a single venue quote is trusted uncorroborated"
                        .into(),
                );
            }

            let cumulative_loss_score: u32 = if trading_cumulative_loss_ceiling { 0 } else { 15 };
            categories.insert("cumulative_loss_risk".into(), cumulative_loss_score);
            if !trading_cumulative_loss_ceiling {
                details.push("cumulative loss risk: no max_cumulative_loss ceiling — repeated small losing trades are not circuit-broken".into());
            }
        }

        // B-52 configuration items risk scoring
        for item in &program.items {
            match &item.node {
                Item::SolverMarket(market) => {
                    if market.min_reputation < 100 {
                        *categories.entry("solver_risk".to_string()).or_insert(0) += 20;
                        details.push(format!("solver market min_reputation {} is low", market.min_reputation));
                    }
                }
                Item::RelayerSwarm(swarm) => {
                    if swarm.quorum_numerator == 1 && swarm.quorum_denominator == 1 {
                        *categories.entry("relayer_risk".to_string()).or_insert(0) += 15;
                        details.push("single relayer quorum 1_of_1 — no redundancy".into());
                    }
                }
                Item::RpcQuorum(quorum) => {
                    if quorum.require_numerator == 1 && quorum.require_denominator == 1 {
                        *categories.entry("rpc_risk".to_string()).or_insert(0) += 15;
                        details.push("single RPC node — no consensus".into());
                    }
                }
                Item::RiskPolicy(policy) => {
                    if policy.max_slippage > 500 {
                        *categories.entry("slippage_risk".to_string()).or_insert(0) += 10;
                        details.push(format!("max slippage {} bps is high", policy.max_slippage));
                    }
                }
                Item::PrivacyBlock(privacy) => {
                    if !privacy.encrypted {
                        *categories.entry("mev_risk".to_string()).or_insert(0) += 5;
                        details.push("privacy block without encryption — visible to relayers".into());
                    }
                }
                _ => {}
            }
        }

        // Chain risk score
        let chain_score: u32 = chains_used
            .iter()
            .map(|c| self.chain_risk.get(c.as_str()).copied().unwrap_or(50))
            .sum();
        categories.insert("chain_risk".into(), chain_score);
        if chains_used.is_empty() {
            details.push("chain risk: no chains specified".into());
        }

        // Bridge risk score
        let bridge_score: u32 = bridges_used
            .iter()
            .map(|b| self.bridge_risk.get(b.as_str()).copied().unwrap_or(50))
            .sum();
        categories.insert("bridge_risk".into(), bridge_score);
        if !bridges_used.is_empty() {
            details.push(format!(
                "bridge risk: {} (via {})",
                bridge_score,
                bridges_used.join(", ")
            ));
        }

        // Solver risk - higher when no profit check
        let solver_score: u32 = if has_profit_check { 10 } else { 40 };
        categories.insert("solver_risk".into(), solver_score);
        if !has_profit_check {
            details.push("solver risk: no profit threshold set".into());
        }

        // Liquidity risk
        let liquidity_score: u32 = if has_liquidity_check { 10 } else { 35 };
        categories.insert("liquidity_risk".into(), liquidity_score);
        if !has_liquidity_check {
            details.push("liquidity risk: no bridge_liquidity check".into());
        }

        // Timeout risk
        let timeout_score: u32 = if timeout_secs > 86400 {
            30
        } else if timeout_secs > 3600 {
            15
        } else if timeout_secs > 0 {
            5
        } else {
            25
        };
        categories.insert("timeout_risk".into(), timeout_score);
        if timeout_secs == 0 {
            details.push("timeout risk: no timeout set".into());
        } else if timeout_secs > 86400 {
            details.push(format!("timeout risk: long deadline ({}s)", timeout_secs));
        }

        // Slippage risk (basis points; 100 bps = 1%)
        let slippage_score: u32 = if slippage_bps > 1000 {
            40
        } else if slippage_bps > 500 {
            20
        } else if slippage_bps > 0 {
            5
        } else {
            10
        };
        categories.insert("slippage_risk".into(), slippage_score);
        if slippage_bps > 1000 {
            details.push(format!(
                "slippage risk: high slippage ({}bps / {:.2}%)",
                slippage_bps,
                slippage_bps as f64 / 100.0
            ));
        }

        // Refund path risk
        let refund_score: u32 = if has_refund { 0 } else { 30 };
        categories.insert("refund_risk".into(), refund_score);
        if !has_refund {
            details.push("refund risk: no refund path configured".into());
        }

        // Nonce risk
        let nonce_score: u32 = if has_nonce { 0 } else { 15 };
        categories.insert("nonce_risk".into(), nonce_score);
        if !has_nonce {
            details.push("nonce risk: no nonce guard for replay protection".into());
        }

        // Route score risk. Genuinely inapplicable to a pure trading-core-v1
        // program — there is no route-scoring concept in that AST — so it
        // is only scored when the program actually declares an intent.
        if has_intent_decl {
            let route_score_risk: u32 = if has_route_score { 0 } else { 10 };
            categories.insert("route_score_risk".into(), route_score_risk);
            if !has_route_score {
                details.push("route score risk: no route score threshold".into());
            }
        }

        let overall_score: u32 = categories.values().sum();
        let max_score: u32 = 500;

        RiskReport {
            overall_score: overall_score.min(max_score),
            max_score,
            categories,
            details,
        }
    }
}

impl Default for RiskScorer {
    fn default() -> Self {
        Self::new()
    }
}
