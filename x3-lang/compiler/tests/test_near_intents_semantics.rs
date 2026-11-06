//! NEAR Intents (1Click) is a solver-network route, not a generic bridge.
//!
//! The compiler's three fail-closed production checks — adapter allow-list,
//! chain compatibility, and explicit source finality — treat every bridge the
//! same way by default. NEAR Intents breaks that symmetry on purpose: the
//! solver network owns source-chain finality and the 1Click API exposes no
//! confirmation-count control, so the program *cannot* declare `require
//! finality.*` and `near_intents.py` refuses the guard rather than pretend to
//! honour it. These tests pin that the NEAR route is admitted as a named,
//! auditable extension while the general production allow-lists stay
//! fail-closed.

use std::path::PathBuf;

use x3_lang_compiler::semantic::CompilationMode;

fn errors(source: &str) -> Vec<String> {
    match x3_lang_compiler::check_source_diagnostics_with_mode(source, CompilationMode::Dev) {
        Ok((_, _, outcome)) => outcome.errors.iter().map(|error| error.to_string()).collect(),
        Err(error) => vec![format!("{error}")],
    }
}

fn example(name: &str) -> String {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../examples").join(name);
    std::fs::read_to_string(&path).unwrap_or_else(|error| panic!("read {}: {error}", path.display()))
}

/// The five NEAR rollout examples — the mandate declaration and the four
/// intent-side programs — all compile under `x3c check` with no semantic
/// errors. They may carry warnings (the Dev-mode proof reminders), but the
/// allow-list / chain / finality checks that used to refuse them are now
/// resolved by the NEAR Intents extension.
#[test]
fn every_near_rollout_example_compiles() {
    for name in [
        "near_mandate_agent.x3",
        "near_intents_sol_to_near.x3",
        "near_intents_agent_payment.x3",
        "near_intents_agent_denied.x3",
        "bsc_bnb_to_near_wnear_dry.x3",
    ] {
        let found = errors(&example(name));
        assert!(found.is_empty(), "{name} must compile, but was refused: {found:?}");
    }
}

/// A solver-network adapter admits `near` as a chain and skips the source
/// finality requirement only because it is the named NEAR Intents adapter. A
/// bridge spelled through a different, unknown adapter is still refused.
#[test]
fn an_unknown_adapter_stays_refused() {
    let source = r#"
intent probe {
    from solana.USDC amount 25000000
    to near.USDC receiver x3-demo.near
    route {
        bridge Foobridge solana.USDC -> near.USDC min_receive 24800000;
    }
    require slippage <= 0.5%
    require fees <= 100 bps
    timeout 10m refund solana.USDC
    on_fail rollback
}
"#;
    let found = errors(source);
    assert!(
        found
            .iter()
            .any(|error| error.contains("production adapter allow-list")),
        "an unknown adapter must stay refused: {found:?}"
    );
}

/// The chain extension is scoped to the NEAR route: a bridge through a generic
/// adapter to an unlisted chain is still refused.
#[test]
fn an_unknown_chain_stays_refused() {
    let source = r#"
intent probe {
    from solana.USDC amount 25000000
    to martian.USDC receiver x3-demo.near
    route {
        bridge wormhole solana.USDC -> martian.USDC min_receive 24800000;
    }
    require slippage <= 0.5%
    require fees <= 100 bps
    timeout 10m refund solana.USDC
    on_fail rollback
}
"#;
    let found = errors(source);
    assert!(
        found.iter().any(|error| error.contains("unknown chain")),
        "an unknown chain must stay refused: {found:?}"
    );
}

/// The finality relaxation is scoped to the solver-network adapter. A generic
/// bridge from the same source chain still has to declare source finality.
#[test]
fn a_generic_bridge_still_requires_finality() {
    let source = r#"
intent probe {
    from solana.USDC amount 25000000
    to ethereum.USDC receiver 0x0000000000000000000000000000000000000000
    route {
        bridge wormhole solana.USDC -> ethereum.USDC min_receive 24800000;
    }
    require slippage <= 0.5%
    require fees <= 100 bps
    timeout 10m refund solana.USDC
    on_fail rollback
}
"#;
    let found = errors(source);
    assert!(
        found
            .iter()
            .any(|error| error.contains("no explicit finality requirement")),
        "a generic bridge without source finality must stay refused: {found:?}"
    );
}
