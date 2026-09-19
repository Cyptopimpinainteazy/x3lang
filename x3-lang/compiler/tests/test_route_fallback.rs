//! Bounded route fallback — spec build-order item 15.
//!
//! The construct's promise is "the runtime may choose only among
//! compiler-approved routes". These tests pin the closedness and boundedness of
//! the approval set, and the one thing the per-substitution check can catch
//! today. The honest limit is recorded at the bottom.

use x3_lang_compiler::ir::Operation;
use x3_lang_compiler::semantic::CompilationMode;

fn errors(source: &str) -> Vec<String> {
    match x3_lang_compiler::check_source_diagnostics_with_mode(source, CompilationMode::Dev) {
        Ok((_, _, outcome)) => outcome.errors.iter().map(|error| error.to_string()).collect(),
        Err(error) => vec![format!("{error}")],
    }
}

fn approved(source: &str) -> Vec<String> {
    let (_, ir, outcome) =
        x3_lang_compiler::check_source_diagnostics_with_mode(source, CompilationMode::Dev).expect("source must lower");
    assert!(outcome.errors.is_empty(), "unexpected errors: {:?}", outcome.errors);
    ir.operations
        .iter()
        .find_map(|operation| match operation {
            Operation::RouteFallback { approved } => Some(approved.clone()),
            _ => None,
        })
        .expect("the artifact must carry the approved venue set")
}

/// A same-chain route with `fallback` lines spliced in.
fn with_fallback(fallback_body: &str) -> String {
    format!(
        r#"intent route_with_fallback {{
    from ethereum.USDC amount 100 receiver 0x1
    to ethereum.ETH receiver 0x2
    route {{
        swap uniswap ethereum.USDC -> ethereum.ETH amount 100 min_output 1
        fallback {{
{fallback_body}
        }}
    }}
    require nonce unused route_fb_1
    require slippage <= 50
    timeout 30s refund ethereum.USDC to sender
    on_fail rollback
}}
"#
    )
}

#[test]
fn an_approved_set_reaches_the_artifact() {
    let source = with_fallback("            replace with curve\n            replace with balancer");
    assert_eq!(
        approved(&source),
        vec!["curve".to_string(), "balancer".to_string()],
        "the approved venues must travel with the artifact, in declaration order"
    );
}

#[test]
fn an_empty_approval_set_is_rejected() {
    let found = errors(&with_fallback("            require slippage <= 7"));
    assert!(
        found.iter().any(|error| error.contains("approves no replacements")),
        "a fallback that approves nothing would leave a failing leg with no checked substitute, got {found:?}"
    );
}

#[test]
fn an_approval_set_above_the_production_bound_is_rejected() {
    let body: String = (0..9)
        .map(|index| format!("            replace with venue{index}\n"))
        .collect();
    let found = errors(&with_fallback(&body));
    assert!(
        found.iter().any(|error| error.contains("production bound")),
        "an unbounded approval set is not a statically bounded fallback, got {found:?}"
    );
}

#[test]
fn a_duplicate_approval_is_rejected() {
    let found = errors(&with_fallback(
        "            replace with curve\n            replace with curve",
    ));
    assert!(
        found.iter().any(|error| error.contains("twice")),
        "a duplicate makes it ambiguous which entry was verified, got {found:?}"
    );
}

#[test]
fn a_guard_that_does_not_bound_a_substitution_is_rejected() {
    // `require nonce unused` says nothing about what a replacement may cost, so
    // accepting it would let the block look like it constrains the runtime
    // while constraining nothing.
    let found = errors(&with_fallback(
        "            replace with curve\n            require nonce unused other_nonce",
    ));
    assert!(
        found
            .iter()
            .any(|error| error.contains("does not bound a substitution")),
        "only guards that bound a substitution belong in a fallback, got {found:?}"
    );
}

#[test]
fn a_non_literal_bound_is_rejected() {
    let found = errors(&with_fallback(
        "            replace with curve\n            require slippage <= some_variable",
    ));
    assert!(
        found.iter().any(|error| error.contains("not an integer literal")),
        "a bound the compiler cannot evaluate does not bound the runtime, got {found:?}"
    );
}

#[test]
fn a_fallback_without_a_swap_leg_is_rejected() {
    // Nothing to replace: the approval set would describe substitutions for a
    // leg that does not exist.
    let source = r#"finality_policy strict {
    chain ethereum
    requirement finalized
    blocks 12
}

intent busy_route {
    from ethereum.USDC amount 100 receiver 0x1
    to ethereum.ETH receiver 0x2
    route {
        bridge x3 ethereum.USDC -> ethereum.ETH receiver 0x2
        fallback {
            replace with curve
        }
    }
    require nonce unused busy_route_1
    require finality.ethereum >= 12
    timeout 30s refund ethereum.USDC to sender
    on_fail rollback
}
"#;
    let found = errors(source);
    assert!(
        found.iter().any(|error| error.contains("no swap leg to replace")),
        "a fallback with no leg to replace must be refused, got {found:?}"
    );
}

#[test]
fn a_venue_the_artifact_could_not_round_trip_is_rejected() {
    // The approved set travels as a comma-separated payload, so a venue
    // containing the separator would come back as two venues and the runtime
    // would hold a different approval set than the compiler verified.
    //
    // Source cannot express this — `curve,balancer` is not an identifier, and
    // the parser says so — so the guard lives where a hand-built IR can still
    // reach it, and is tested there rather than through source that cannot
    // exist.
    let mut ir = x3_lang_compiler::ir::X3IR::new();
    ir.operations = vec![
        Operation::AtomicBegin,
        Operation::RouteFallback {
            approved: vec!["curve,balancer".to_string()],
        },
        Operation::AtomicEnd,
    ];
    let diagnostics = x3_lang_compiler::verify::verify_ir(&ir).expect_err("a comma-bearing venue must be refused");
    assert!(
        diagnostics
            .iter()
            .any(|diagnostic| diagnostic.message.contains("round-trip")),
        "a venue that breaks the payload encoding must not be approved, got {diagnostics:?}"
    );
}

// The honest limit, recorded rather than papered over: the per-substitution
// check re-verifies the route with the replacement venue in place, and the only
// thing that changes is the venue's *name* — no check in the language reads a
// venue's properties, because venues have no declared properties yet. So the
// check proves "the substituted route is as valid as the original" and cannot
// yet catch a venue that is cheaper-looking but unusable, or more expensive
// than the fallback bounds allow. It becomes load-bearing when venues carry
// declared fees and liquidity (spec item 16, the opportunity graph) so the
// `require slippage` / `require profit` bounds have something to compare
// against.
