//! A second, independent statement of the trading-sequence rules, and a differential test that
//! holds `verify_trading_program` to it (X3-LANG-007).
//!
//! `verify_trading_sequences` is one hand-written state machine: it walks the list once, carrying
//! flags (`began`, `bridged`, `receipt_seen`, ...) and maps (`open_debts`, `bindings`). A mistake in
//! when a flag is set, or which flag a rule reads, is invisible to tests that only feed it the cases
//! its author thought of. This file states the same rules the other way round — each one a
//! predicate over the *whole* sequence, phrased with positions ("no swap at any index after a
//! bridge", "a debt's close has an open before it") — and a property test generates sequences and
//! requires the two to agree on every one.
//!
//! Every generated operation is field-valid (non-empty ids, positive amounts, a well-formed policy),
//! so the per-operation field checks never fire and a disagreement is always about sequencing.

use std::collections::BTreeSet;

use proptest::prelude::*;
use x3_lang_compiler::ir::{
    AssetKey, CompiledTradingPolicy, CostKind, InvariantKind, StateBindingMode, SubmissionProfile,
    TradingOperation as Op, ValueRef,
};
use x3_lang_compiler::verify::verify_trading_program;

// ─── Field-valid operations ─────────────────────────────────────────────────────────────────────

fn asset(symbol: &str) -> AssetKey {
    AssetKey {
        vm_family: "evm".to_string(),
        chain: "ethereum".to_string(),
        canonical_id: format!("0x{symbol}"),
        symbol: symbol.to_string(),
        decimals: 6,
    }
}

fn begin() -> Op {
    Op::BeginAtomicTrade {
        trade_id: "T".to_string(),
        policy: CompiledTradingPolicy {
            policy_id: "P".to_string(),
            policy_version: 1,
            chain: "ethereum".to_string(),
            max_slippage_bps: 30,
            max_gas: 1_000_000,
            max_gas_asset: asset("USDC"),
            max_flash_fee_bps: 10,
            deadline_blocks: 10,
            require_private_submission: false,
            minimum_net_profit: None,
            minimum_net_profit_asset: None,
            quote_freshness_blocks: Some(10),
            submission_profile: SubmissionProfile::Public,
            state_binding: StateBindingMode::Exact,
            allowed_cost_kinds: BTreeSet::from([CostKind::Gas]),
            allow_mint: false,
            allow_burn: false,
            max_oracle_deviation_bps: None,
            max_cumulative_loss: None,
            max_cumulative_loss_asset: None,
            max_price_impact_bps: None,
            max_mev_leakage_bps: None,
        },
    }
}

const ASSETS: [&str; 2] = ["USDC", "WETH"];
const DEBTS: [&str; 2] = ["d1", "d2"];
/// `x` and `y` are the names a valid trade uses; `z` is free, so an inserted swap can create a
/// binding without colliding and reach the rules after the binding check.
const BINDINGS: [&str; 3] = ["x", "y", "z"];

/// An input: a literal, a binding a swap may have created, or a debt's amount.
fn input(kind: u8, index: usize) -> ValueRef {
    match kind % 3 {
        0 => ValueRef::Literal(1_000),
        1 => ValueRef::Binding(BINDINGS[index % 3].to_string()),
        _ => ValueRef::Binding(format!("{}.amount", DEBTS[index % 2])),
    }
}

fn op_strategy() -> impl Strategy<Value = Op> {
    let asset_ix = 0usize..2;
    prop_oneof![
        2 => Just(begin()),
        3 => (0usize..2, asset_ix.clone()).prop_map(|(d, a)| Op::OpenDebt {
            debt_id: DEBTS[d].to_string(),
            provider: "aave_v3".to_string(),
            asset: asset(ASSETS[a]),
            principal: 1_000,
        }),
        4 => (0usize..3, asset_ix.clone(), asset_ix.clone(), 0u8..3, 0usize..3).prop_map(
            |(b, from, to, kind, ix)| Op::ExecuteSwap {
                binding: BINDINGS[b].to_string(),
                venue: "uniswap_v3".to_string(),
                from: asset(ASSETS[from]),
                to: asset(ASSETS[to]),
                input: input(kind, ix),
                min_output: 1,
            }
        ),
        3 => (0usize..2).prop_map(|d| Op::CloseDebt { debt_id: DEBTS[d].to_string() }),
        1 => (asset_ix.clone(), asset_ix, 0u8..3, 0usize..2).prop_map(|(from, to, kind, ix)| Op::Bridge {
            via: "wormhole".to_string(),
            from: asset(ASSETS[from]),
            to: asset(ASSETS[to]),
            input: input(kind, ix),
            receiver: "0xreceiver".to_string(),
        }),
        2 => Just(Op::AssertMinNetProfit { settlement_asset: asset("USDC"), minimum: 1 }),
        2 => Just(Op::AssertAllDebtsClosed),
        1 => Just(Op::AssertInvariant { kind: InvariantKind::Solvent }),
        2 => Just(Op::EmitTradeReceipt),
        2 => Just(Op::CommitAtomicTrade),
        1 => Just(Op::AbortAtomicTrade),
    ]
}

/// Sequences shaped like a trade often enough that both verdicts are exercised: a `Begin` first and
/// a terminal last most of the time, with arbitrary operations between.
fn sequence_strategy() -> impl Strategy<Value = Vec<Op>> {
    (
        any::<u8>(),
        proptest::collection::vec(op_strategy(), 0..10),
        any::<u8>(),
    )
        .prop_map(|(head, mut body, tail)| {
            if head % 4 != 0 {
                body.insert(0, begin());
            }
            match tail % 5 {
                0 | 1 => body.push(Op::CommitAtomicTrade),
                2 => body.push(Op::AbortAtomicTrade),
                _ => {}
            }
            body
        })
}

/// A trade that satisfies every rule, built in the order the rules demand, so the accepting side is
/// exercised with real variety and not only by chance.
fn valid_trade_strategy() -> impl Strategy<Value = Vec<Op>> {
    (0usize..3, any::<bool>(), any::<bool>(), any::<bool>(), any::<bool>()).prop_map(
        |(debts, bridge, invariant, abort, invariant_after_guards)| {
            let mut ops = vec![begin()];
            for d in 0..debts.min(2) {
                ops.push(Op::OpenDebt {
                    debt_id: DEBTS[d].to_string(),
                    provider: "aave_v3".to_string(),
                    asset: asset("USDC"),
                    principal: 1_000,
                });
            }
            let first_input = if debts > 0 {
                ValueRef::Binding("d1.amount".to_string())
            } else {
                ValueRef::Literal(1_000)
            };
            ops.push(Op::ExecuteSwap {
                binding: "x".to_string(),
                venue: "uniswap_v3".to_string(),
                from: asset("USDC"),
                to: asset("WETH"),
                input: first_input,
                min_output: 1,
            });
            ops.push(Op::ExecuteSwap {
                binding: "y".to_string(),
                venue: "uniswap_v3".to_string(),
                from: asset("WETH"),
                to: asset("USDC"),
                input: ValueRef::Binding("x".to_string()),
                min_output: 1,
            });
            for d in 0..debts.min(2) {
                ops.push(Op::CloseDebt {
                    debt_id: DEBTS[d].to_string(),
                });
            }
            if bridge {
                ops.push(Op::Bridge {
                    via: "wormhole".to_string(),
                    from: asset("USDC"),
                    to: asset("WETH"),
                    input: ValueRef::Binding("y".to_string()),
                    receiver: "0xreceiver".to_string(),
                });
            }
            if invariant && !invariant_after_guards {
                ops.push(Op::AssertInvariant {
                    kind: InvariantKind::Solvent,
                });
            }
            ops.push(Op::AssertMinNetProfit {
                settlement_asset: asset("USDC"),
                minimum: 1,
            });
            ops.push(Op::AssertAllDebtsClosed);
            if invariant && invariant_after_guards {
                ops.push(Op::AssertInvariant {
                    kind: InvariantKind::Solvent,
                });
            }
            ops.push(Op::EmitTradeReceipt);
            ops.push(if abort {
                Op::AbortAtomicTrade
            } else {
                Op::CommitAtomicTrade
            });
            ops
        },
    )
}

/// One small edit to a sequence.
#[derive(Clone, Debug)]
enum Edit {
    Insert(usize, Op),
    Drop(usize),
    Duplicate(usize),
    Move(usize, usize),
}

fn edit_strategy() -> impl Strategy<Value = Edit> {
    prop_oneof![
        (0usize..32, op_strategy()).prop_map(|(at, op)| Edit::Insert(at, op)),
        (0usize..32).prop_map(Edit::Drop),
        (0usize..32).prop_map(Edit::Duplicate),
        (0usize..32, 0usize..32).prop_map(|(a, b)| Edit::Move(a, b)),
    ]
}

fn apply(ops: &mut Vec<Op>, edit: &Edit) {
    let n = ops.len();
    match edit {
        Edit::Insert(at, op) => ops.insert(at % (n + 1), op.clone()),
        Edit::Drop(at) if n > 0 => {
            ops.remove(at % n);
        }
        Edit::Duplicate(at) if n > 0 => {
            let op = ops[at % n].clone();
            ops.insert(at % n, op);
        }
        Edit::Move(from, to) if n > 0 => {
            let op = ops.remove(from % n);
            let len = ops.len();
            ops.insert(to % (len + 1), op);
        }
        _ => {}
    }
}

/// A valid trade with one to three small edits: the sequences that sit right at a rule's boundary
/// (one guard too many, one operation on the wrong side of another), which a random sequence almost
/// never reaches because it is already refused for some other reason.
fn near_valid_strategy() -> impl Strategy<Value = Vec<Op>> {
    (valid_trade_strategy(), proptest::collection::vec(edit_strategy(), 1..4)).prop_map(|(mut ops, edits)| {
        for edit in &edits {
            apply(&mut ops, edit);
        }
        ops
    })
}

// ─── The reference: every rule as a predicate over the whole sequence ────────────────────────────

fn is_terminal(op: &Op) -> bool {
    matches!(op, Op::CommitAtomicTrade | Op::AbortAtomicTrade)
}

/// Indices strictly before `i` whose operation matches `pred`.
fn any_before(ops: &[Op], i: usize, pred: impl Fn(&Op) -> bool) -> bool {
    ops[..i].iter().any(pred)
}

fn count(ops: &[Op], pred: impl Fn(&Op) -> bool) -> usize {
    ops.iter().filter(|op| pred(op)).count()
}

/// Is debt `id` open immediately before index `i`: opened earlier and not closed since.
fn debt_open_before(ops: &[Op], i: usize, id: &str) -> Option<AssetKey> {
    let opened = ops[..i]
        .iter()
        .position(|op| matches!(op, Op::OpenDebt { debt_id, .. } if debt_id == id))?;
    let closed = ops[opened + 1..i]
        .iter()
        .any(|op| matches!(op, Op::CloseDebt { debt_id } if debt_id == id));
    if closed {
        return None;
    }
    match &ops[opened] {
        Op::OpenDebt { asset, .. } => Some(asset.clone()),
        _ => None,
    }
}

/// The asset binding `name` holds immediately before index `i`: the `to` of the first swap before
/// `i` that created it.
fn binding_before(ops: &[Op], i: usize, name: &str) -> Option<AssetKey> {
    ops[..i].iter().find_map(|op| match op {
        Op::ExecuteSwap { binding, to, .. } if binding == name => Some(to.clone()),
        _ => None,
    })
}

/// An input spent as `from` at index `i` is available and is that asset.
fn input_ok(ops: &[Op], i: usize, input: &ValueRef, from: &AssetKey) -> bool {
    match input {
        ValueRef::Literal(_) => true,
        ValueRef::Binding(name) => match name.split_once('.') {
            Some((debt, field)) => field == "amount" && debt_open_before(ops, i, debt).as_ref() == Some(from),
            None => binding_before(ops, i, name).as_ref() == Some(from),
        },
    }
}

/// Every debt opened before `i` has a close before `i`.
fn no_open_debt_before(ops: &[Op], i: usize) -> bool {
    DEBTS.iter().all(|id| debt_open_before(ops, i, id).is_none())
}

/// Whether the rules accept `ops`. Stated independently of `verify_trading_sequences`: no running
/// state, each rule a question about positions in the sequence.
fn reference_accepts(ops: &[Op]) -> bool {
    let n = ops.len();
    if n == 0 {
        return true; // no trading operations: nothing for the trading rules to say
    }

    // Shape: exactly one `Begin`, first; exactly one terminal, last.
    if !matches!(ops[0], Op::BeginAtomicTrade { .. }) || count(ops, |op| matches!(op, Op::BeginAtomicTrade { .. })) != 1
    {
        return false;
    }
    if !is_terminal(&ops[n - 1]) || count(ops, is_terminal) != 1 {
        return false;
    }

    let bridged_before = |i: usize| any_before(ops, i, |op| matches!(op, Op::Bridge { .. }));
    let receipt_before = |i: usize| any_before(ops, i, |op| matches!(op, Op::EmitTradeReceipt));
    let profit_before = |i: usize| any_before(ops, i, |op| matches!(op, Op::AssertMinNetProfit { .. }));
    let all_debts_before = |i: usize| any_before(ops, i, |op| matches!(op, Op::AssertAllDebtsClosed));
    let final_guards_before = |i: usize| receipt_before(i) || all_debts_before(i);

    // At most one of each guard and of the receipt, and of each invariant kind.
    for single in [
        count(ops, |op| matches!(op, Op::Bridge { .. })),
        count(ops, |op| matches!(op, Op::AssertMinNetProfit { .. })),
        count(ops, |op| matches!(op, Op::AssertAllDebtsClosed)),
        count(ops, |op| matches!(op, Op::EmitTradeReceipt)),
        count(ops, |op| {
            matches!(
                op,
                Op::AssertInvariant {
                    kind: InvariantKind::Solvent
                }
            )
        }),
    ] {
        if single > 1 {
            return false;
        }
    }

    for (i, op) in ops.iter().enumerate() {
        let ok = match op {
            Op::BeginAtomicTrade { .. } => true,
            Op::OpenDebt { debt_id, .. } => {
                // Never opened before (open or already closed), and not on the far side of a bridge.
                !any_before(ops, i, |o| matches!(o, Op::OpenDebt { debt_id: d, .. } if d == debt_id))
                    && !bridged_before(i)
            }
            Op::CloseDebt { debt_id } => debt_open_before(ops, i, debt_id).is_some() && !bridged_before(i),
            Op::ExecuteSwap {
                binding, from, input, ..
            } => {
                !bridged_before(i)
                    && !final_guards_before(i)
                    && input_ok(ops, i, input, from)
                    && binding_before(ops, i, binding).is_none()
            }
            Op::Bridge { from, to, input, .. } => {
                from != to && !final_guards_before(i) && input_ok(ops, i, input, from)
            }
            Op::AssertMinNetProfit { .. } => !final_guards_before(i),
            Op::AssertAllDebtsClosed => profit_before(i) && no_open_debt_before(ops, i),
            Op::AssertInvariant { .. } => !receipt_before(i),
            Op::EmitTradeReceipt => profit_before(i) && all_debts_before(i),
            Op::CommitAtomicTrade => {
                no_open_debt_before(ops, i) && profit_before(i) && all_debts_before(i) && receipt_before(i)
            }
            Op::AbortAtomicTrade => true,
        };
        if !ok {
            return false;
        }
    }
    true
}

// ─── The comparison ─────────────────────────────────────────────────────────────────────────────

fn verifier_accepts(ops: &[Op]) -> bool {
    verify_trading_program(ops).is_ok()
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(4096))]

    /// The two statements of the rules agree on every generated sequence.
    #[test]
    fn the_verifier_and_the_reference_agree_on_arbitrary_sequences(ops in sequence_strategy()) {
        let verifier = verifier_accepts(&ops);
        let reference = reference_accepts(&ops);
        prop_assert_eq!(
            verifier, reference,
            "verifier {} / reference {} on {:#?}\nverifier said: {:?}",
            verifier, reference, ops, verify_trading_program(&ops).err()
        );
    }

    /// And on valid trades with a few edits: the boundary cases.
    #[test]
    fn the_verifier_and_the_reference_agree_next_to_every_valid_trade(ops in near_valid_strategy()) {
        let verifier = verifier_accepts(&ops);
        let reference = reference_accepts(&ops);
        prop_assert_eq!(
            verifier, reference,
            "verifier {} / reference {} on {:#?}\nverifier said: {:?}",
            verifier, reference, ops, verify_trading_program(&ops).err()
        );
    }

    /// And on trades built to satisfy every rule — the accepting side, with variety.
    #[test]
    fn both_accept_every_trade_built_to_the_rules(ops in valid_trade_strategy()) {
        prop_assert!(reference_accepts(&ops), "the reference refuses a valid trade: {ops:#?}");
        prop_assert!(
            verifier_accepts(&ops),
            "the verifier refuses a valid trade: {ops:#?}\n{:?}",
            verify_trading_program(&ops).err()
        );
    }
}

/// The comparison is not vacuous: the random generator produces both verdicts in quantity.
#[test]
fn the_generator_reaches_both_verdicts() {
    use proptest::strategy::ValueTree;
    use proptest::test_runner::{Config, TestRunner};

    let mut runner = TestRunner::new(Config::default());
    let strategy = sequence_strategy();
    let (mut accepted, mut refused) = (0, 0);
    for _ in 0..4096 {
        let ops = strategy.new_tree(&mut runner).unwrap().current();
        if reference_accepts(&ops) {
            accepted += 1;
        } else {
            refused += 1;
        }
    }
    assert!(refused > 1000, "refused {refused}");
    assert!(accepted > 0, "the random generator never produced an accepted sequence");
}
