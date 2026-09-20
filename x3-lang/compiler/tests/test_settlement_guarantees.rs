//! Settlement guarantees on a venue — spec PHASE 39.
//!
//! PHASE 39 is the phase about not lying: "Do not claim atomic CEX execution
//! unless the external venue exposes enforceable settlement semantics." So the two
//! properties tested here are the two ends of that sentence. A venue that matches
//! off-chain has to say where its guarantee actually comes from, and it may not
//! say `atomic` — the VM is not what settles a leg an off-chain venue fills.
//!
//! The other five shapes are not penalties. Each is a *true* description of a real
//! arrangement, and a program that picks one is accepted and round-trips.

use x3_lang_compiler::semantic::CompilationMode;

use x3_lang_compiler::ir::{Operation, SettlementGuarantee};

/// The operations a program lowers to, and the disassembly of the artifact it emits.
///
/// The second half is the half that matters for TICKET-075: everything asserted against
/// `trace` is read out of the **artifact**, through the artifact's own decode path, with
/// no access to the source that produced it.
fn artifact(source: &str) -> (Vec<Operation>, String) {
    let (_, ir, outcome) =
        x3_lang_compiler::check_source_diagnostics_with_mode(source, CompilationMode::Dev).expect("source must lower");
    assert!(outcome.errors.is_empty(), "unexpected errors: {:?}", outcome.errors);
    let bytecode = x3_lang_compiler::emitter::emit_x3ir(&ir).expect("the program must emit");
    let trace = x3_lang_compiler::emitter::disassemble(&bytecode).expect("its own artifact must disassemble");
    (ir.operations, trace)
}

fn carried_settlement(ops: &[Operation]) -> (String, Option<SettlementGuarantee>) {
    ops.iter()
        .find_map(|operation| match operation {
            Operation::VenueSettlement { venue, guarantee } => Some((venue.clone(), *guarantee)),
            _ => None,
        })
        .expect("a declared venue's settlement must reach the artifact")
}

fn errors(source: &str) -> Vec<String> {
    match x3_lang_compiler::check_source_diagnostics_with_mode(source, CompilationMode::Dev) {
        Ok((_, _, outcome)) => outcome.errors.iter().map(|error| error.to_string()).collect(),
        Err(error) => vec![format!("{error}")],
    }
}

/// A venue of `kind` with an optional `settlement` clause.
fn venue_of_kind(name: &str, kind: &str, settlement: Option<&str>) -> String {
    let clause = match settlement {
        Some(shape) => format!("    settlement {shape}\n"),
        None => String::new(),
    };
    format!(
        r#"venue {name} {{
    kind {kind}
    chain ethereum
    domain evm
    asset_in ethereum.USDC
    asset_out ethereum.ETH
    fee_bps 5
    liquidity 1_000_000
    slippage_bps 8
    latency_ms 12
    finality_blocks 12
    risk 2
{clause}}}
"#
    )
}

/// The five shapes that do not claim the venue enforces settlement itself.
const HONEST: [&str; 5] = ["trusted_adapter", "escrow", "pre_funded", "attested", "compensating"];

#[test]
fn an_orderbook_venue_must_say_how_its_leg_settles() {
    // Silence is the case the phase exists to prevent: a program that trades on an
    // off-chain venue and never says who carries the guarantee reads as though the
    // trade were atomic.
    let found = errors(&venue_of_kind("binance_spot", "orderbook", None));
    assert!(
        found
            .iter()
            .any(|error| error.contains("declares no `settlement`") && error.contains("trusted_adapter")),
        "the refusal must say what to write: {found:?}"
    );
}

#[test]
fn an_orderbook_venue_may_not_claim_atomic_settlement() {
    let found = errors(&venue_of_kind("binance_spot", "orderbook", Some("atomic")));
    assert!(
        found
            .iter()
            .any(|error| error.contains("claims `settlement atomic`") && error.contains("would be false")),
        "the refusal must say the claim is false and why: {found:?}"
    );
}

#[test]
fn every_honest_shape_is_accepted_on_an_off_chain_venue() {
    for shape in HONEST {
        let found = errors(&venue_of_kind("binance_spot", "orderbook", Some(shape)));
        assert!(
            found.is_empty(),
            "`settlement {shape}` is a true description of an off-chain leg: {found:?}"
        );
    }
}

#[test]
fn an_on_chain_venue_may_claim_atomic_settlement() {
    // A pool leg is executed by this VM, so the VM is what enforces both sides or
    // neither. Refusing `atomic` here would be refusing the truth.
    let found = errors(&venue_of_kind("uniswap_v3", "pool", Some("atomic")));
    assert!(found.is_empty(), "an on-chain leg really is atomic: {found:?}");
}

#[test]
fn an_on_chain_venue_that_states_no_guarantee_is_left_alone() {
    // The rule is about off-chain venues. An on-chain venue that says nothing has
    // said nothing false.
    let found = errors(&venue_of_kind("uniswap_v3", "pool", None));
    assert!(
        found.is_empty(),
        "an unstated guarantee on an on-chain venue is not a false claim: {found:?}"
    );
}

#[test]
fn an_unknown_shape_is_refused_with_the_list_of_honest_ones() {
    let error = x3_lang_compiler::parser::parse_source(&venue_of_kind("binance_spot", "orderbook", Some("vibes")))
        .expect_err("an invented guarantee must not parse");
    let message = format!("{error}");
    assert!(
        message.contains("unknown settlement guarantee 'vibes'"),
        "the refusal must name the word: {message}"
    );
    for shape in HONEST {
        assert!(message.contains(shape), "the refusal must list '{shape}': {message}");
    }
    assert!(
        message.contains("enforces both sides or neither"),
        "the refusal must say what `atomic` claims: {message}"
    );
}

#[test]
fn the_guarantee_round_trips_through_the_formatter() {
    // The trust model has to survive a reformat, or `x3c fmt` would quietly delete
    // the one line that says the trade is not atomic.
    let source = venue_of_kind("binance_spot", "orderbook", Some("compensating"));
    let program = x3_lang_compiler::parser::parse_source(&source).expect("a venue must parse");
    let formatted = x3_lang_compiler::formatter::X3Formatter::new().format_program(&program);
    assert!(
        formatted.contains("settlement compensating"),
        "the guarantee must survive the formatter: {formatted}"
    );
    let again = x3_lang_compiler::parser::parse_source(&formatted).expect("the output must parse");
    let round_tripped = x3_lang_compiler::formatter::X3Formatter::new().format_program(&again);
    assert_eq!(
        formatted, round_tripped,
        "formatting must be idempotent over a venue's guarantee"
    );
}

#[test]
fn a_serialized_venue_from_before_the_clause_still_loads() {
    // `#[serde(default)]`: an AST stored before `settlement` existed carries no
    // guarantee, and "nothing was written" is the honest answer for it rather than
    // a load failure.
    let json = r#"{
        "name": "binance_spot",
        "kind": "Orderbook",
        "chain": "ethereum",
        "domain": "evm",
        "asset_in": {"chain": "ethereum", "name": "USDC"},
        "asset_out": {"chain": "ethereum", "name": "ETH"},
        "fee_bps": 5,
        "liquidity": 1000000,
        "slippage_bps": 8,
        "latency_ms": 12,
        "finality_blocks": 12,
        "risk": 2,
        "proof": null
    }"#;
    let venue: x3_lang_ast::ast::VenueDecl = serde_json::from_str(json).expect("an older AST must still load");
    assert_eq!(
        venue.settlement, None,
        "an AST with no guarantee loads with none, and the `orderbook` rule then asks for one"
    );
}

// ===== the guarantee reaches the artifact (TICKET-075) =====

/// PHASE 39's clause was enforced at compile time and carried nowhere. The emitted
/// bytecode said nothing about whether a leg is atomic, escrowed, pre-funded, attested or
/// merely compensated, so a counterparty, an auditor or a replayer reading the *artifact*
/// could not see the assumption the trade rests on — which is half of what the phase asks
/// for, and the half a reader who does not have the source depends on.
#[test]
fn every_shape_reaches_the_artifact_and_the_artifact_decodes_it_back() {
    for shape in SettlementGuarantee::ALL {
        let source = venue_of_kind("hedge_venue", "pool", Some(shape.as_str()));
        let (ops, trace) = artifact(&source);

        assert_eq!(
            carried_settlement(&ops),
            ("hedge_venue".to_string(), Some(shape)),
            "{shape:?} must travel as a typed value, not as a string a reader re-parses"
        );
        let expected = format!("venue hedge_venue settles {}", shape.as_str());
        assert!(
            trace.contains(&expected),
            "the artifact's own decode must state `{expected}`:\n{trace}"
        );
    }
}

/// The `Option` is load-bearing: the compiler *requires* a guarantee of an `orderbook`
/// venue and leaves an on-chain venue that states none alone, so "states none" and
/// "states atomic" are two different facts about a trade. A record that defaulted the
/// absent case would invent the one claim PHASE 39 adds this clause to stop, which is why
/// this asserts the *absence* rather than only that something arrived.
#[test]
fn a_venue_that_states_no_shape_travels_as_none_rather_than_as_atomic() {
    let (ops, trace) = artifact(&venue_of_kind("pool_plain", "pool", None));

    assert_eq!(
        carried_settlement(&ops),
        ("pool_plain".to_string(), None),
        "a venue that states no settlement must not be given one"
    );
    assert!(
        trace.contains("venue pool_plain settles none"),
        "and the artifact must say so rather than stay silent:\n{trace}"
    );
}

/// This is the acceptance criterion in the phase's own words — "a host or replayer that
/// recovers it can tell an `atomic` leg from a `compensating` one" — and the two artifacts
/// are compared with nothing else in scope, so nothing but the artifact's own decode is
/// doing the telling.
#[test]
fn an_atomic_leg_and_a_compensating_leg_are_distinguishable_from_the_artifact_alone() {
    let (_, atomic_trace) = artifact(&venue_of_kind("leg", "pool", Some("atomic")));
    let (_, compensating_trace) = artifact(&venue_of_kind("leg", "orderbook", Some("compensating")));

    assert!(
        atomic_trace.contains("venue leg settles atomic"),
        "an on-chain leg's artifact says atomic:\n{atomic_trace}"
    );
    assert!(
        compensating_trace.contains("venue leg settles compensating"),
        "an off-chain leg's artifact says compensating:\n{compensating_trace}"
    );
    assert_ne!(
        atomic_trace, compensating_trace,
        "the two assumptions must not produce the same artifact"
    );
}

/// The record names its venue, so the guarantee can be attributed to the leg that rests on
/// it. A record a reader cannot attribute is a guarantee about nothing, and declaration
/// order is what lets a reader line the records up with the source it no longer has.
#[test]
fn each_record_names_the_venue_it_belongs_to_in_declaration_order() {
    let source = format!(
        "{}{}",
        venue_of_kind("first_venue", "pool", Some("atomic")),
        venue_of_kind("second_venue", "orderbook", Some("escrow"))
    );
    let (ops, trace) = artifact(&source);

    let carried: Vec<(String, Option<SettlementGuarantee>)> = ops
        .iter()
        .filter_map(|operation| match operation {
            Operation::VenueSettlement { venue, guarantee } => Some((venue.clone(), *guarantee)),
            _ => None,
        })
        .collect();
    assert_eq!(
        carried,
        vec![
            ("first_venue".to_string(), Some(SettlementGuarantee::Atomic)),
            ("second_venue".to_string(), Some(SettlementGuarantee::Escrow)),
        ],
        "each record belongs to its own venue, in declaration order"
    );
    assert!(trace.contains("venue first_venue settles atomic"), "{trace}");
    assert!(trace.contains("venue second_venue settles escrow"), "{trace}");
}
