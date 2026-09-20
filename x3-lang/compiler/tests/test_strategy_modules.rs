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

/// The declared fee ceiling reaches the artifact.
///
/// PHASE 7: "Risk policy must compile into the artifact". The *slippage* ceiling arrives through the
/// guard the body writes; the fee ceiling had no statement that wrote it, so it arrived nowhere.
/// It is a declaration record now, the shape the finality policy's depth already uses, and the
/// emitter carries its figure the way it carries any static guard's (TICKET-114).
mod the_declared_fee_ceiling_travels {
    use super::module;
    use x3_lang_compiler::compile_source;

    fn artifact(ceiling: u32) -> Vec<u8> {
        let sections = format!(
            "    effects [swap]\n    domains [ethereum]\n    risk {{ max_slippage_bps 50 \
             max_total_fee_bps {ceiling} }}\n    bounds {{ max_steps 10 max_gas 200_000 }}\n"
        );
        let execute = "        swap uniswap ethereum.USDC -> ethereum.ETH amount 1000 min_output 1\n        require slippage <= 50\n        on_fail refund ethereum.USDC to sender";
        compile_source(&module(execute, &sections)).unwrap_or_else(|error| panic!("ceiling {ceiling}: {error:?}"))
    }

    #[test]
    fn the_artifact_states_the_ceiling_and_what_it_counts() {
        let trace = x3_lang_compiler::emitter::disassemble(&artifact(8)).expect("it must disassemble");
        assert!(
            trace.contains("REQUIRE static fees 8"),
            "a reader of the artifact must be able to see the ceiling the module declared: {trace}"
        );
    }

    #[test]
    fn two_different_ceilings_are_two_different_artifacts() {
        assert_ne!(
            artifact(8),
            artifact(3),
            "two modules accepting different fee ceilings must not compile to the same bytes"
        );
    }
}

/// A body can state its own fee ceiling, and the declared profile has to bound it.
///
/// The pair the slippage ceiling has: `risk { max_total_fee_bps M }` is what a module accepts, and
/// `require fees <= N` is what a body relies on. Before TICKET-116 only the policy half existed —
/// a module could declare a ceiling and no statement could say what its body would pay.
mod a_body_can_state_its_own_fee_ceiling {
    use super::{errors, module};
    use x3_lang_compiler::compile_source;

    fn with_guard(guard: &str) -> String {
        let sections = "    effects [swap]\n    domains [ethereum]\n    risk { max_slippage_bps 50 max_total_fee_bps 8 }\n    bounds { max_steps 10 max_gas 200_000 }\n";
        let execute = format!(
            "        swap uniswap ethereum.USDC -> ethereum.ETH amount 1000 min_output 1\n        \
             require slippage <= 50\n        {guard}\n        on_fail refund ethereum.USDC to sender"
        );
        module(&execute, sections)
    }

    #[test]
    fn a_guard_within_the_declared_ceiling_is_accepted() {
        let found = errors(&with_guard("require fees <= 8"));
        assert!(found.is_empty(), "8bps is what the profile accepts: {found:?}");
    }

    #[test]
    fn a_guard_above_the_declared_ceiling_is_refused_with_both_figures() {
        let found = errors(&with_guard("require fees <= 30"));
        assert!(
            found.iter().any(|error| error.contains("30") && error.contains("8bps")),
            "the refusal must name the guard's bound and the profile's: {found:?}"
        );
    }

    #[test]
    fn a_guard_with_no_declaration_behind_it_is_refused() {
        // An intent has no `risk { }` clause, so a fee guard in one claims something nothing backs —
        // the same rule every other declared kind follows.
        let source = "intent unbacked {\n    from ethereum.USDC amount 1\n    to solana.SOL\n    \
                      route {\n        swap uniswap ethereum.USDC -> solana.SOL amount 1 min_output 1\n    }\n    \
                      require fees <= 8\n    require slippage <= 50\n    on_fail refund ethereum.USDC to sender\n}\n";
        let found = errors(source);
        assert!(
            found.iter().any(|error| error.contains("nothing declares one")),
            "a fee guard with no ceiling behind it must be refused by name: {found:?}"
        );
    }

    #[test]
    fn the_guard_reaches_the_artifact() {
        let bytecode = compile_source(&with_guard("require fees <= 5")).expect("it must compile");
        let trace = x3_lang_compiler::emitter::disassemble(&bytecode).expect("it must disassemble");
        let fees_records = trace.lines().filter(|line| line.contains("fees")).count();
        assert!(
            fees_records >= 2,
            "the declaration and the body's guard are two records, both stating a ceiling: {trace}"
        );
        assert!(
            trace.contains("static fees 5"),
            "the guard's own figure must be readable: {trace}"
        );
    }
}

/// `bounded_slippage` is a guarantee the body can discharge (PHASE 5's own example).
///
/// The spec's example writes `guarantees [debt_closed, min_profit, bounded_slippage]`, and the
/// language refused the third name — while `verify_slippage_explicit` already required exactly that
/// ceiling of any body with a swap leg. A module may state the guarantee it has (TICKET-022).
mod bounded_slippage_is_a_guarantee_a_body_can_discharge {
    use super::module;

    fn with(guarantees: &str, body_guards: &str) -> String {
        let sections = format!(
            "    effects [swap]\n    guarantees [{guarantees}]\n    domains [ethereum]\n    risk \
             {{ max_slippage_bps 50 max_total_fee_bps 8 }}\n    bounds {{ max_steps 10 max_gas 200_000 }}\n"
        );
        let execute = format!(
            "        swap uniswap ethereum.USDC -> ethereum.ETH amount 1000 min_output 1\n{body_guards}\n        \
             on_fail refund ethereum.USDC to sender"
        );
        module(&execute, &sections)
    }

    #[test]
    fn a_body_with_a_ceiling_discharges_it() {
        let found = super::errors(&with(
            "min_profit, bounded_slippage",
            "        require slippage <= 50\n        require profit >= 5",
        ));
        assert!(found.is_empty(), "the body writes the ceiling it claims: {found:?}");
    }

    #[test]
    fn a_body_without_one_is_refused_with_the_clause_to_add() {
        let found = super::errors(&with("bounded_slippage", "        require profit >= 5"));
        assert!(
            found
                .iter()
                .any(|error| error.contains("bounded_slippage") && error.contains("require slippage <= ")),
            "the refusal must name the guarantee and the clause that discharges it: {found:?}"
        );
    }

    #[test]
    fn a_guarantee_the_language_does_not_model_is_still_refused_by_name() {
        // `principal_preserved` is the spec's other name and is *not* admitted: its only derivable
        // discharge would be a profit floor of zero, which is what `min_profit` already means, and a
        // second name for one claim is the "label that means nothing" defect TICKET-022 exists to
        // avoid. This test fails the day someone admits it — which is the point: the decision has to
        // be recorded rather than assumed.
        let found = super::errors(&with("principal_preserved", "        require slippage <= 50"));
        assert!(
            found
                .iter()
                .any(|error| error.contains("unknown guarantee 'principal_preserved'")
                    && error.contains("bounded_slippage")),
            "an unmodelled name is refused with the vocabulary it could have used: {found:?}"
        );
    }
}

/// A module's declared `max_steps` is a cap on what its body lowers to (PHASE 41).
///
/// Measured before this: a module declaring `bounds { max_steps 1 }` and a body that lowers to 74
/// operations *compiled and ran to completion* — nothing compared the number the module declared
/// with the number it produced. PHASE 41's own words are "prevent pathological execution graphs",
/// which is a property of the graph the compiler builds, so the cap is checked where both figures
/// are known.
mod the_declared_step_cap_bounds_the_body {
    use super::module;

    /// The same module with the gas cap given, for the weight half of PHASE 41.
    fn with_max_gas(gas: u32) -> String {
        let sections = format!(
            "    effects [swap]\n    domains [ethereum]\n    risk {{ max_slippage_bps 50 \
             max_total_fee_bps 8 }}\n    bounds {{ max_steps 1_000 max_gas {gas} }}\n"
        );
        let execute = "        swap uniswap ethereum.USDC -> ethereum.ETH amount 1000 min_output 1\n        \
                       require slippage <= 50\n        on_fail refund ethereum.USDC to sender";
        module(execute, &sections)
    }

    fn with_max_steps(steps: u32) -> String {
        let sections = format!(
            "    effects [swap]\n    domains [ethereum]\n    risk {{ max_slippage_bps 50 \
             max_total_fee_bps 8 }}\n    bounds {{ max_steps {steps} max_gas 200_000 }}\n"
        );
        let execute = "        swap uniswap ethereum.USDC -> ethereum.ETH amount 1000 min_output 1\n        \
                       require slippage <= 50\n        on_fail refund ethereum.USDC to sender";
        module(execute, &sections)
    }

    #[test]
    fn a_body_that_exceeds_the_cap_is_refused_with_both_figures() {
        let found = super::errors(&with_max_steps(1));
        assert!(
            found
                .iter()
                .any(|error| error.contains("max_steps 1") && error.contains("lowers to")),
            "the refusal must give the declared cap and what the body lowers to: {found:?}"
        );
    }

    #[test]
    fn a_body_within_the_cap_is_accepted() {
        // Non-vacuous: the same module with a cap its body fits.
        let found = super::errors(&with_max_steps(1_000));
        assert!(found.is_empty(), "a thousand steps is room for this body: {found:?}");
    }

    #[test]
    fn the_cap_counts_the_modules_own_operations_and_the_boundary_is_exact() {
        // The count is read out of the refusal rather than written down here: a cap of 1 always
        // fails and the message states what the body lowers to, so the test derives the figure and
        // then asserts both sides of it — the exact count passes, one less is refused. A number
        // typed into the test would be a second opinion about the lowering, which is the thing this
        // is checking.
        let refusal = super::errors(&with_max_steps(1));
        let count: u32 = refusal
            .iter()
            .find_map(|error| {
                let (_, rest) = error.split_once("lowers to ")?;
                rest.split_whitespace().next()?.parse().ok()
            })
            .unwrap_or_else(|| panic!("the refusal must state the count: {refusal:?}"));
        assert!(count > 1, "the body must lower to more than one operation: {count}");
        assert!(
            super::errors(&with_max_steps(count)).is_empty(),
            "a cap equal to the body's own count is a cap the body fits: {count}"
        );
        assert!(
            super::errors(&with_max_steps(count - 1))
                .iter()
                .any(|error| error.contains("max_steps")),
            "and one below it is refused: {count}"
        );
    }

    #[test]
    fn the_gas_cap_bounds_what_the_vm_charges_for_the_body() {
        // Measured before this: a module declaring `max_gas 1` ran to completion with 999115 gas
        // left. The figure comes from the refusal again, so the two sides of the boundary are the
        // compiler's own number rather than one typed here.
        let refusal = super::errors(&with_max_gas(1));
        let cost: u32 = refusal
            .iter()
            .find_map(|error| {
                let (_, rest) = error.split_once("operations cost ")?;
                rest.split_whitespace().next()?.parse().ok()
            })
            .unwrap_or_else(|| panic!("the refusal must state the cost: {refusal:?}"));
        assert!(cost > 1, "the body must cost more than one unit: {cost}");
        assert!(
            super::errors(&with_max_gas(cost)).is_empty(),
            "a cap equal to the body's own cost is a cap the body fits: {cost}"
        );
        assert!(
            super::errors(&with_max_gas(cost - 1))
                .iter()
                .any(|error| error.contains("max_gas")),
            "and one below it is refused: {cost}"
        );
    }
}

/// PHASE 41's own `resources { … }` block, and which of its five caps the compiler can back.
///
/// The phase writes `resources { max_compute = …; max_memory = …; max_network_calls = …; max_routes
/// = …; max_branches = …; }`; the implementation had two caps under a different block name. Four of
/// the five have a figure this compiler holds, and the fifth is refused by name rather than accepted
/// as a number nothing measures.
mod the_phases_own_resource_caps {
    use super::module;

    /// A module with the phase's block, over a body whose figures the caller chooses.
    fn with_resources(resources: &str, body: &str) -> String {
        let sections = format!(
            "    effects [swap]\n    domains [ethereum]\n    risk {{ max_slippage_bps 50 \
             max_total_fee_bps 8 }}\n    bounds {{ max_steps 1_000 max_gas 200_000 }}\n    \
             resources {{ {resources} }}\n"
        );
        module(body, &sections)
    }

    const SWAP_BODY: &str = "        swap uniswap ethereum.USDC -> ethereum.ETH amount 1000 min_output 1\n        \
                             require slippage <= 50\n        on_fail refund ethereum.USDC to sender";

    #[test]
    fn max_compute_bounds_the_body_in_the_same_unit_as_max_steps() {
        let found = super::errors(&with_resources("max_compute = 1;", SWAP_BODY));
        assert!(
            found
                .iter()
                .any(|error| error.contains("max_compute 1") && error.contains("lowers to")),
            "the phase's word for the count bounds the same figure: {found:?}"
        );
        assert!(
            super::errors(&with_resources("max_compute = 100;", SWAP_BODY)).is_empty(),
            "a cap the body fits is accepted"
        );
    }

    #[test]
    fn max_network_calls_counts_what_leaves_the_vm() {
        // A swap is executed by the VM; `mempool_scan` is a call the host answers, so it is the one
        // the cap counts.
        let with_a_call = "        mempool_scan(max_results=10);\n        swap uniswap ethereum.USDC -> \
                           ethereum.ETH amount 1000 min_output 1\n        require slippage <= 50\n        \
                           on_fail refund ethereum.USDC to sender";
        let refused = super::errors(&with_resources("max_network_calls = 0;", with_a_call));
        assert!(
            refused.iter().any(|error| error.contains("max_network_calls 0")),
            "a body that calls the host meets a cap of zero calls: {refused:?}"
        );
        assert!(
            super::errors(&with_resources("max_network_calls = 1;", with_a_call)).is_empty(),
            "and one call is room for it"
        );
        assert!(
            super::errors(&with_resources("max_network_calls = 0;", SWAP_BODY)).is_empty(),
            "a body with no host call meets a cap of zero — the cap counts calls, not instructions"
        );
    }

    #[test]
    fn max_routes_bounds_the_hops() {
        let found = super::errors(&with_resources("max_routes = 0;", SWAP_BODY));
        assert!(
            found
                .iter()
                .any(|error| error.contains("max_routes 0") && error.contains("hop")),
            "a route's hop is a swap or a bridge, and this body takes one: {found:?}"
        );
        assert!(
            super::errors(&with_resources("max_routes = 1;", SWAP_BODY)).is_empty(),
            "one hop is what it takes"
        );
    }

    #[test]
    fn max_branches_bounds_the_decisions() {
        let branching = "        if 1 > 0 { mempool_scan(max_results=1); }\n        swap uniswap \
                         ethereum.USDC -> ethereum.ETH amount 1000 min_output 1\n        require slippage \
                         <= 50\n        on_fail refund ethereum.USDC to sender";
        let found = super::errors(&with_resources("max_branches = 0;", branching));
        assert!(
            found
                .iter()
                .any(|error| error.contains("max_branches 0") && error.contains("decision")),
            "a decided `if` is still a decision the module contains: {found:?}"
        );
        assert!(
            super::errors(&with_resources("max_branches = 1;", branching)).is_empty(),
            "and one decision fits a cap of one"
        );
    }

    #[test]
    fn max_memory_is_refused_because_nothing_measures_it() {
        let found = super::errors(&with_resources("max_memory = 1024;", SWAP_BODY));
        assert!(
            found
                .iter()
                .any(|error| error.contains("max_memory") && error.contains("no memory model")),
            "the one cap this VM cannot back is refused by name: {found:?}"
        );
    }

    #[test]
    fn an_unknown_resources_field_is_refused_by_name() {
        let found = super::errors(&with_resources("max_gas_again = 1;", SWAP_BODY));
        assert!(
            found.iter().any(|error| error.contains("unknown resources field")),
            "the vocabulary is closed, so a misspelling is not a cap nothing checks: {found:?}"
        );
    }
}

/// A declaration the formatter does not know about is a declaration `x3c fmt` deletes.
///
/// The `resources` block was dropped by the formatter for exactly that reason — the writer had no
/// arm for it — which is the defect the annotations had one construct over. The check here is the
/// one that generalises: a block whose cap *binds* survives formatting, so a program the compiler
/// refuses cannot be turned into one it accepts by running `x3c fmt` over it.
mod formatting_keeps_the_resource_caps {
    use super::module;

    fn source(resources: &str) -> String {
        let sections = format!(
            "    effects [swap]\n    domains [ethereum]\n    risk {{ max_slippage_bps 50 \
             max_total_fee_bps 8 }}\n    bounds {{ max_steps 1_000 max_gas 200_000 }}\n    \
             resources {{ {resources} }}\n"
        );
        let execute = "        swap uniswap ethereum.USDC -> ethereum.ETH amount 1000 min_output 1\n        \
                       require slippage <= 50\n        on_fail refund ethereum.USDC to sender";
        module(execute, &sections)
    }

    #[test]
    fn a_binding_cap_still_refuses_after_formatting() {
        let program = x3_lang_compiler::parser::parse_source(&source("max_compute = 1;"))
            .expect("the program parses (the cap is a semantic bound, not a parse error)");
        let formatted = x3_lang_compiler::formatter::X3Formatter::new().format_program(&program);
        assert!(
            formatted.contains("max_compute = 1"),
            "the cap must be written back, not dropped: {formatted}"
        );
        let found = super::errors(&formatted);
        assert!(
            found.iter().any(|error| error.contains("max_compute 1")),
            "and the formatted program must still be refused for the same reason: {found:?}"
        );
    }
}

/// Two of the four permissions are tested and two are refused for naming a capability nothing has.
///
/// Measured: `StrategyPermission::PrivateSubmission` and `::FlashCapital` have **no reader** anywhere
/// in the compiler, the VM or the tooling. The other two are required by the body's own shape — a body
/// touching two chains must declare `cross_domain`, and one that opts into fusion must declare
/// `intent_fusion` — so they are the ceiling the phase describes rather than decoration.
mod permissions_that_nothing_tests_are_refused {
    use super::module;

    fn with_permission(permission: &str) -> String {
        let sections = format!(
            "    effects [swap]\n    domains [ethereum]\n    permissions [{permission}]\n    risk \
             {{ max_slippage_bps 50 max_total_fee_bps 8 }}\n    bounds {{ max_steps 10 max_gas \
             200_000 }}\n"
        );
        let execute = "        swap uniswap ethereum.USDC -> ethereum.ETH amount 1000 min_output 1\n        \
                       require slippage <= 50\n        on_fail refund ethereum.USDC to sender";
        module(execute, &sections)
    }

    #[test]
    fn private_submission_points_at_the_clause_that_works() {
        let found = super::errors(&with_permission("private_submission"));
        assert!(
            found
                .iter()
                .any(|error| error.contains("private_submission") && error.contains("submission { private")),
            "the refusal must name the spelling that reaches the artifact: {found:?}"
        );
    }

    #[test]
    fn flash_capital_is_refused_with_the_ticket_that_records_it() {
        let found = super::errors(&with_permission("flash_capital"));
        assert!(
            found
                .iter()
                .any(|error| error.contains("flash_capital") && error.contains("TICKET-040")),
            "the refusal must say why nothing can test it, and where that is recorded: {found:?}"
        );
    }

    #[test]
    fn the_two_permissions_the_body_can_exceed_are_accepted() {
        for permission in ["cross_domain", "intent_fusion"] {
            let found = super::errors(&with_permission(permission));
            assert!(
                !found.iter().any(|error| error.contains("permission")),
                "`{permission}` is a ceiling the body can exceed, not decoration: {found:?}"
            );
        }
    }
}
