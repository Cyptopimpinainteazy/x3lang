//! Risk scoring for X3 intents.
//!
//! Computes a numeric risk score from intent constraints,
//! chain risk, bridge risk, solver risk, and liquidity risk.

use crate::semantic::CompilationMode;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use x3_lang_ast::ast::*;
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
        let mut slippage_pct: u64 = 0;
        let mut has_refund = false;
        let mut has_nonce = false;
        let mut has_route_score = false;

        for item in &program.items {
            if let Item::IntentDecl(intent) = &item.node {
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
                                    slippage_pct = *value as u64;
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

        // Slippage risk
        let slippage_score: u32 = if slippage_pct > 10 {
            40
        } else if slippage_pct > 5 {
            20
        } else if slippage_pct > 0 {
            5
        } else {
            10
        };
        categories.insert("slippage_risk".into(), slippage_score);
        if slippage_pct > 10 {
            details.push(format!("slippage risk: high slippage ({}%)", slippage_pct));
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

        // Route score risk
        let route_score_risk: u32 = if has_route_score { 0 } else { 10 };
        categories.insert("route_score_risk".into(), route_score_risk);
        if !has_route_score {
            details.push("route score risk: no route score threshold".into());
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
