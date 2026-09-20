//! Strategy modules — spec build-order item 22, PHASE 23.
//!
//! Eight required declarations would be decoration if nothing read them, so
//! these tests are about each declaration being *used*: an effect the body does
//! not perform, a guarantee nothing discharges, a chain the module never
//! declared, a body that exceeds its permissions, a risk profile the body's own
//! guards exceed.

use x3_lang_compiler::semantic::CompilationMode;

fn errors(source: &str) -> Vec<String> {
    match x3_lang_compiler::check_source_diagnostics_with_mode(source, CompilationMode::Dev) {
        Ok((_, _, outcome)) => outcome.errors.iter().map(|error| error.to_string()).collect(),
        Err(error) => vec![format!("{error}")],
    }
}

/// A module with every section, and an `execute` that swaps as declared.
fn module(execute: &str, sections: &str) -> String {
    format!(
        r#"strategy TriDexArb {{
    input ethereum.USDC amount 25_000_000
    output ethereum.ETH
{sections}    execute {{
{execute}
    }}
}}
"#
    )
}

/// Every section a module needs, with the optional clauses omitted rather than
/// declared empty — the parser refuses an empty list, on the grounds that
/// omission says the same thing more clearly.
fn defaults() -> String {
    "    effects [swap]\n    domains [ethereum]\n    risk { max_slippage_bps 50 max_total_fee_bps 8 }\n    bounds { max_steps 10 max_gas 200_000 }\n".to_string()
}

fn swap_steps() -> String {
    "        swap uniswap ethereum.USDC -> ethereum.ETH amount 1000 min_output 1\n        require slippage <= 50\n        on_fail refund ethereum.USDC to sender".to_string()
}

#[test]
fn a_well_formed_module_compiles() {
    let found = errors(&module(&swap_steps(), &defaults()));
    assert!(found.is_empty(), "a complete module must compile: {found:?}");
}

#[test]
fn an_input_without_an_amount_is_rejected() {
    let source =
        module(&swap_steps(), &defaults()).replace("input ethereum.USDC amount 25_000_000", "input ethereum.USDC");
    let found = errors(&source);
    assert!(
        found.iter().any(|error| error.contains("states no amount")),
        "capital nobody bounded is capital nobody agreed to: {found:?}"
    );
}

#[test]
fn a_declared_effect_the_body_does_not_perform_is_rejected() {
    let sections = defaults().replace("effects [swap]", "effects [swap, bridge]");
    let found = errors(&module(&swap_steps(), &sections));
    assert!(
        found
            .iter()
            .any(|error| error.contains("effect 'bridge'") && error.contains("nothing in `execute`")),
        "claiming work the module does not do must be refused: {found:?}"
    );
}

#[test]
fn an_effect_a_module_body_cannot_discharge_says_so() {
    // `borrow` has no statement form in a module body. That is a different
    // answer from "you forgot to write it", and the message says which.
    let sections = defaults().replace("effects [swap]", "effects [borrow]");
    let found = errors(&module(&swap_steps(), &sections));
    assert!(
        found
            .iter()
            .any(|error| error.contains("no statement that could discharge it")),
        "the language's limit must be reported as the language's limit: {found:?}"
    );
}

#[test]
fn a_min_profit_guarantee_needs_a_profit_floor_in_the_body() {
    let sections = format!("{}    guarantees [min_profit]\n", defaults());
    let found = errors(&module(&swap_steps(), &sections));
    assert!(
        found
            .iter()
            .any(|error| error.contains("guarantee 'min_profit'") && error.contains("require profit")),
        "a guarantee with nothing behind it must be refused, with the fix: {found:?}"
    );
}

#[test]
fn a_profit_floor_discharges_the_min_profit_guarantee() {
    // Non-vacuous, and direction-aware: a ceiling on profit would not be a floor.
    let sections = format!("{}    guarantees [min_profit]\n", defaults());
    let execute = format!("{}\n        require profit >= 5", swap_steps());
    let found = errors(&module(&execute, &sections));
    assert!(
        !found.iter().any(|error| error.contains("min_profit")),
        "a profit floor discharges the guarantee: {found:?}"
    );
}

#[test]
fn a_profit_ceiling_does_not_discharge_the_min_profit_guarantee() {
    // `require profit <= 5` says the opposite of a floor. Reading it as one is
    // the mistake the guard's comparison exists to prevent.
    let sections = format!("{}    guarantees [min_profit]\n", defaults());
    let execute = format!("{}\n        require profit <= 5", swap_steps());
    let found = errors(&module(&execute, &sections));
    assert!(
        found.iter().any(|error| error.contains("min_profit")),
        "a ceiling is not a floor: {found:?}"
    );
}

#[test]
fn a_body_that_reaches_an_undeclared_chain_is_rejected() {
    let execute = "        bridge x3 ethereum.USDC -> solana.SOL amount 1 receiver 0x1\n        require finality.ethereum >= 12\n        timeout 30s refund ethereum.USDC to sender\n        on_fail refund ethereum.USDC to sender".to_string();
    let sections = defaults().replace("effects [swap]", "effects [bridge]");
    let found = errors(&module(&execute, &sections));
    assert!(
        found
            .iter()
            .any(|error| error.contains("touches chain 'solana'") && error.contains("domains")),
        "a module that reaches a chain it never listed has not declared its requirements: {found:?}"
    );
}

#[test]
fn a_body_touching_two_chains_needs_the_cross_domain_permission() {
    let execute = "        bridge x3 ethereum.USDC -> solana.SOL amount 1 receiver 0x1\n        require finality.ethereum >= 12\n        timeout 30s refund ethereum.USDC to sender\n        on_fail refund ethereum.USDC to sender".to_string();
    let sections = defaults()
        .replace("effects [swap]", "effects [bridge]")
        .replace("domains [ethereum]", "domains [ethereum, solana]");
    let found = errors(&module(&execute, &sections));
    assert!(
        found
            .iter()
            .any(|error| error.contains("cross_domain") && error.contains("permission")),
        "doing more than the module declared must be refused: {found:?}"
    );
    // And it carries the catalogue's code for the class: a build system keys on the code, a
    // person reads the message, and a code that renders on one check and not another is the
    // drift the catalogue exists to stop (PHASE 52, TICKET-021).
    assert!(
        found.iter().any(|error| error.contains("X3E4025")),
        "the declaration that its body exceeds is the `declaration` class: {found:?}"
    );
}

#[test]
fn declaring_the_cross_domain_permission_permits_the_bridge() {
    let execute = "        bridge x3 ethereum.USDC -> solana.SOL amount 1 receiver 0x1\n        require finality.ethereum >= 12\n        require nonce unused module_bridge_1\n        timeout 30s refund ethereum.USDC to sender\n        on_fail refund ethereum.USDC to sender".to_string();
    let sections = format!(
        "{}    permissions [cross_domain]\n",
        defaults()
            .replace("effects [swap]", "effects [bridge]")
            .replace("domains [ethereum]", "domains [ethereum, solana]")
    );
    let found = errors(&module(&execute, &sections));
    assert!(
        !found.iter().any(|error| error.contains("cross_domain")),
        "a declared capability permits the body that uses it: {found:?}"
    );
}

#[test]
fn a_body_that_opts_into_fusion_needs_the_intent_fusion_permission() {
    let execute = format!("{}\n        allow intent_fusion", swap_steps());
    let found = errors(&module(&execute, &defaults()));
    assert!(
        found.iter().any(|error| error.contains("intent_fusion")),
        "the module must declare the capability its body uses: {found:?}"
    );
}

#[test]
fn a_body_relying_on_more_slippage_than_the_module_declares_is_rejected() {
    // The risk profile is part of the module, not a suggestion: a body whose own
    // guard accepts more slippage than the profile allows contradicts it.
    let execute = "        swap uniswap ethereum.USDC -> ethereum.ETH amount 1000 min_output 1\n        require slippage <= 900\n        on_fail refund ethereum.USDC to sender".to_string();
    let found = errors(&module(&execute, &defaults()));
    assert!(
        found
            .iter()
            .any(|error| error.contains("max_slippage_bps") && error.contains("risk profile")),
        "the declared risk profile has to bound what the body accepts: {found:?}"
    );
}

#[test]
fn a_zero_step_bound_is_rejected() {
    let sections = defaults().replace("max_steps 10", "max_steps 0");
    let found = errors(&module(&swap_steps(), &sections));
    assert!(
        found.iter().any(|error| error.contains("max_steps at zero")),
        "a module bounded at zero steps cannot do anything: {found:?}"
    );
}

#[test]
fn a_missing_risk_profile_is_rejected() {
    let sections = defaults().replace("    risk { max_slippage_bps 50 max_total_fee_bps 8 }\n", "");
    let found = errors(&module(&swap_steps(), &sections));
    assert!(
        found.iter().any(|error| error.contains("no risk profile")),
        "the bounds a module accepts are part of the module: {found:?}"
    );
}

/// A module's declared fee ceiling bounds the venues its body routes through.
///
/// `risk { max_total_fee_bps N }` is a cost bound the module states, and nothing compared it to
/// what the route costs: a module declaring 1bps and routing through a venue declaring 100bps
/// compiled with no error. The `arb` path has had this rule since it had a search (a venue over the
/// ceiling is struck from the standings); a strategy *names* its venue rather than searching, so
/// here it is a refusal.
mod the_declared_fee_ceiling_bounds_the_route {
    use super::{errors, module};

    /// A venue declaration with the fee given.
    fn venue(name: &str, fee_bps: u32) -> String {
        format!(
            "venue {name} {{\n    kind pool\n    chain ethereum\n    domain evm\n    asset_in \
             ethereum.USDC\n    asset_out ethereum.ETH\n    fee_bps {fee_bps}\n    liquidity \
             1_000_000\n    slippage_bps 8\n    latency_ms 12\n    finality_blocks 12\n    risk 2\n}}\n\n"
        )
    }

    /// The module from `defaults`, but routing through the named venue.
    fn routing_through(name: &str, ceiling: u32) -> String {
        let sections = format!(
            "    effects [swap]\n    domains [ethereum]\n    risk {{ max_slippage_bps 50 \
             max_total_fee_bps {ceiling} }}\n    bounds {{ max_steps 10 max_gas 200_000 }}\n"
        );
        let execute = format!("        swap {name} ethereum.USDC -> ethereum.ETH amount 1000 min_output 1\n        require slippage <= 50\n        on_fail refund ethereum.USDC to sender");
        module(&execute, &sections)
    }

    #[test]
    fn a_venue_over_the_ceiling_is_refused_with_both_figures() {
        let source = format!("{}{}", venue("pricey", 100), routing_through("pricey", 1));
        let found = errors(&source);
        assert!(
            found
                .iter()
                .any(|error| error.contains("pricey") && error.contains("100bps") && error.contains("is 1")),
            "the refusal must name the venue, its fee and the ceiling: {found:?}"
        );
    }

    #[test]
    fn a_venue_within_the_ceiling_is_accepted() {
        // Non-vacuous: the same module and venue with a ceiling the venue fits.
        let source = format!("{}{}", venue("pricey", 100), routing_through("pricey", 120));
        assert!(errors(&source).is_empty(), "100bps is within 120: {source}");
    }

    #[test]
    fn a_venue_the_program_never_declares_is_not_compared() {
        // Nothing states that venue's fee, so there is nothing to compare — a check that read the
        // missing declaration as 0bps would pass every ceiling, and one that refused it would
        // reject the corpus's own module, which routes through `uniswap` and declares no venue.
        let sections = "    effects [swap]\n    domains [ethereum]\n    risk { max_slippage_bps 50 max_total_fee_bps 1 }\n    bounds { max_steps 10 max_gas 200_000 }\n";
        let execute = "        swap uniswap ethereum.USDC -> ethereum.ETH amount 1000 min_output 1\n        require slippage <= 50\n        on_fail refund ethereum.USDC to sender";
        assert!(
            errors(&module(execute, sections)).is_empty(),
            "an undeclared venue has no fee to compare"
        );
    }
}
