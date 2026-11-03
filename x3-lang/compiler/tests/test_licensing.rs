//! Strategy licensing, royalties and profit splitting — items 23 (PHASE 24/25).
//!
//! The phase's constraint is "licensing must never compromise deterministic
//! execution", and one test below is that constraint stated as something the
//! compiler can be held to: the executable stream is identical with and without
//! a licence, and only a non-executing record differs.

use x3_lang_compiler::ir::Operation;
use x3_lang_compiler::semantic::CompilationMode;

fn errors(source: &str) -> Vec<String> {
    match x3_lang_compiler::check_source_diagnostics_with_mode(source, CompilationMode::Dev) {
        Ok((_, _, outcome)) => outcome.errors.iter().map(|error| error.to_string()).collect(),
        Err(error) => vec![format!("{error}")],
    }
}

/// The lowered operations of a module, for the determinism comparison.
fn operations(source: &str) -> Vec<Operation> {
    let (_, ir, outcome) =
        x3_lang_compiler::check_source_diagnostics_with_mode(source, CompilationMode::Dev).expect("source must lower");
    assert!(outcome.errors.is_empty(), "unexpected errors: {:?}", outcome.errors);
    ir.operations
}

/// A module whose body asserts a profit floor, so a split has something to
/// distribute.
fn module(extra: &str) -> String {
    format!(
        r#"strategy TriDexArb {{
    input ethereum.USDC amount 25_000_000
    output ethereum.ETH
    effects [swap]
    guarantees [min_profit]
    domains [ethereum]
    risk {{ max_slippage_bps 50 max_total_fee_bps 8 }}
{extra}    bounds {{ max_steps 10 max_gas 200_000 }}
    execute {{
        swap uniswap ethereum.USDC -> ethereum.ETH amount 1_000 min_output 1
        require slippage <= 50
        require profit >= 5
        on_fail refund ethereum.USDC to sender
    }}
}}
"#
    )
}

const LICENSE: &str = "    license { creator x3_creator profit_share 3% executions 10_000 expires_block 88_000_000 }\n";

fn split(shares: &str) -> String {
    format!("    split profit {{\n{shares}    }}\n")
}

fn standard_split() -> String {
    split("        92% -> trader\n        5% -> liquidity_provider\n        3% -> strategy_author\n")
}

#[test]
fn a_licensed_module_with_a_sound_split_compiles() {
    let source = module(&format!("{LICENSE}{}", standard_split()));
    let found = errors(&source);
    assert!(found.is_empty(), "a licensed module must compile: {found:?}");
}

#[test]
fn a_split_that_does_not_total_one_hundred_percent_is_rejected() {
    let source = module(&format!(
        "{LICENSE}{}",
        split("        92% -> trader\n        5% -> liquidity_provider\n")
    ));
    let found = errors(&source);
    assert!(
        found.iter().any(|error| error.contains("totals 9700 bps")),
        "a split that does not add up must be refused with the total: {found:?}"
    );
}

#[test]
fn a_royalty_the_split_does_not_pay_is_rejected() {
    // The licence grants 3% and the split pays the author 1%: the artifact
    // promises something it does not do.
    let source = module(&format!(
        "{LICENSE}{}",
        split("        94% -> trader\n        5% -> liquidity_provider\n        1% -> strategy_author\n")
    ));
    let found = errors(&source);
    assert!(
        found
            .iter()
            .any(|error| error.contains("licence grants the author 300 bps") && error.contains("pays 100 bps")),
        "a royalty the split does not pay must be refused: {found:?}"
    );
}

#[test]
fn a_licence_with_no_split_to_pay_it_from_is_rejected() {
    let source = module(LICENSE);
    let found = errors(&source);
    assert!(
        found.iter().any(|error| error.contains("no `split profit`")),
        "a royalty with nothing to pay it from is the same unkept promise: {found:?}"
    );
}

#[test]
fn a_split_without_a_profit_floor_is_rejected() {
    // PHASE 25: distribution happens after final net profit is known. Without a
    // floor there is no net profit the split applies to.
    let source = module(&standard_split()).replace("        require profit >= 5\n", "");
    let found = errors(&source);
    assert!(
        found.iter().any(|error| error.contains("asserts no profit floor")),
        "a split needs a profit floor to distribute: {found:?}"
    );
}

#[test]
fn a_profit_split_may_stand_alone_without_a_licence() {
    // Non-vacuous: an unlicensed module may still split its profit.
    let source = module(&standard_split());
    let found = errors(&source);
    assert!(found.is_empty(), "an unlicensed split is fine: {found:?}");
}

#[test]
fn naming_a_recipient_twice_is_rejected() {
    let source = module(&split(
        "        50% -> trader\n        47% -> trader\n        3% -> strategy_author\n",
    ));
    let found = errors(&source);
    assert!(
        found.iter().any(|error| error.contains("'trader' twice")),
        "a duplicated recipient makes the shares ambiguous: {found:?}"
    );
}

#[test]
fn a_zero_share_is_rejected() {
    // 100% accounted for, but the author gets nothing while the licence says 3%.
    let source = module(&format!(
        "{LICENSE}{}",
        split("        92% -> trader\n        5% -> liquidity_provider\n        3% -> validator_pool\n")
    ));
    let found = errors(&source);
    assert!(
        found.iter().any(|error| error.contains("pays 0 bps")),
        "a royalty the author does not receive must be refused: {found:?}"
    );
}

#[test]
fn an_unknown_recipient_names_the_closed_set() {
    let source = module(&split(
        "        92% -> trader\n        5% -> liquidity_provider\n        3% -> my_friend\n",
    ));
    let found = errors(&source);
    assert!(
        found
            .iter()
            .any(|error| error.contains("unknown split recipient") && error.contains("strategy_author")),
        "the recipient set is closed and the error lists it: {found:?}"
    );
}

#[test]
fn a_share_finer_than_one_percent_says_it_is_not_expressible() {
    let source = module(&split("        99.5% -> trader\n        0.5% -> strategy_author\n"));
    let found = errors(&source);
    assert!(
        found.iter().any(|error| error.contains("finer than one percent")),
        "the granularity limit must be stated rather than silently rounded: {found:?}"
    );
}

#[test]
fn a_licence_does_not_change_the_executable_stream() {
    // PHASE 24: "licensing must never compromise deterministic execution". The
    // licence is a record; the instructions that actually run are identical with
    // and without it.
    let without = operations(&module(&standard_split()));
    let with = operations(&module(&format!("{LICENSE}{}", standard_split())));
    let executable = |ops: &[Operation]| -> Vec<String> {
        ops.iter()
            .filter(|op| !matches!(op, Operation::StrategyLicense { .. }))
            .map(|op| format!("{op:?}"))
            .collect()
    };
    assert_eq!(
        executable(&without),
        executable(&with),
        "a licence must not add, remove or reorder a single executable instruction"
    );
    assert!(
        with.iter().any(|op| matches!(op, Operation::StrategyLicense { .. })),
        "and the licence must be recorded"
    );
}

#[test]
fn the_royalty_reaches_the_artifact_as_basis_points() {
    let ops = operations(&module(&format!("{LICENSE}{}", standard_split())));
    let record = ops
        .iter()
        .find_map(|op| match op {
            Operation::StrategyLicense {
                creator,
                royalty_bps,
                split,
                executions,
                expires_block,
            } => Some((
                creator.clone(),
                *royalty_bps,
                split.clone(),
                *executions,
                *expires_block,
            )),
            _ => None,
        })
        .expect("the licence must be recorded in the artifact");
    assert_eq!(record.0, "x3_creator");
    assert_eq!(record.1, 300, "3% is 300 basis points, not a float");
    assert_eq!(record.3, Some(10_000));
    assert_eq!(record.4, Some(88_000_000));
    assert_eq!(
        record.2,
        vec![
            ("trader".to_string(), 9200),
            ("liquidity_provider".to_string(), 500),
            ("strategy_author".to_string(), 300)
        ]
    );
}

#[test]
fn a_licence_creator_may_be_written_quoted() {
    // An identity is opaque to the compiler, so the spelling the outside world
    // uses has to survive being written as a string.
    let source = module(&format!(
        "    license {{ creator \"x3:0xABC\" profit_share 3% }}\n{}",
        standard_split()
    ));
    let found = errors(&source);
    assert!(found.is_empty(), "a quoted creator must parse: {found:?}");
}
