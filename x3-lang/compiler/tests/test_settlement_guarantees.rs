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
