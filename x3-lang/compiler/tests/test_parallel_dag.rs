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
            Operation::ParallelPlan { waves, edges, .. } => Some((
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
        leg("buy_sol", "DAI", "ethereum.SOL")
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
        leg("buy_sol", "DAI", "ethereum.SOL"),
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
        leg("beta", "DAI", "ethereum.SOL")
    ));
    let reversed = parallel(&format!(
        "{}{}",
        leg("beta", "DAI", "ethereum.SOL"),
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
        leg("buy_sol", "DAI", "ethereum.SOL"),
        leg("convert", "ETH", "ethereum.X3")
    ));
    let (waves, _) = plan(&source);
    let mut legs: Vec<&String> = waves.iter().flatten().collect();
    assert_eq!(legs.len(), 3, "all three legs must be in the plan: {waves:?}");
    legs.sort();
    legs.dedup();
    assert_eq!(legs.len(), 3, "no leg may appear twice: {waves:?}");
}

/// A `vm` declaration mapping a chain to a VM family.
fn vm(chain: &str, adapter: &str) -> String {
    format!("vm {{\n    chain {chain}\n    adapter {adapter}\n    finality safe\n}}\n")
}

/// The per-leg domains the plan reports.
fn domains(source: &str) -> std::collections::BTreeMap<String, Vec<String>> {
    let (_, ir, outcome) =
        x3_lang_compiler::check_source_diagnostics_with_mode(source, CompilationMode::Dev).expect("source must lower");
    assert!(outcome.errors.is_empty(), "unexpected errors: {:?}", outcome.errors);
    ir.operations
        .iter()
        .find_map(|operation| match operation {
            Operation::ParallelPlan { domains, .. } => Some(
                domains
                    .iter()
                    .map(|(leg, set)| (leg.clone(), set.iter().cloned().collect::<Vec<_>>()))
                    .collect(),
            ),
            _ => None,
        })
        .expect("the artifact must carry the plan")
}

#[test]
fn a_cross_chain_leg_without_a_bridge_step_is_refused() {
    // Moving value between chains takes a step that says so. This used to pass:
    // the IR dropped a swap's output chain, so a leg could move value across a
    // domain boundary and nothing could see it. With the output chain carried,
    // the leg names two chains and has no bridge among them.
    let source = parallel(&format!(
        "{}{}",
        leg("buy_eth", "USDC", "ethereum.ETH"),
        leg("buy_sol", "DAI", "solana.SOL")
    ));
    let found = errors(&source);
    assert!(
        found
            .iter()
            .any(|error| error.contains("touches 2 chains") && error.contains("cross-chain step")),
        "a leg that moves value between chains implicitly must be refused: {found:?}"
    );
}

#[test]
fn a_cross_chain_leg_with_a_bridge_step_is_accepted() {
    // Non-vacuous: the same movement, said out loud, is fine.
    let source = r#"parallel cross {
    leg a {
        swap uniswap ethereum.USDC -> ethereum.ETH amount 1 min_output 1
        require slippage <= 50
        require nonce unused bridge_plan_1
        on_fail refund ethereum.USDC to sender
    }
    leg b {
        swap uniswap ethereum.DAI -> ethereum.X3 amount 1 min_output 1
        require slippage <= 50
        on_fail refund ethereum.DAI to sender
    }
    leg c {
        bridge x3 ethereum.ETH -> solana.SOL amount 1 receiver 0x1
        require finality.ethereum >= 12
        timeout 30s refund ethereum.ETH to sender
        on_fail refund ethereum.ETH to sender
    }
}
"#;
    let found = errors(&source);
    assert!(
        found.is_empty(),
        "a leg that bridges explicitly must compile cleanly, not merely avoid one message: {found:?}"
    );
}

#[test]
fn declared_vm_families_are_what_the_plan_reports() {
    let source = format!(
        "{}{}{}",
        vm("ethereum", "evm"),
        vm("solana", "svm"),
        parallel(&format!(
            "{}{}",
            leg("buy_eth", "USDC", "ethereum.ETH"),
            leg("buy_sol", "DAI", "ethereum.SOL")
        ))
    );
    let domains = domains(&source);
    assert_eq!(
        domains.get("buy_eth"),
        Some(&vec!["evm".to_string()]),
        "the plan names the VM family the declaration gave, not the chain: {domains:?}"
    );
    assert_eq!(domains.get("buy_sol"), Some(&vec!["evm".to_string()]));
}

#[test]
fn an_undeclared_chain_is_its_own_domain() {
    // Without a declaration the compiler can only honestly report the chain.
    let source = parallel(&format!(
        "{}{}",
        leg("buy_eth", "USDC", "ethereum.ETH"),
        leg("buy_sol", "DAI", "ethereum.SOL")
    ));
    let domains = domains(&source);
    assert_eq!(
        domains.get("buy_eth"),
        Some(&vec!["ethereum".to_string()]),
        "an undeclared chain is its own domain: {domains:?}"
    );
}

#[test]
fn two_declarations_disagreeing_about_one_chain_are_refused() {
    // The plan would have to say which VM executes a leg on this chain, and two
    // declarations mean it cannot.
    let source = format!(
        "{}{}{}",
        vm("ethereum", "evm"),
        vm("ethereum", "svm"),
        parallel(&format!(
            "{}{}",
            leg("buy_eth", "USDC", "ethereum.ETH"),
            leg("buy_sol", "DAI", "ethereum.SOL")
        ))
    );
    let found = errors(&source);
    assert!(
        found.iter().any(|error| error.contains("declared on two domains")),
        "an ambiguous chain-to-VM mapping must be refused: {found:?}"
    );
}

/// The per-wave settlement records the plan carries.
fn settlement(source: &str) -> Vec<(usize, Vec<String>, Vec<String>, bool)> {
    let (_, ir, outcome) =
        x3_lang_compiler::check_source_diagnostics_with_mode(source, CompilationMode::Dev).expect("source must lower");
    assert!(outcome.errors.is_empty(), "unexpected errors: {:?}", outcome.errors);
    ir.operations
        .iter()
        .find_map(|operation| match operation {
            Operation::ParallelPlan { settlement, .. } => Some(
                settlement
                    .iter()
                    .map(|record| {
                        (
                            record.wave,
                            record.domains.iter().cloned().collect::<Vec<_>>(),
                            record.outstanding_proofs.iter().cloned().collect::<Vec<_>>(),
                            record.locally_recoverable,
                        )
                    })
                    .collect(),
            ),
            _ => None,
        })
        .expect("the artifact must carry the settlement section")
}

/// A parallel block where the last leg bridges to another chain.
fn bridging_plan() -> String {
    r#"parallel cross {
    leg alpha {
        swap uniswap ethereum.USDC -> ethereum.ETH amount 1 min_output 1
        require slippage <= 50
        require nonce unused cross_plan_1
        on_fail refund ethereum.USDC to sender
    }
    leg beta {
        swap uniswap ethereum.DAI -> ethereum.X3 amount 1 min_output 1
        require slippage <= 50
        on_fail refund ethereum.DAI to sender
    }
    leg bridge_out {
        bridge x3 ethereum.ETH -> solana.SOL amount 1 receiver 0x1
        require finality.ethereum >= 12
        timeout 30s refund ethereum.ETH to sender
        on_fail refund ethereum.ETH to sender
    }
}
"#
    .to_string()
}

#[test]
fn a_wave_over_one_domain_is_locally_recoverable() {
    let source = format!(
        "{}{}",
        vm("ethereum", "evm"),
        parallel(&format!(
            "{}{}",
            leg("buy_eth", "USDC", "ethereum.ETH"),
            leg("buy_sol", "DAI", "ethereum.SOL")
        ))
    );
    let records = settlement(&source);
    assert_eq!(records.len(), 1, "one wave, one record: {records:?}");
    assert!(records[0].3, "a single-domain wave is undoable by the VM alone");
}

#[test]
fn a_wave_over_two_domains_is_not_locally_recoverable() {
    // This is the fact a coordinator needs and that nothing else in the plan
    // states: part of this wave is beyond the reach of the local rollback.
    let source = format!(
        "{}{}{}{}",
        vm("ethereum", "evm"),
        vm("solana", "svm"),
        concurrent_across_domains(),
        ""
    );
    let records = settlement(&source);
    assert_eq!(records.len(), 1, "one wave: {records:?}");
    assert_eq!(records[0].1, vec!["evm".to_string(), "svm".to_string()]);
    assert!(
        !records[0].3,
        "a wave spanning two domains cannot be undone by this VM alone"
    );
}

/// A wave containing one `evm` leg and one `svm` leg, with no dependency
/// between them.
fn concurrent_across_domains() -> String {
    r#"parallel cross {
    leg on_eth {
        swap uniswap ethereum.USDC -> ethereum.ETH amount 1 min_output 1
        require slippage <= 50
        on_fail refund ethereum.USDC to sender
    }
    leg on_sol {
        swap raydium solana.USDC -> solana.SOL amount 1 min_output 1
        require slippage <= 50
        on_fail refund solana.USDC to sender
    }
}
"#
    .to_string()
}

#[test]
fn a_bridge_without_its_proof_inputs_reports_them_as_outstanding() {
    // The bridge takes a source-finality proof and a transfer proof as inputs.
    // A program that writes the bridge without them leaves the obligation open,
    // and the plan says so, naming the chain each proof is owed on.
    let source = format!("{}{}", vm("ethereum", "evm"), bridging_plan());
    let records = settlement(&source);
    let bridging_wave = records
        .iter()
        .find(|(_, domains, _, _)| domains.contains(&"solana".to_string()))
        .expect("the bridging wave must be recorded");
    assert_eq!(
        bridging_wave.2,
        vec![
            "ethereum:source_finality_proof".to_string(),
            "solana:transfer_proof".to_string()
        ],
        "the outstanding proofs must name the chain they are owed on: {records:?}"
    );
}

#[test]
fn a_bridge_with_its_proof_inputs_reports_none_outstanding() {
    // Non-vacuous: the same bridge, said with its proofs, owes nothing.
    let source = format!(
        "{}{}",
        vm("ethereum", "evm"),
        r#"parallel cross {
    leg alpha {
        swap uniswap ethereum.USDC -> ethereum.ETH amount 1 min_output 1
        require slippage <= 50
        require nonce unused cross_plan_1
        on_fail refund ethereum.USDC to sender
    }
    leg beta {
        swap uniswap ethereum.DAI -> ethereum.X3 amount 1 min_output 1
        require slippage <= 50
        on_fail refund ethereum.DAI to sender
    }
    leg bridge_out {
        bridge x3 ethereum.ETH -> solana.SOL amount 1 receiver 0x1 transfer_proof eth_receipt
        require finality.ethereum >= 12
        timeout 30s refund ethereum.ETH to sender
        on_fail refund ethereum.ETH to sender
    }
}
"#
    );
    let records = settlement(&source);
    let bridging_wave = records
        .iter()
        .find(|(_, domains, _, _)| domains.contains(&"solana".to_string()))
        .expect("the bridging wave must be recorded");
    assert!(
        !bridging_wave.2.iter().any(|proof| proof.contains("transfer_proof")),
        "a bridge that carries its transfer proof owes none: {records:?}"
    );
}

#[test]
fn every_wave_has_a_settlement_record_indexed_to_it() {
    let source = format!("{}{}", vm("ethereum", "evm"), bridging_plan());
    let records = settlement(&source);
    for (index, record) in records.iter().enumerate() {
        assert_eq!(record.0, index, "records are indexed by wave: {records:?}");
    }
}
