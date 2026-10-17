//! The canonical-X3BC bridge: `x3-lang/compiler` lowers trading programs into the root X3BC format
//! through `x3bc-core` (docs/roadmap/X3LANG_PRODUCTION_UNIFICATION.md). Constructs that cannot be
//! represented are refused with an explicit `X3BC-L…` diagnostic — never dropped, never stubbed.

use x3_lang_compiler::semantic::CompilationMode;
use x3_lang_compiler::x3bc::{compile_trading_x3bc, X3bcArtifact};
use x3bc_core::format::BytecodeModule;
use x3bc_core::trading::{CapabilityKind, CompilationModeId, TradeOp};

const SOURCE: &str = include_str!("../../examples/x3chain_flash_arb.x3");

fn lower(source: &str, mode: CompilationMode) -> Result<X3bcArtifact, String> {
    compile_trading_x3bc(source, mode).map_err(|error| format!("{error:?}"))
}

fn refusal(source: &str) -> String {
    match lower(source, CompilationMode::Testnet) {
        Ok(_) => panic!("must be refused"),
        Err(error) => error,
    }
}

#[test]
fn the_flagship_program_lowers_to_canonical_x3bc() {
    let artifact = lower(SOURCE, CompilationMode::Testnet).expect("lowers");
    let module = BytecodeModule::from_bytes(&artifact.bytes).expect("canonical decoder");
    let section = module.trading.expect("trading section");
    assert_eq!(section.chain_id, 650_000);
    assert_eq!(section.mode, CompilationModeId::Testnet);
    assert_eq!(section.source_hash, x3bc_core::hash::sha256(SOURCE.as_bytes()));
    assert_eq!(section.compiler_version, x3_lang_compiler::COMPILER_VERSION);
    assert_eq!(section.assets.iter().map(|asset| asset.id).collect::<Vec<_>>(), [1, 2]);
    let capabilities: Vec<(CapabilityKind, &str, u16)> = section
        .capabilities
        .iter()
        .map(|cap| (cap.kind, cap.name.as_str(), cap.version))
        .collect();
    assert_eq!(
        capabilities,
        [
            (CapabilityKind::FlashProvider, "x3_flashloan", 1),
            (CapabilityKind::Venue, "x3dex_pool_1", 1),
            (CapabilityKind::Venue, "x3dex_pool_2", 1),
            (CapabilityKind::Oracle, "x3_oracle", 1),
        ]
    );
    assert!(section.policy.private_submission_required);
    assert!(x3bc_core::bytecode::requires_private_submission(&artifact.bytes));
    assert!(matches!(section.ops.last(), Some(TradeOp::AssertAllDebtsClosed)));
    x3bc_core::trading::verify_section(&section).expect("statically sound");
}

#[test]
fn compilation_mode_is_carried_into_the_artifact() {
    for (mode, id) in [
        (CompilationMode::Dev, CompilationModeId::Dev),
        (CompilationMode::Testnet, CompilationModeId::Testnet),
    ] {
        let artifact = lower(SOURCE, mode).expect("lowers");
        assert_eq!(artifact.mode, id);
        let module = BytecodeModule::from_bytes(&artifact.bytes).expect("decodes");
        assert_eq!(module.trading.expect("section").mode, id);
    }
}

#[test]
fn mainnet_mode_keeps_its_existing_private_submission_refusal() {
    assert!(lower(SOURCE, CompilationMode::Mainnet).is_err());
}

#[test]
fn lowering_is_deterministic() {
    let a = lower(SOURCE, CompilationMode::Testnet).expect("lowers");
    let b = lower(SOURCE, CompilationMode::Testnet).expect("lowers");
    assert_eq!(a.bytes, b.bytes);
}

#[test]
fn a_bridge_cannot_lower_yet_and_says_so() {
    let source = SOURCE
        .replace(
            "risk policy",
            "asset USDX_BASE = evm.base.0x833589 {\n    decimals: 6\n}\n\nrisk policy",
        )
        .replace(
            "    require net_profit >= 1 USDX\n",
            "    require net_profit >= 1 USDX\n    bridge usdx_back USDX -> USDX_BASE via wormhole to \"0x1234567890abcdef1234567890abcdef12345678\"\n",
        );
    let error = refusal(&source);
    assert!(error.contains("X3BC-L") && error.contains("bridge"), "{error}");
}

#[test]
fn mev_leakage_has_no_on_chain_source_and_is_refused() {
    let source = SOURCE.replace(
        "    quote_freshness: 3\n",
        "    quote_freshness: 3\n    max_mev_leakage: 10 bps\n",
    );
    let error = refusal(&source);
    assert!(error.contains("X3BC-L") && error.contains("max_mev_leakage"), "{error}");
}

#[test]
fn cross_trade_loss_breaker_is_refused() {
    let source = SOURCE.replace(
        "    quote_freshness: 3\n",
        "    quote_freshness: 3\n    max_cumulative_loss: 10 USDX\n",
    );
    let error = refusal(&source);
    assert!(
        error.contains("X3BC-L") && error.contains("max_cumulative_loss"),
        "{error}"
    );
}

#[test]
fn an_off_chain_asset_is_refused() {
    let error = refusal(&SOURCE.replace("x3.x3chain.asset_2", "evm.ethereum.0xC02a"));
    assert!(error.contains("X3BC-L"), "{error}");
}

#[test]
fn an_unknown_venue_is_refused() {
    let error = refusal(&SOURCE.replace("via x3dex_pool_2", "via uniswap_v3"));
    assert!(error.contains("X3BC-L") && error.contains("uniswap_v3"), "{error}");
}

#[test]
fn an_unknown_flash_provider_is_refused() {
    let error = refusal(&SOURCE.replace("from x3_flashloan", "from aave_v3"));
    assert!(error.contains("X3BC-L") && error.contains("aave_v3"), "{error}");
}

#[test]
fn a_non_trading_program_is_not_a_trading_artifact() {
    let error = refusal(include_str!("../../examples/simple_swap.x3"));
    assert!(error.contains("X3BC-L"), "{error}");
}
