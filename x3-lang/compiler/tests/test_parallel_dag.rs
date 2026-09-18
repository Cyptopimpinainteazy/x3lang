//! The parallel dependency DAG — spec build-order item 18, PHASE 16.
//!
//! PHASE 16 requires three things of this construct: the compiler must build a
//! dependency DAG, parallelism must be deterministic, and races must be
//! rejected or resolved by explicit semantics. These tests are those three.
//!
//! The tests drive the compiler end to end rather than the module in isolation,
//! because the DAG's inputs are *lowered operations*: a read/write set computed
//! from anything else would be a second, weaker opinion about the same question.

use x3_lang_compiler::ir::Operation;
use x3_lang_compiler::semantic::CompilationMode;

fn errors(source: &str) -> Vec<String> {
    match x3_lang_compiler::check_source_diagnostics_with_mode(source, CompilationMode::Dev) {
        Ok((_, _, outcome)) => outcome.errors.iter().map(|error| error.to_string()).collect(),
        Err(error) => vec![format!("{error}")],
    }
}

/// The plan the compiler produced, as `(waves, edges)`.
fn plan(source: &str) -> (Vec<Vec<String>>, Vec<String>) {
    let (_, ir, outcome) =
        x3_lang_compiler::check_source_diagnostics_with_mode(source, CompilationMode::Dev).expect("source must lower");
    assert!(outcome.errors.is_empty(), "unexpected errors: {:?}", outcome.errors);
    ir.operations
        .iter()
        .find_map(|operation| match operation {
            Operation::ParallelPlan { waves, edges } => Some((
                waves.clone(),
                edges
                    .iter()
                    .map(|(from, to)| format!("{from}->{to}"))
                    .collect::<Vec<_>>(),
            )),
            _ => None,
        })
        .expect("the artifact must carry the plan")
}

/// A leg that swaps one asset for another and refunds its input.
fn leg(name: &str, from_asset: &str, to_asset: &str) -> String {
    format!(
        r#"    leg {name} {{
        swap venue_{name} ethereum.{from_asset} -> {to_asset} amount 1000 min_output 1
        require slippage <= 50
        on_fail refund ethereum.{from_asset} to sender
    }}
"#
    )
}

fn parallel(legs: &str) -> String {
    format!("parallel block {{\n{legs}}}\n")
}

#[test]
fn independent_legs_share_a_wave() {
    // Two legs whose assets are disjoint: nothing orders them, so they run
    // together — which is the whole point of the construct.
    let source = parallel(&format!(
        "{}{}",
        leg("buy_eth", "USDC", "ethereum.ETH"),
        leg("buy_sol", "DAI", "solana.SOL")
    ));
    let (waves, edges) = plan(&source);
    assert_eq!(waves, vec![vec!["buy_eth".to_string(), "buy_sol".to_string()]]);
    assert!(edges.is_empty(), "independent legs have no dependency: {edges:?}");
}

#[test]
fn a_consumer_is_ordered_after_its_producer() {
    // `convert` consumes the ETH that `buy_eth` produces, so there is an edge
    // and it lands in a later wave. This is the "resolved by explicit
    // semantics" half of the race requirement.
    let source = parallel(&format!(
        "{}{}{}",
        leg("buy_eth", "USDC", "ethereum.ETH"),
        leg("buy_sol", "DAI", "solana.SOL"),
        leg("convert", "ETH", "ethereum.X3")
    ));
    let (waves, edges) = plan(&source);
    assert_eq!(
        waves,
        vec![
            vec!["buy_eth".to_string(), "buy_sol".to_string()],
            vec!["convert".to_string()]
        ],
        "the consumer waits, the unrelated leg does not"
    );
    assert_eq!(edges, vec!["buy_eth->convert".to_string()]);
}

#[test]
fn two_legs_producing_the_same_asset_are_rejected() {
    // Neither leg depends on the other and both produce ETH: the program does
    // not say which write wins, so there is nothing to order them by. Rejecting
    // is the only honest option; sequencing them silently would invent an
    // execution order the program never expressed.
    let source = parallel(&format!(
        "{}{}",
        leg("alpha", "USDC", "ethereum.ETH"),
        leg("beta", "DAI", "ethereum.ETH")
    ));
    let found = errors(&source);
    assert!(
        found.iter().any(|error| error.contains("both produce ethereum.ETH")
            && error.contains("alpha")
            && error.contains("beta")),
        "a write-write race must be refused and name both legs: {found:?}"
    );
}

#[test]
fn the_plan_does_not_depend_on_declaration_order() {
    // The same two legs declared in the opposite order must produce the same
    // waves with the legs in the same positions: PHASE 42 forbids an optimizer
    // whose output is a property of how a file happens to be laid out.
    let forward = parallel(&format!(
        "{}{}",
        leg("alpha", "USDC", "ethereum.ETH"),
        leg("beta", "DAI", "solana.SOL")
    ));
    let reversed = parallel(&format!(
        "{}{}",
        leg("beta", "DAI", "solana.SOL"),
        leg("alpha", "USDC", "ethereum.ETH")
    ));
    assert_eq!(
        plan(&forward),
        plan(&reversed),
        "the plan must be a function of the legs, not of their order"
    );
}

#[test]
fn a_cycle_is_rejected_rather_than_broken_arbitrarily() {
    // Each leg consumes what the other produces. There is no order to execute
    // in, and inventing one would be the silent wrong answer.
    let source = parallel(&format!(
        "{}{}",
        leg("alpha", "USDC", "ethereum.DAI"),
        leg("beta", "DAI", "ethereum.USDC")
    ));
    let found = errors(&source);
    assert!(
        found.iter().any(|error| error.contains("cycle")),
        "a dependency cycle must be refused: {found:?}"
    );
}

#[test]
fn a_single_leg_is_not_a_parallel_block() {
    let source = parallel(&leg("only", "USDC", "ethereum.ETH"));
    let found = errors(&source);
    assert!(
        found
            .iter()
            .any(|error| error.contains("not") && error.contains("parallel")),
        "a parallel block of one leg would make the artifact's claim false: {found:?}"
    );
}

#[test]
fn a_duplicate_leg_name_is_rejected() {
    let source = parallel(&format!(
        "{}{}",
        leg("same", "USDC", "ethereum.ETH"),
        leg("same", "DAI", "solana.SOL")
    ));
    let found = errors(&source);
    assert!(
        found.iter().any(|error| error.contains("twice")),
        "the plan names legs, so a duplicate makes it ambiguous: {found:?}"
    );
}

#[test]
fn an_empty_leg_is_rejected() {
    let source = format!(
        "parallel thin {{\n    leg empty {{\n    }}\n{} }}\n",
        leg("real", "USDC", "ethereum.ETH")
    );
    let found = errors(&source);
    assert!(
        found.iter().any(|error| error.contains("empty")),
        "a leg with no work contributes no dependencies: {found:?}"
    );
}

#[test]
fn every_leg_of_a_plan_appears_exactly_once() {
    // The artifact's claim is that these legs are the plan. A leg dropped or
    // duplicated between the analysis and the wave list would make the claim
    // describe a different program.
    let source = parallel(&format!(
        "{}{}{}",
        leg("buy_eth", "USDC", "ethereum.ETH"),
        leg("buy_sol", "DAI", "solana.SOL"),
        leg("convert", "ETH", "ethereum.X3")
    ));
    let (waves, _) = plan(&source);
    let mut legs: Vec<&String> = waves.iter().flatten().collect();
    assert_eq!(legs.len(), 3, "all three legs must be in the plan: {waves:?}");
    legs.sort();
    legs.dedup();
    assert_eq!(legs.len(), 3, "no leg may appear twice: {waves:?}");
}
