//! Execution lanes — spec build-order item 30, PHASE 30.
//!
//! Two things are tested. The first is that a lane is *read from what a program
//! does*: a swap is in the trading lane because it swaps, not because anyone said
//! so, and a program that does several things lands in the most constrained lane it
//! needs. The second is that the scheduling policy is total, stable, and has no
//! input a participant could use to buy position — PHASE 30 forbids unfair ordering
//! mechanisms, and the surest way to have none is to have no parameter for one.

use x3_lang_compiler::ir::{Operation, ProgramMetadata, ReleaseAct, X3IR};
use x3_lang_compiler::lanes::{self, Lane, Queued};

/// A program with exactly the operations a test wants to classify.
fn ir_with(operations: Vec<Operation>) -> X3IR {
    X3IR {
        operations,
        metadata: ProgramMetadata {
            nonce: Some("nonce-1".to_owned()),
            chain_id: Some(1),
            timeout_blocks: Some(10),
        },
    }
}

fn lane_of_ir(operations: Vec<Operation>) -> Lane {
    lanes::classify(&ir_with(operations))
}

/// The lane of every declaration in a source, proving the lane is reached from
/// source rather than only from hand-built IR.
fn lanes_of(source: &str) -> Vec<(String, Lane)> {
    let program = x3_lang_compiler::parser::parse_source(source).expect("the source must parse");
    lanes::classify_program(&program)
        .into_iter()
        .map(|(name, lane)| (name, lane.expect("the fixture lowers")))
        .collect()
}

/// An intent that only moves value it already has.
const ONLY_MOVES: &str = "intent only_moves {\n    from ethereum.USDC amount 100 receiver 0xA1\n    \
                          to ethereum.USDC receiver 0xA2\n    require nonce unused \
                          only_moves_nonce\n    timeout 30s refund ethereum.USDC to sender\n    \
                          on_fail rollback\n}\n";

/// An intent that swaps within one chain.
const TRADES: &str = "intent trades {\n    from ethereum.USDC amount 100 receiver 0xA1\n    to \
                      ethereum.SOL receiver 0xA2\n    route {\n        swap uniswap \
                      ethereum.USDC -> ethereum.SOL amount 100 min_output 90\n    }\n    require \
                      nonce unused trades_nonce\n    timeout 30s refund ethereum.USDC to sender\n    \
                      on_fail rollback\n}\n";

/// An intent that swaps value onto another ledger.
const CROSSES: &str = "intent crosses {\n    from ethereum.USDC amount 100 receiver 0xA1\n    to \
                       solana.USDC receiver 0xA2\n    route {\n        swap uniswap \
                       ethereum.USDC -> solana.USDC amount 100 min_output 90\n    }\n    require \
                       nonce unused crosses_nonce\n    timeout 30s refund ethereum.USDC to sender\n    \
                       on_fail rollback\n}\n";

#[test]
fn the_priority_sequence_is_a_total_order_over_every_lane() {
    // The cross-lane policy is a fixed sequence a reader can audit. A lane missing
    // from it would make `rank` panic; a lane listed twice would make two lanes
    // indistinguishable.
    assert_eq!(lanes::PRIORITY.len(), Lane::ALL.len());
    let mut ranks: Vec<usize> = Lane::ALL.iter().map(|lane| lane.rank()).collect();
    ranks.sort_unstable();
    assert_eq!(
        ranks,
        (0..Lane::ALL.len()).collect::<Vec<_>>(),
        "every lane has exactly one place in the sequence"
    );
}

#[test]
fn a_program_that_only_moves_value_is_in_the_settlement_lane() {
    assert_eq!(
        lane_of_ir(vec![
            Operation::Lock {
                chain: "ethereum".to_owned(),
                asset: "USDC".to_owned(),
                amount: 100,
                from: "sender".to_owned(),
            },
            Operation::Release {
                chain: "ethereum".to_owned(),
                asset: "USDC".to_owned(),
                to: "receiver".to_owned(),
                act: ReleaseAct::Claims(0),
            },
        ]),
        Lane::Settlement
    );
}

#[test]
fn a_program_that_swaps_is_in_the_trading_lane() {
    assert_eq!(
        lane_of_ir(vec![Operation::Swap {
            from_chain: "ethereum".to_owned(),
            from_asset: "USDC".to_owned(),
            to_chain: "ethereum".to_owned(),
            to_asset: "SOL".to_owned(),
            input_amount: 100,
            min_output: 90,
            dex: Some("uniswap".to_owned()),
        }]),
        Lane::Trading
    );
}

#[test]
fn a_swap_that_moves_value_between_ledgers_is_in_the_cross_domain_lane() {
    // `ethereum.USDC -> solana.USDC` is one swap over two ledgers, and the operation
    // carries both chains for exactly this reason.
    assert_eq!(
        lane_of_ir(vec![Operation::Swap {
            from_chain: "ethereum".to_owned(),
            from_asset: "USDC".to_owned(),
            to_chain: "solana".to_owned(),
            to_asset: "USDC".to_owned(),
            input_amount: 100,
            min_output: 90,
            dex: Some("uniswap".to_owned()),
        }]),
        Lane::AtomicCrossDomain
    );
}

#[test]
fn a_bridge_is_in_the_cross_domain_lane() {
    assert_eq!(
        lane_of_ir(vec![Operation::Bridge {
            min_output: 0,
            via: "X3".to_owned(),
            from_chain: "ethereum".to_owned(),
            from_asset: "USDC".to_owned(),
            to_chain: "solana".to_owned(),
            to_asset: "USDC".to_owned(),
            amount: 100,
            receiver: "0xB".to_owned(),
            source_finality_proof: vec![1],
            transfer_proof: vec![2],
        }]),
        Lane::AtomicCrossDomain,
        "a bridge crosses ledgers by construction"
    );
}

#[test]
fn a_liquidation_is_in_the_liquidation_lane_even_though_it_also_swaps() {
    // The rule the priority order encodes: a program that needs the liquidation
    // executor needs it whether or not it also swaps, and putting it in the trading
    // lane would hide the guarantee it depends on.
    let lane = lane_of_ir(vec![
        Operation::Swap {
            from_chain: "ethereum".to_owned(),
            from_asset: "USDC".to_owned(),
            to_chain: "ethereum".to_owned(),
            to_asset: "ETH".to_owned(),
            input_amount: 100,
            min_output: 90,
            dex: Some("uniswap".to_owned()),
        },
        Operation::VenueOrder {
            action: "liquidate".to_owned(),
            subject: "borrower.position".to_owned(),
            asset: "ethereum.USDC".to_owned(),
            quantity: 1_000,
        },
    ]);
    assert_eq!(lane, Lane::Liquidation);
    assert!(
        lane.rank() < Lane::Trading.rank(),
        "the more constrained lane is served first: {} < {}",
        lane.rank(),
        Lane::Trading.rank()
    );
}

#[test]
fn a_program_that_does_nothing_with_a_lane_is_standard() {
    assert_eq!(
        lane_of_ir(vec![Operation::Nop, Operation::AtomicBegin, Operation::AtomicEnd]),
        Lane::Standard
    );
}

#[test]
fn the_lane_reached_from_source_matches_the_lane_of_its_operations() {
    // The lane is a property of what the lowered program *does*, so three intents
    // with the same shape and different bodies land in three different lanes.
    let found = lanes_of(&format!("{ONLY_MOVES}{TRADES}{CROSSES}"));
    let by_name = |name: &str| {
        found
            .iter()
            .find(|(declared, _)| declared == name)
            .map(|(_, lane)| *lane)
            .unwrap_or_else(|| panic!("'{name}' must be classified: {found:?}"))
    };
    assert_eq!(by_name("only_moves"), Lane::Settlement);
    assert_eq!(by_name("trades"), Lane::Trading);
    assert_eq!(by_name("crosses"), Lane::AtomicCrossDomain);
}

#[test]
fn a_declaration_that_does_not_lower_is_reported_rather_than_given_a_lane() {
    // "Standard" must mean "does nothing that needs a lane", not "could not be
    // read". Giving a broken declaration the standard lane would make the report
    // useless for the one thing it is for.
    let broken = "atomic_swap simple {\n    lock ethereum.USDC amount 100 from sender\n    \
                  release ethereum.USDC amount 100 to receiver\n}\n";
    let program = x3_lang_compiler::parser::parse_source(broken).expect("it parses");
    let found = lanes::classify_program(&program);
    assert_eq!(found.len(), 1, "{found:?}");
    assert!(
        found[0].1.is_err(),
        "an atomic swap between one chain does not lower, so it has no lane: {found:?}"
    );
}

#[test]
fn within_a_lane_the_order_given_is_the_order_returned() {
    // Rule one of the policy, and the one a participant could otherwise buy their
    // way out of: nothing about a program's contents moves it relative to another
    // program in its own lane.
    let work = vec![
        Queued {
            name: "first".to_owned(),
            lane: Lane::Trading,
        },
        Queued {
            name: "second".to_owned(),
            lane: Lane::Trading,
        },
        Queued {
            name: "third".to_owned(),
            lane: Lane::Trading,
        },
    ];
    assert_eq!(
        lanes::schedule(&work)
            .iter()
            .map(|item| item.name.as_str())
            .collect::<Vec<_>>(),
        vec!["first", "second", "third"],
        "arrival order survives"
    );
}

#[test]
fn across_lanes_the_fixed_sequence_decides() {
    let work = vec![
        Queued {
            name: "standard".to_owned(),
            lane: Lane::Standard,
        },
        Queued {
            name: "trading".to_owned(),
            lane: Lane::Trading,
        },
        Queued {
            name: "liquidation".to_owned(),
            lane: Lane::Liquidation,
        },
        Queued {
            name: "cross".to_owned(),
            lane: Lane::AtomicCrossDomain,
        },
        Queued {
            name: "settlement".to_owned(),
            lane: Lane::Settlement,
        },
    ];
    assert_eq!(
        lanes::schedule(&work)
            .iter()
            .map(|item| item.name.as_str())
            .collect::<Vec<_>>(),
        vec!["liquidation", "cross", "trading", "settlement", "standard"],
        "the serving order is PRIORITY, whatever order the work arrived in"
    );
}

#[test]
fn scheduling_is_deterministic_and_stable() {
    let work = vec![
        Queued {
            name: "a".to_owned(),
            lane: Lane::Standard,
        },
        Queued {
            name: "b".to_owned(),
            lane: Lane::Trading,
        },
        Queued {
            name: "c".to_owned(),
            lane: Lane::Standard,
        },
        Queued {
            name: "d".to_owned(),
            lane: Lane::Trading,
        },
    ];
    assert_eq!(
        lanes::schedule(&work),
        lanes::schedule(&work),
        "the same work orders the same way every time"
    );
    let ordered = lanes::schedule(&work);
    let standards: Vec<&str> = ordered
        .iter()
        .filter(|item| item.lane == Lane::Standard)
        .map(|item| item.name.as_str())
        .collect();
    assert_eq!(standards, vec!["a", "c"], "ties keep arrival order");
}

#[test]
fn the_policy_has_no_parameter_for_fee_stake_or_declared_priority() {
    // PHASE 30 forbids unfair ordering mechanisms. This is the structural half of
    // that: `Queued` is a name and a lane, and `schedule` takes nothing else, so
    // there is no input a participant could use to buy position. If a field is ever
    // added here, this test is where the argument for it has to be made.
    let item = Queued {
        name: "x".to_owned(),
        lane: Lane::Standard,
    };
    let Queued { name, lane } = item;
    assert_eq!((name.as_str(), lane), ("x", Lane::Standard));
}
