//! `atomic_choice` — bounded branch representation.
//!
//! The construct's contract is that the compiler enumerated every permitted
//! branch, verified each one, chose one by a criterion it can evaluate, and
//! emitted only that one. These tests pin each clause, including the negative
//! ones: a branch set the compiler cannot rank, a branch that fails a check the
//! winner would have passed, and a branch set that is not equivalent.

use x3_lang_compiler::ir::{Operation, X3IR};
use x3_lang_compiler::semantic::CompilationMode;

fn lower(source: &str) -> Result<X3IR, String> {
    match x3_lang_compiler::check_source_diagnostics_with_mode(source, CompilationMode::Dev) {
        Ok((_, ir, outcome)) => {
            if outcome.errors.is_empty() {
                Ok(ir)
            } else {
                Err(outcome
                    .errors
                    .iter()
                    .map(|error| error.to_string())
                    .collect::<Vec<_>>()
                    .join("; "))
            }
        }
        Err(error) => Err(format!("{error}")),
    }
}

/// The index an `atomic_choice` selected, if the IR carries one.
fn selected(ir: &X3IR) -> Option<u32> {
    ir.operations.iter().find_map(|operation| match operation {
        Operation::AtomicChoice { selected, .. } => Some(*selected),
        _ => None,
    })
}

/// How many `Swap` operations the IR contains, at any depth.
fn swap_count(ir: &X3IR) -> usize {
    ir.operations
        .iter()
        .filter(|operation| matches!(operation, Operation::Swap { .. }))
        .count()
}

fn errors(source: &str) -> Vec<String> {
    match x3_lang_compiler::check_source_with_mode(source, CompilationMode::Dev) {
        Ok((_, _, errors)) => errors.iter().map(|error| format!("{error}")).collect(),
        Err(error) => vec![format!("{error}")],
    }
}

/// Two paths, the first worth more, both producing `ETH`.
fn two_paths(first_output: &str, second_output: &str) -> String {
    format!(
        r#"atomic_choice best_route {{
    path via_uniswap {{
        swap uniswap ethereum.USDC -> ethereum.ETH amount 100 min_output 1
        require slippage <= 50
        on_fail refund ethereum.USDC to sender
        net_output {first_output} ETH
    }}
    path via_curve {{
        swap curve ethereum.USDC -> ethereum.ETH amount 100 min_output 1
        require slippage <= 50
        on_fail refund ethereum.USDC to sender
        net_output {second_output} ETH
    }}
    choose highest_net_output;
}}
"#
    )
}

#[test]
fn a_choice_selects_the_branch_the_criterion_ranks_first() {
    let lowered = lower(&two_paths("108", "104")).expect("a well-formed choice must compile");
    assert_eq!(
        selected(&lowered),
        Some(0),
        "highest_net_output must take the 108 branch"
    );
}

#[test]
fn the_criterion_actually_decides_and_not_the_declaration_order() {
    // Non-vacuous: the second path wins when it is worth more. Without this,
    // a selector that always returned path 0 would pass the test above.
    let lowered = lower(&two_paths("104", "108")).expect("a well-formed choice must compile");
    assert_eq!(
        selected(&lowered),
        Some(1),
        "highest_net_output must take the 108 branch whichever order it is declared in"
    );
}

#[test]
fn only_the_selected_branch_reaches_the_artifact() {
    // The unselected branch is verified but must not be emitted: a runtime that
    // can run a second branch is not a bounded choice.
    let lowered = lower(&two_paths("108", "104")).expect("a well-formed choice must compile");
    assert_eq!(swap_count(&lowered), 1, "exactly one branch may be emitted");
}

#[test]
fn every_branch_is_verified_not_only_the_winner() {
    // The losing branch has no refund path, so it fails a check the winning
    // branch passes. If only the winner were verified this program would
    // compile, and the compiler's "every branch was checked" claim would be
    // false for exactly the branches nobody looks at.
    let source = r#"atomic_choice best_route {
    path good {
        swap uniswap ethereum.USDC -> ethereum.ETH amount 100 min_output 1
        require slippage <= 50
        on_fail refund ethereum.USDC to sender
        net_output 108 ETH
    }
    path bad {
        swap curve ethereum.USDC -> ethereum.ETH amount 100 min_output 1
        require slippage <= 50
        net_output 104 ETH
    }
    choose highest_net_output;
}
"#;
    let found = errors(source);
    assert!(
        found.iter().any(|error| error.contains("path 'bad'")),
        "the losing branch's missing refund path must be reported and must name the branch, got {found:?}"
    );
}

#[test]
fn branches_that_do_not_produce_the_same_asset_are_rejected() {
    let source = r#"atomic_choice mismatched {
    path a {
        swap uniswap ethereum.USDC -> ethereum.ETH amount 100 min_output 1
        require slippage <= 50
        on_fail refund ethereum.USDC to sender
        net_output 108 ETH
    }
    path b {
        swap curve ethereum.USDC -> ethereum.USDC amount 100 min_output 1
        require slippage <= 50
        on_fail refund ethereum.USDC to sender
        net_output 104 USDC
    }
    choose highest_net_output;
}
"#;
    let found = errors(source);
    assert!(
        found.iter().any(|error| error.contains("same output asset")),
        "a choice between different output assets must be rejected, got {found:?}"
    );
}

#[test]
fn a_path_with_no_declared_output_cannot_be_ranked() {
    let source = r#"atomic_choice unranked {
    path a {
        swap uniswap ethereum.USDC -> ethereum.ETH amount 100 min_output 1
        require slippage <= 50
        on_fail refund ethereum.USDC to sender
    }
    path b {
        swap curve ethereum.USDC -> ethereum.ETH amount 100 min_output 1
        require slippage <= 50
        on_fail refund ethereum.USDC to sender
        net_output 104 ETH
    }
    choose highest_net_output;
}
"#;
    let found = errors(source);
    assert!(
        found.iter().any(|error| error.contains("net_output")),
        "a branch whose output the compiler cannot evaluate must be refused, got {found:?}"
    );
}

#[test]
fn an_unknown_criterion_is_a_parse_error_naming_the_closed_set() {
    let source = two_paths("108", "104").replace("choose highest_net_output", "choose whatever_looks_good");
    let found = errors(&source);
    assert!(
        found
            .iter()
            .any(|error| error.contains("unknown choice criterion") && error.contains("highest_net_output")),
        "the criterion set is closed and the error must say so, got {found:?}"
    );
}

#[test]
fn a_branch_set_larger_than_the_production_bound_is_rejected() {
    let mut paths = String::new();
    for index in 0..9 {
        paths.push_str(&format!(
            r#"    path p{index} {{
        swap uniswap ethereum.USDC -> ethereum.ETH amount 100 min_output 1
        require slippage <= 50
        on_fail refund ethereum.USDC to sender
        net_output {} ETH
    }}
"#,
            100 + index
        ));
    }
    let source = format!("atomic_choice too_many {{\n{paths}    choose highest_net_output;\n}}\n");
    let found = errors(&source);
    assert!(
        found.iter().any(|error| error.contains("production bound")),
        "above the bound the construct stops being bounded execution, got {found:?}"
    );
}
