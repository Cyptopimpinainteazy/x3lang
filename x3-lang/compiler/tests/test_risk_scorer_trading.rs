//! Regression coverage for the RiskScorer / trading-core-v1 blind spot
//! flagged (but not fixed) in PR #216: `score_program` only ever walked
//! `Item::IntentDecl`, so a trading-core-v1 program's real declarations
//! (assets, risk policy, atomic trade) never fed into the risk report at
//! all. A well-formed trading-core-v1 program used to score as if it had
//! no chains, no profit check, no refund path, no replay protection, and
//! no liquidity check — regardless of what it actually declared.

use x3_lang_compiler::parser::parse_source;
use x3_lang_compiler::risk::RiskScorer;
use x3_lang_compiler::CompilationMode;

const SOURCE: &str = include_str!("../../examples/trading_core_v1.x3");

const BRIDGE_SOURCE: &str = r#"
asset USDC = evm.ethereum.0xA0b8 { decimals: 6 }
asset WETH = evm.ethereum.0xC02a { decimals: 18 }
asset ETH = evm.ethereum.0x0000000000000000000000000000000000000000 { decimals: 18 }
asset USDC_BASE = evm.base.0xB1a0 { decimals: 6 }

risk policy MainnetArbBridge {
    max_slippage: 30 bps
    max_gas: 0.02 ETH
    max_flash_fee: 10 bps
    deadline: 2 blocks
    require_private_submission: false
}

atomic trade CrossDexArbToBase using MainnetArbBridge {
    borrow 1_000_000 USDC from aave_v3 as debt

    let weth = swap debt.amount USDC -> WETH
        via uniswap_v3
        min_out 410 WETH

    let returned = swap weth WETH -> USDC
        via sushiswap
        min_out 1_002_000 USDC

    repay debt

    bridge returned USDC -> USDC_BASE via wormhole to "0x1234567890abcdef1234567890abcdef12345678"

    require net_profit >= 1 USDC_BASE
    require all_debts_repaid
    emit receipt
}
"#;

#[test]
fn trading_core_v1_program_is_not_scored_as_if_it_were_empty() {
    let program = parse_source(SOURCE).expect("example must parse");
    let scorer = RiskScorer::with_mode(CompilationMode::Dev);
    let report = scorer.score_program(&program);

    // Before the fix, every one of these was the worst-case default because
    // Item::AtomicTrade / Item::TradeRiskPolicy were invisible to this
    // function. The example declares real assets on "ethereum", a real
    // profit floor, real min_out guards on both swaps, and `emit receipt`.
    assert_eq!(
        report.categories.get("chain_risk").copied(),
        Some(10),
        "ethereum should be recognized as the used chain, not scored as absent"
    );
    assert_eq!(
        report.categories.get("solver_risk").copied(),
        Some(10),
        "require net_profit should count as a real profit check"
    );
    assert_eq!(
        report.categories.get("liquidity_risk").copied(),
        Some(10),
        "a program with swap min_out guards should count as having a liquidity check"
    );
    assert_eq!(
        report.categories.get("refund_risk").copied(),
        Some(0),
        "atomic execution always rolls back on failure — strictly stronger than an optional refund path"
    );
    assert_eq!(
        report.categories.get("nonce_risk").copied(),
        Some(0),
        "emit receipt is this language's replay-protection mechanism"
    );

    // route_score_risk is an intent-DSL-only concept; a pure trading-core-v1
    // program must not be penalized for lacking a concept it cannot express.
    assert!(!report.categories.contains_key("route_score_risk"));

    // New, trading-core-v1-specific categories this fixture genuinely
    // triggers: its risk policy declares neither an oracle-deviation
    // ceiling nor a cumulative-loss ceiling.
    assert_eq!(
        report.categories.get("gas_risk").copied(),
        Some(0),
        "the resolved policy declares max_gas"
    );
    assert_eq!(
        report.categories.get("oracle_risk").copied(),
        Some(20),
        "MainnetArb does not set max_oracle_deviation_bps"
    );
    assert_eq!(
        report.categories.get("cumulative_loss_risk").copied(),
        Some(15),
        "MainnetArb does not set max_cumulative_loss"
    );
}

#[test]
fn trading_core_v1_bridge_trade_is_scored_for_the_chains_and_bridge_it_actually_uses() {
    let program = parse_source(BRIDGE_SOURCE).expect("bridge example must parse");
    let scorer = RiskScorer::with_mode(CompilationMode::Dev);
    let report = scorer.score_program(&program);

    // ethereum (10) + base (30) = 40, not the "no chains specified" default.
    assert_eq!(report.categories.get("chain_risk").copied(), Some(40));
    // wormhole bridge risk = 20, not silently absent.
    assert_eq!(report.categories.get("bridge_risk").copied(), Some(20));
}

#[test]
fn a_program_with_no_declarations_at_all_still_scores_without_panicking() {
    let program = parse_source("").expect("empty source must parse");
    let scorer = RiskScorer::with_mode(CompilationMode::Dev);
    let report = scorer.score_program(&program);
    assert_eq!(
        report.overall_score,
        report.categories.values().sum::<u32>().min(report.max_score)
    );
}
