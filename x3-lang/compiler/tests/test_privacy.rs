//! Private strategy commitments and submission policy — item 24 (PHASE 27/28).
//!
//! PHASE 27 asks for private execution and, in the same breath, says not to
//! invent cryptographic guarantees that are not implemented, with each privacy
//! level clearly labelled. The tests below are both halves: what a program may
//! declare, and what it is refused for declaring.

use x3_lang_compiler::ir::Operation;
use x3_lang_compiler::semantic::{implemented_privacy_levels, CompilationMode};

fn errors(source: &str) -> Vec<String> {
    match x3_lang_compiler::check_source_diagnostics_with_mode(source, CompilationMode::Dev) {
        Ok((_, _, outcome)) => outcome.errors.iter().map(|error| error.to_string()).collect(),
        Err(error) => vec![format!("{error}")],
    }
}

fn operations(source: &str) -> Vec<Operation> {
    let (_, ir, outcome) =
        x3_lang_compiler::check_source_diagnostics_with_mode(source, CompilationMode::Dev).expect("source must lower");
    assert!(outcome.errors.is_empty(), "unexpected errors: {:?}", outcome.errors);
    ir.operations
}

/// A module with the eight declarations and an optional extra section.
fn module(extra: &str) -> String {
    format!(
        r#"strategy PrivateArb {{
    input ethereum.USDC amount 1_000_000
    output ethereum.ETH
    effects [swap]
    domains [ethereum]
    risk {{ max_slippage_bps 50 max_total_fee_bps 8 }}
{extra}    bounds {{ max_steps 10 max_gas 200_000 }}
    execute {{
        swap uniswap ethereum.USDC -> ethereum.ETH amount 1_000 min_output 1
        require slippage <= 50
        on_fail refund ethereum.USDC to sender
    }}
}}
"#
    )
}

const REQUIRED: &str = "    submission { private = required }\n";

#[test]
fn declaring_encryption_is_refused_because_nothing_implements_it() {
    // The executor reads `encrypted` as `encrypted: _`. A program that declared
    // it compiled into an artifact carrying a claim of encryption with nothing
    // behind it, which is the specific failure PHASE 27 names.
    let source = "privacy {\n    hide_route_until_commit true\n    reveal_on claim\n    encrypted true\n}\n";
    let found = errors(source);
    assert!(
        found
            .iter()
            .any(|error| error.contains("no mechanism in this language implements encryption")),
        "the unimplemented guarantee must be refused: {found:?}"
    );
}

#[test]
fn the_refusal_labels_the_levels_that_are_implemented() {
    // "Clearly label each privacy level": the refusal has to say what *is*
    // available, not only what is not.
    let source = "privacy {\n    hide_route_until_commit true\n    reveal_on claim\n    encrypted true\n}\n";
    let found = errors(source);
    let message = found.join(" ");
    for level in implemented_privacy_levels() {
        assert!(
            message.contains(level.0),
            "the refusal must name the implemented level '{}': {message}",
            level.0
        );
    }
}

#[test]
fn the_commitment_level_that_is_implemented_is_accepted() {
    // Non-vacuous: hiding the route until a reveal point is real (PRIVACY_COMMIT
    // is emitted and executed), so it stays declarable.
    let source = "privacy {\n    hide_route_until_commit true\n    reveal_on claim\n}\n";
    let found = errors(source);
    assert!(found.is_empty(), "the implemented level must compile: {found:?}");
}

#[test]
fn declaring_that_encryption_is_off_is_fine() {
    let source = "privacy {\n    hide_route_until_commit true\n    reveal_on claim\n    encrypted false\n}\n";
    let found = errors(source);
    assert!(found.is_empty(), "`encrypted false` claims nothing: {found:?}");
}

#[test]
fn a_required_private_submission_reaches_the_artifact_as_a_mode_check() {
    let ops = operations(&module(REQUIRED));
    let check = ops.iter().find_map(|op| match op {
        Operation::ModeCheck { mode, restriction } => Some((mode.clone(), restriction.clone())),
        _ => None,
    });
    assert_eq!(
        check,
        Some(("submission".to_string(), "private_required".to_string())),
        "the policy has to be in the artifact for a runtime to enforce it"
    );
}

#[test]
fn a_module_without_a_submission_policy_emits_no_mode_check() {
    let ops = operations(&module(""));
    assert!(
        !ops.iter().any(|op| matches!(op, Operation::ModeCheck { .. })),
        "a module that does not require privacy must not claim to"
    );
}

#[test]
fn an_unknown_submission_mode_names_the_closed_set() {
    let source = module("    submission { private = maybe }\n");
    let found = errors(&source);
    assert!(
        found
            .iter()
            .any(|error| error.contains("unknown submission mode") && error.contains("required")),
        "the mode set is closed and the error lists it: {found:?}"
    );
}

#[test]
fn a_submission_policy_without_a_mode_is_refused() {
    let source = module("    submission { }\n");
    let found = errors(&source);
    assert!(
        found.iter().any(|error| error.contains("missing `private =")),
        "a policy that does not state a mode is not a policy: {found:?}"
    );
}

#[test]
fn the_preferred_and_allowed_modes_do_not_demand_a_private_channel() {
    // Only `required` is a demand on the runtime; the other two are recorded so
    // the artifact states the intent without refusing a public submission.
    for mode in ["preferred", "allowed"] {
        let ops = operations(&module(&format!("    submission {{ private = {mode} }}\n")));
        let restriction = ops.iter().find_map(|op| match op {
            Operation::ModeCheck { restriction, .. } => Some(restriction.clone()),
            _ => None,
        });
        assert_eq!(restriction, Some(format!("private_{mode}")));
    }
}
