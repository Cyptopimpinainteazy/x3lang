//! `venue` declarations — the nodes the opportunity graph is built from.
//!
//! A venue's declared attributes are what the planner reads, so a declaration
//! that is internally inconsistent is worse than a missing one: it is a graph
//! edge that looks traversable and is not.

use x3_lang_compiler::semantic::CompilationMode;

fn errors(source: &str) -> Vec<String> {
    match x3_lang_compiler::check_source_diagnostics_with_mode(source, CompilationMode::Dev) {
        Ok((_, _, outcome)) => outcome.errors.iter().map(|error| error.to_string()).collect(),
        Err(error) => vec![format!("{error}")],
    }
}

/// A venue with every field present. Fields are parameters rather than
/// appended lines, because appending a second `fee_bps` is now its own error
/// (a repeated field would silently override the first) and a fixture that
/// relied on that would be testing the wrong thing.
fn venue_with(name: &str, kind: &str, fee_bps: u32, liquidity: u128, slippage_bps: u32, risk: u32) -> String {
    format!(
        r#"venue {name} {{
    kind {kind}
    chain ethereum
    domain evm
    asset_in ethereum.USDC
    asset_out ethereum.ETH
    fee_bps {fee_bps}
    liquidity {liquidity}
    slippage_bps {slippage_bps}
    latency_ms 12
    finality_blocks 12
    risk {risk}
}}
"#
    )
}

fn venue(name: &str) -> String {
    venue_with(name, "pool", 5, 1_000_000, 8, 2)
}

#[test]
fn a_well_formed_venue_is_accepted() {
    let found = errors(&venue("uniswap_v3"));
    assert!(found.is_empty(), "a well-formed venue must compile: {found:?}");
}

#[test]
fn a_repeated_field_is_rejected_rather_than_silently_overriding() {
    let source = venue("twice").replace("    fee_bps 5\n", "    fee_bps 5\n    fee_bps 7\n");
    let found = errors(&source);
    assert!(
        found.iter().any(|error| error.contains("declares `fee_bps` twice")),
        "a program that writes one line twice must be told, not silently given the last: {found:?}"
    );
}

#[test]
fn the_bridge_kind_parses_even_though_bridge_is_a_keyword() {
    // `bridge` is a lexer keyword, so `kind bridge` arrives as a keyword token
    // rather than an identifier. The kind set decides what a venue kind is; the
    // token class is an accident of where else the word is used.
    let source = r#"venue to_solana {
    kind bridge
    chain ethereum
    domain evm
    asset_in ethereum.ETH
    asset_out solana.SOL
    fee_bps 2
    liquidity 500_000
    slippage_bps 5
    latency_ms 900
    finality_blocks 32
    risk 4
}
"#;
    let found = errors(source);
    assert!(found.is_empty(), "`kind bridge` must parse as a venue kind: {found:?}");
}

#[test]
fn an_unknown_kind_names_the_closed_set() {
    let source = venue_with("v", "magic_pool", 5, 1_000_000, 8, 2);
    let found = errors(&source);
    assert!(
        found
            .iter()
            .any(|error| error.contains("unknown venue kind") && error.contains("orderbook")),
        "the kind set is closed and the error must list it: {found:?}"
    );
}

#[test]
fn a_duplicate_venue_name_is_rejected() {
    let source = format!("{}{}", venue("twin"), venue("twin"));
    let found = errors(&source);
    assert!(
        found.iter().any(|error| error.contains("declared twice")),
        "the graph addresses venues by name, so a duplicate makes an edge ambiguous: {found:?}"
    );
}

#[test]
fn a_fee_that_is_the_whole_amount_is_rejected() {
    let source = venue_with("greedy", "pool", 10_000, 1_000_000, 8, 2);
    let found = errors(&source);
    assert!(
        found.iter().any(|error| error.contains("fee at or above 10_000")),
        "a 100% fee is not a venue: {found:?}"
    );
}

#[test]
fn a_risk_above_the_scale_is_rejected() {
    let source = venue_with("risky", "pool", 5, 1_000_000, 8, 101);
    let found = errors(&source);
    assert!(
        found.iter().any(|error| error.contains("risk 101")),
        "the risk scale is 0 to 100: {found:?}"
    );
}

#[test]
fn a_venue_with_no_liquidity_is_rejected() {
    let source = venue_with("empty", "pool", 5, 0, 8, 2);
    let found = errors(&source);
    assert!(
        found.iter().any(|error| error.contains("zero liquidity")),
        "a route no size can use is not a route: {found:?}"
    );
}

#[test]
fn a_pool_that_moves_one_asset_to_itself_is_rejected() {
    let source = venue("pointless").replace("asset_out ethereum.ETH", "asset_out ethereum.USDC");
    let found = errors(&source);
    assert!(
        found.iter().any(|error| error.contains("same asset")),
        "an edge from an asset to itself is a path nothing can use: {found:?}"
    );
}

#[test]
fn a_lending_venue_may_move_one_asset() {
    // Non-vacuous: the same shape is legitimate for a lending or flash venue,
    // which moves one asset and charges for it.
    let source = venue_with("lender", "lending", 5, 1_000_000, 8, 2)
        .replace("asset_out ethereum.ETH", "asset_out ethereum.USDC");
    let found = errors(&source);
    assert!(found.is_empty(), "a lending venue moves one asset by design: {found:?}");
}

#[test]
fn a_venue_settling_on_another_chain_is_rejected_unless_it_bridges() {
    let source = venue("confused").replace("chain ethereum", "chain solana");
    let found = errors(&source);
    assert!(
        found.iter().any(|error| error.contains("settles on chain")),
        "only a bridge adapter moves an asset to another chain: {found:?}"
    );
}

/// A fallback that approves a declared venue, with a slippage bound.
fn fallback_approving(bound: u32) -> String {
    format!(
        r#"intent bounded_route {{
    from ethereum.USDC amount 100 receiver 0x1
    to ethereum.ETH receiver 0x2
    route {{
        swap uniswap ethereum.USDC -> ethereum.ETH amount 100 min_output 1
        fallback {{
            replace with declaration
            require slippage <= {bound}
        }}
    }}
    require nonce unused bounded_route_1
    require slippage <= 50
    timeout 30s refund ethereum.USDC to sender
    on_fail rollback
}}
"#
    )
}

#[test]
fn a_fallback_bound_is_checked_against_the_venues_declared_slippage() {
    // This is what the opportunity graph buys the fallback: before it, a venue
    // was only a name, so `require slippage <= 7` bounded a quantity nothing
    // knew. Now the bound has something to be wrong about.
    let source = format!(
        "{}{}",
        venue_with("declaration", "pool", 5, 1_000_000, 20, 2),
        fallback_approving(7)
    );
    let found = errors(&source);
    assert!(
        found
            .iter()
            .any(|error| error.contains("promises something the venue does not offer")),
        "approving a venue that declares more slippage than the block allows must be refused: {found:?}"
    );
}

#[test]
fn a_fallback_bound_inside_the_venues_declared_slippage_is_accepted() {
    // Non-vacuous: the same program with the venue inside the bound compiles.
    let source = format!(
        "{}{}",
        venue_with("declaration", "pool", 5, 1_000_000, 6, 2),
        fallback_approving(7)
    );
    let found = errors(&source);
    assert!(
        found.is_empty(),
        "a venue within the fallback's bound must be approved: {found:?}"
    );
}
