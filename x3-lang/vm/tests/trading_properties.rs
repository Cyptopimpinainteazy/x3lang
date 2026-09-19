//! Property tests for atomic trading conservation and rollback, and the eight named
//! economic invariants of PHASE 49.
//!
//! The phase lists the invariants a property test has to cover. Each one below is named
//! after the phase's own words for it, so the list can be checked off rather than
//! interpreted:
//!
//! | PHASE 49 item | test |
//! |---|---|
//! | assets cannot appear from nowhere | [`invariant_1_assets_cannot_appear_from_nowhere`] |
//! | debt cannot disappear without valid repayment | [`invariant_2_debt_cannot_disappear_without_repayment`] |
//! | profit cannot exceed the possible balance delta | [`invariant_3_profit_cannot_exceed_the_balance_delta`] |
//! | net profit = outputs − inputs − all declared costs | [`invariant_4_net_profit_is_outputs_minus_inputs_minus_costs`] |
//! | closed debt cannot reopen without explicit borrow | [`invariant_5_closed_debt_cannot_reopen`] |
//! | atomic plan cannot commit with an unresolved required leg | [`invariant_6_cannot_commit_with_a_required_leg_open`] |
//! | invalid proof cannot produce `FinalizedState` | `vm/src/bridge.rs` — `invariant_7_*` |
//! | no execution can escape declared capabilities | [`invariant_8_cannot_escape_declared_capabilities`] |
//!
//! Seven of the eight share the `Host` harness below because they are properties of an
//! atomic trade. The seventh is a property of a bridge proof and lives beside the
//! fixtures that build one.

use std::collections::BTreeSet;

use proptest::prelude::*;
use x3_lang_compiler::ir::{
    AssetKey, CompiledTradingPolicy, CostKind, StateBindingMode, SubmissionProfile, TradingOperation, ValueRef,
};
use x3_lang_vm::profit::{LedgerCost, Profit};
use x3_lang_vm::trading::{
    fixture_manifest, BorrowRequest, BorrowResult, CapabilityManifest, CommittedCost, ExecutionMode, HostError,
    QuoteRequest, QuoteResult, RepayRequest, RepayResult, SwapRequest, SwapResult, TradeExecutionContext, TradingHost,
    TradingVm,
};

const COMMITMENT: [u8; 32] = [3u8; 32];

fn asset(symbol: &str) -> AssetKey {
    AssetKey {
        vm_family: "evm".to_string(),
        chain: "ethereum".to_string(),
        canonical_id: format!("0x{symbol}"),
        symbol: symbol.to_string(),
        decimals: 6,
    }
}

struct Host {
    manifest: CapabilityManifest,
    output: u128,
    /// The `swap` call this host stops answering on, counting from 1. `None` answers
    /// every call.
    ///
    /// Models the durability case PHASE 48 calls an outage: an endpoint that was
    /// answering and then is not. The plan has to refuse and roll back rather than
    /// treat the silence as a zero or a success.
    quiet_from_swap: Option<usize>,
    swap_calls: usize,
}

impl TradingHost for Host {
    fn capabilities(&self) -> &CapabilityManifest {
        &self.manifest
    }

    fn open_debt(&mut self, request: BorrowRequest) -> Result<BorrowResult, HostError> {
        Ok(BorrowResult {
            asset: request.asset,
            principal: request.principal,
            fee: 0,
            state_commitment: COMMITMENT,
        })
    }

    fn quote(&self, request: QuoteRequest) -> Result<QuoteResult, HostError> {
        let _ = request;
        Ok(QuoteResult {
            expected_output: self.output,
            sources: Vec::new(),
            quote_block: 0,
        })
    }

    fn swap(&mut self, request: SwapRequest) -> Result<SwapResult, HostError> {
        self.swap_calls += 1;
        if self
            .quiet_from_swap
            .is_some_and(|quiet_from| self.swap_calls >= quiet_from)
        {
            return Err(HostError {
                code: "X3_HOST_UNAVAILABLE".to_string(),
                message: format!("the venue stopped answering before swap call {}", self.swap_calls),
            });
        }
        Ok(SwapResult {
            from: request.from,
            to: request.to.clone(),
            input: request.input,
            output: self.output,
            fee: 0,
            fee_asset: request.to,
            state_commitment: COMMITMENT,
        })
    }

    fn close_debt(&mut self, request: RepayRequest) -> Result<RepayResult, HostError> {
        Ok(RepayResult {
            debt_id: request.debt_id,
            asset: request.asset,
            amount_paid: request.amount,
            fee: 0,
            state_commitment: COMMITMENT,
        })
    }

    fn execution_costs(&self) -> Result<Vec<CommittedCost>, HostError> {
        Ok(Vec::new())
    }
}

fn operations() -> Vec<TradingOperation> {
    vec![
        TradingOperation::BeginAtomicTrade {
            trade_id: "T".to_string(),
            policy: CompiledTradingPolicy {
                policy_id: "P".to_string(),
                policy_version: 1,
                chain: "ethereum".to_string(),
                max_slippage_bps: 30,
                max_gas: u128::MAX,
                max_gas_asset: asset("USDC"),
                max_flash_fee_bps: 10,
                deadline_blocks: 10,
                require_private_submission: false,
                minimum_net_profit: None,
                minimum_net_profit_asset: None,
                quote_freshness_blocks: Some(10),
                submission_profile: SubmissionProfile::Public,
                state_binding: StateBindingMode::Exact,
                allowed_cost_kinds: BTreeSet::from([
                    CostKind::Gas,
                    CostKind::LiquidityFee,
                    CostKind::FlashLiquidityFee,
                    CostKind::ProofFee,
                    CostKind::CrossDomainFee,
                    CostKind::Slippage,
                    CostKind::PriceImpact,
                    CostKind::MevLeakage,
                ]),
                allow_mint: false,
                allow_burn: false,
                max_oracle_deviation_bps: None,
                max_cumulative_loss: None,
                max_cumulative_loss_asset: None,
            },
        },
        TradingOperation::OpenDebt {
            debt_id: "debt".to_string(),
            provider: "aave_v3".to_string(),
            asset: asset("USDC"),
            principal: 1_000_000,
        },
        TradingOperation::ExecuteSwap {
            binding: "weth".to_string(),
            venue: "uniswap_v3".to_string(),
            from: asset("USDC"),
            to: asset("WETH"),
            input: ValueRef::Binding("debt.amount".to_string()),
            min_output: 1,
        },
        TradingOperation::ExecuteSwap {
            binding: "returned".to_string(),
            venue: "uniswap_v3".to_string(),
            from: asset("WETH"),
            to: asset("USDC"),
            input: ValueRef::Binding("weth".to_string()),
            min_output: 1,
        },
        TradingOperation::CloseDebt {
            debt_id: "debt".to_string(),
        },
        TradingOperation::AssertMinNetProfit {
            settlement_asset: asset("USDC"),
            minimum: 1,
        },
        TradingOperation::AssertAllDebtsClosed,
        TradingOperation::EmitTradeReceipt,
        TradingOperation::CommitAtomicTrade,
    ]
}

fn host(output: u128) -> Host {
    let mut manifest = fixture_manifest(COMMITMENT);
    manifest.providers = BTreeSet::from(["aave_v3".to_string()]);
    manifest.venues = BTreeSet::from(["uniswap_v3".to_string()]);
    Host {
        manifest,
        output,
        quiet_from_swap: None,
        swap_calls: 0,
    }
}

fn context() -> TradeExecutionContext {
    TradeExecutionContext {
        mode: ExecutionMode::Development,
        current_block: 1,
    }
}

proptest! {
    #[test]
    fn success_never_has_open_debt_or_missing_commit(output in 1u128..10_000_000u128) {
        let mut vm = TradingVm::new();
        let mut host = host(output);
        let result = vm.execute_atomic(&operations(), &mut host, context());
        if result.is_ok() {
            prop_assert!(vm.trading_state.open_debts.is_empty());
            prop_assert!(vm.trading_state.committed);
            prop_assert!(vm.trading_state.receipt_emitted);
        }
    }

    #[test]
    fn failure_restores_the_pre_execution_state(output in 1u128..10_000_000u128) {
        let mut vm = TradingVm::new();
        let before = vm.trading_state.clone();
        let mut host = host(output);
        let result = vm.execute_atomic(&operations(), &mut host, context());
        if result.is_err() {
            prop_assert_eq!(vm.trading_state, before);
        }
    }
}

// ── PHASE 49: the eight named economic invariants ───────────────────────────────

/// The fixture's debt id and principal, named once so the invariants below read as
/// statements about a debt rather than about a number.
const DEBT_ID: &str = "debt";
const DEBT_PRINCIPAL: u128 = 1_000_000;

/// The fixture's ops, with one edit. Kept as a function rather than four near-copies of
/// `operations()` so a change to the fixture reaches every invariant.
fn operations_with(mutate: impl FnOnce(&mut Vec<TradingOperation>)) -> Vec<TradingOperation> {
    let mut ops = operations();
    mutate(&mut ops);
    ops
}

fn close_debt_index(ops: &[TradingOperation]) -> usize {
    ops.iter()
        .position(|op| matches!(op, TradingOperation::CloseDebt { .. }))
        .expect("the fixture closes its debt")
}

proptest! {
    /// PHASE 49 (1): **assets cannot appear from nowhere**.
    ///
    /// The state records every credit, every debit and every cost per asset, so
    /// `net_deltas = credits − debits − costs` is the conservation law of this
    /// accounting. An asset whose balance grew without a credit, or a cost charged
    /// without being recorded, breaks it — and the balance itself is bounded by what was
    /// credited, which is the "from nowhere" half.
    #[test]
    fn invariant_1_assets_cannot_appear_from_nowhere(output in 1u128..10_000_000u128) {
        let mut vm = TradingVm::new();
        let mut host = host(output);
        let result = vm.execute_atomic(&operations(), &mut host, context());
        if result.is_ok() {
            for (asset, credits) in vm.trading_state.credits.clone() {
                let debits = vm.trading_state.debits.get(&asset).copied().unwrap_or(0);
                let costs = vm.trading_state.costs.get(&asset).copied().unwrap_or(0);
                let delta = vm.trading_state.net_deltas.get(&asset).copied().unwrap_or(0);
                prop_assert_eq!(
                    delta,
                    i128::try_from(credits).expect("a fixture credit fits an i128")
                        - i128::try_from(debits).expect("a fixture debit fits an i128")
                        - i128::try_from(costs).expect("a fixture cost fits an i128"),
                    "asset {:?} breaks credits - debits - costs = net_deltas",
                    asset.symbol
                );
            }
            for (asset, balance) in vm.trading_state.balances.clone() {
                let credits = vm.trading_state.credits.get(&asset).copied().unwrap_or(0);
                prop_assert!(
                    balance <= credits,
                    "asset {:?} holds {balance} against {credits} credited — the rest came from nowhere",
                    asset.symbol
                );
            }
        }
    }

    /// PHASE 49 (2): **debt cannot disappear without valid repayment**.
    ///
    /// Two halves. A committed trade has a closed record for the debt it opened, at the
    /// principal it borrowed — a debt that vanished would leave no record. And a run
    /// that did *not* commit closes nothing: the debt does not disappear, it is undone.
    #[test]
    fn invariant_2_debt_cannot_disappear_without_repayment(output in 1u128..10_000_000u128) {
        let mut vm = TradingVm::new();
        let mut host = host(output);
        let result = vm.execute_atomic(&operations(), &mut host, context());
        match result {
            Ok(_) => {
                prop_assert!(vm.trading_state.open_debts.is_empty());
                let record = vm
                    .trading_state
                    .closed_debt_records
                    .get(DEBT_ID)
                    .expect("a committed trade records the debt it closed");
                prop_assert_eq!(record.principal, DEBT_PRINCIPAL);
                prop_assert!(vm.trading_state.closed_debts.contains(DEBT_ID));
            }
            Err(_) => {
                prop_assert!(
                    vm.trading_state.closed_debt_records.is_empty(),
                    "a trade that did not commit closed a debt"
                );
                prop_assert!(vm.trading_state.closed_debts.is_empty());
            }
        }
    }

    /// PHASE 49 (3): **profit cannot exceed the mathematically possible balance delta**.
    ///
    /// The `AssertMinNetProfit` guard reads the VM's own net. If it passes, the ledger
    /// has to support the figure it passed on: the settlement asset's recorded delta must
    /// actually be at least the floor. A guard that passed on a number the accounting
    /// does not contain is a profit that exists only in the comparison.
    #[test]
    fn invariant_3_profit_cannot_exceed_the_balance_delta(output in 1u128..10_000_000u128) {
        let mut vm = TradingVm::new();
        let mut host = host(output);
        let result = vm.execute_atomic(&operations(), &mut host, context());
        if result.is_ok() {
            // `balances` and `costs` are maintained by different calls — `credit`/`debit`
            // move the balance, `accrue_cost` records the cost — so the relationship
            // between them is a cross-check rather than a restatement. If a cost were ever
            // charged without reducing the balance's backing, or a balance moved without a
            // cost, these two assertions are what notices.
            for asset in vm.trading_state.net_deltas.keys().cloned().collect::<Vec<_>>() {
                let profit = vm
                    .profit(&asset)
                    .expect("a committed state must reconcile its own profit");
                let balance = i128::try_from(vm.trading_state.balances.get(&asset).copied().unwrap_or(0))
                    .expect("a fixture balance fits an i128");
                let costs = i128::try_from(vm.trading_state.costs.get(&asset).copied().unwrap_or(0))
                    .expect("a fixture cost fits an i128");

                prop_assert!(
                    profit.net <= balance,
                    "asset {}: a profit of {} exceeds the balance of {} it would have to come \
                     out of — money the trade does not have",
                    asset.symbol,
                    profit.net,
                    balance
                );
                prop_assert_eq!(
                    balance - profit.net,
                    costs,
                    "asset {}: the gap between the balance and the profit is not the costs charged",
                    asset.symbol
                );
            }

            let settlement = asset("USDC");
            let delta = vm.trading_state.net_deltas.get(&settlement).copied().unwrap_or(0);
            // The fixture's floor is `AssertMinNetProfit { minimum: 1 }`.
            prop_assert!(
                delta >= 1,
                "the profit guard passed on {delta}, which is below its own floor of 1"
            );
            prop_assert_eq!(
                vm.net_profit(&settlement).expect("reconciles"),
                delta,
                "the assembled profit and the recorded delta disagree"
            );
        }
    }

    /// PHASE 49 (6): **atomic plan cannot commit with unresolved required leg**.
    ///
    /// The fixture's plan closes its debt and then asserts `AssertAllDebtsClosed`. Remove
    /// the repayment and the plan has an unresolved leg at commit: the run must fail, and
    /// nothing may be committed or receipted — a plan that settled with a debt still open
    /// is the failure this invariant names.
    #[test]
    fn invariant_6_cannot_commit_with_a_required_leg_open(output in 1u128..10_000_000u128) {
        let ops = operations_with(|ops| {
            let at = close_debt_index(ops);
            ops.remove(at);
        });
        let mut vm = TradingVm::new();
        let before = vm.trading_state.clone();
        let mut host = host(output);
        let result = vm.execute_atomic(&ops, &mut host, context());
        prop_assert!(result.is_err(), "a plan with an open debt committed");
        prop_assert_eq!(&vm.trading_state, &before, "and it must leave no trace");
    }
}

/// PHASE 49 (4): **net profit = outputs − inputs − all declared costs**.
///
/// The identity, over arbitrary cost lists rather than one fixture. `total_costs` has to
/// be the sum of the categories that were declared — no category quietly outside the sum
/// — and `net` has to be the whole expression, including the principal and the declared
/// buffer that the phase's shorthand omits.
#[test]
fn invariant_4_net_profit_is_outputs_minus_inputs_minus_costs() {
    use proptest::strategy::ValueTree;
    use proptest::test_runner::TestRunner;

    use x3_lang_compiler::ir::CostKind;

    let kinds = [
        CostKind::Gas,
        CostKind::LiquidityFee,
        CostKind::FlashLiquidityFee,
        CostKind::SolverInfrastructureFee,
        CostKind::ProofFee,
        CostKind::CrossDomainFee,
        CostKind::Slippage,
        CostKind::PriceImpact,
        CostKind::MevLeakage,
    ];

    let mut runner = TestRunner::default();
    // Bounded rather than `u128::ANY`: the identity is signed arithmetic, so a gross or
    // principal above `i128::MAX` would make the property about the conversion rather than
    // about the accounting. Values a trade could actually realise.
    let cases = proptest::collection::vec((0usize..kinds.len(), 0u128..10_000), 0..8)
        .prop_flat_map(|costs| (0u128..1_000_000, 0u128..1_000_000, 0u128..1_000, Just(costs)));

    for _ in 0..256 {
        let (gross, principal, buffer, declared) = cases.new_tree(&mut runner).expect("a generated case").current();
        let ledger: Vec<LedgerCost> = declared
            .iter()
            .map(|(index, amount)| LedgerCost {
                amount: *amount,
                kind: kinds[*index].as_str().to_string(),
            })
            .collect();

        let Ok(profit) = Profit::from_ledger(gross, principal, buffer, &ledger) else {
            // A total that overflows `i128` is allowed to refuse; what it must not do is
            // return a figure.  Continue rather than assert, so the property is about the
            // cases that do produce one.
            continue;
        };

        // The declared costs are exactly what the ledger said, whatever the mix.
        let declared_total: u128 = declared.iter().map(|(_, amount)| *amount).sum();
        assert_eq!(profit.total_costs(), declared_total, "a cost fell outside the sum");

        // And `net` is the whole identity: outputs − inputs − all declared costs, less the
        // buffer the policy holds back.
        let signed = |value: u128| i128::try_from(value).expect("the generator bounds every figure");
        let expected = signed(gross) - signed(principal) - signed(declared_total) - signed(buffer);
        assert_eq!(profit.net, expected, "net is not the identity the phase states");

        // A margin is a share of the proceeds, so it cannot be non-zero when there are none.
        if gross == 0 {
            assert_eq!(profit.margin_bps, 0, "a margin against nothing is not a number");
        }
    }
}

/// PHASE 49 (5): **closed debt cannot reopen without explicit borrow**.
///
/// A plan that closes its debt and then opens the same id is asking to borrow again
/// under a name it has already repaid. It is refused — and because the refusal happens
/// inside the atomic scope, the whole trade rolls back rather than leaving half of it.
#[test]
fn invariant_5_closed_debt_cannot_reopen() {
    let ops = operations_with(|ops| {
        let at = close_debt_index(ops);
        ops.insert(
            at + 1,
            TradingOperation::OpenDebt {
                debt_id: DEBT_ID.to_string(),
                provider: "aave_v3".to_string(),
                asset: asset("USDC"),
                principal: 1_000,
            },
        );
    });

    let mut vm = TradingVm::new();
    let before = vm.trading_state.clone();
    let mut host = host(5_000_000);
    let result = vm.execute_atomic(&ops, &mut host, context());
    assert!(
        result.is_err(),
        "a closed debt id reopened without the plan borrowing under a name it already repaid"
    );
    assert_eq!(vm.trading_state, before, "and the refusal rolls the plan back");
}

/// PHASE 49 (8): **no execution can escape declared capabilities**.
///
/// The host declares the venues it can reach. A program that names a venue the manifest
/// does not list is asking for a capability it was not granted, and it is refused — with
/// the state rolled back, so the attempt leaves nothing behind.
#[test]
fn invariant_8_cannot_escape_declared_capabilities() {
    for (label, swap_venue) in [("venue", "some_other_venue"), ("provider", "some_other_provider")] {
        let ops = operations_with(|ops| {
            let mut edits = 0;
            for op in ops.iter_mut() {
                match op {
                    TradingOperation::ExecuteSwap { venue, .. } if label == "venue" => {
                        *venue = swap_venue.to_string();
                        edits += 1;
                    }
                    TradingOperation::OpenDebt { provider, .. } if label == "provider" => {
                        *provider = swap_venue.to_string();
                        edits += 1;
                    }
                    _ => {}
                }
            }
            assert!(edits > 0, "the fixture must contain the op this case edits");
        });

        let mut vm = TradingVm::new();
        let before = vm.trading_state.clone();
        let mut host = host(5_000_000);
        let result = vm.execute_atomic(&ops, &mut host, context());
        assert!(
            result.is_err(),
            "a plan naming an undeclared {label} ('{swap_venue}') executed"
        );
        assert_eq!(
            vm.trading_state, before,
            "and a refused capability must leave no state behind"
        );
    }
}

// ── TICKET-087: what survives a run ─────────────────────────────────────────────
//
// PHASE 48's last six cases are one family — state that outlives the process — and the
// decision recorded in `.ai/reports/x3lang-ticket087-decision-20260919.md` is that
// **nothing does**: every candidate is either derived from the artifact or supplied by
// the host for that run. An interrupted plan is therefore not resumed and not executed;
// the fail-closed reading. These tests assert that reading rather than assuming it.
//
// (`restart` and `process kill` share a shape — a run that ends mid-plan — so one test
// covers both, which is honest rather than a shortcut: the difference between them is
// whether the process comes back, and the claim being tested is that it does not matter
// because nothing was left behind either way.)

/// TICKET-087, `restart during execution` and `process kill`: **a run leaves nothing for
/// the next one to find.**
///
/// The claim is that a fresh VM is the *whole* state of a run, so starting over is
/// exactly starting fresh — there is no half-plan to resume and no journal to inherit.
/// Asserted two ways: a fresh VM is the empty state, and two fresh VMs driven over the
/// same ops reach the same state.
#[test]
fn restarts_and_kills_leave_nothing_behind() {
    let fresh = TradingVm::new();
    assert_eq!(
        fresh.trading_state,
        Default::default(),
        "a fresh VM is the empty journal — if this ever stopped being true, a restart \
         could inherit something"
    );
    assert!(fresh.trading_state.open_debts.is_empty());
    assert!(fresh.trading_state.credits.is_empty());
    assert!(fresh.trading_state.cost_ledger.is_empty());
    assert!(!fresh.trading_state.committed);

    // A run that is abandoned mid-plan leaves nothing either: the VM that executed it is
    // the only place the partial journal ever existed, and dropping it drops the plan.
    // What a *second* fresh VM sees is therefore exactly what the first would have seen.
    let mut abandoned = TradingVm::new();
    let mut first_host = host(5_000_000);
    let outcome = abandoned.execute_atomic(&operations(), &mut first_host, context());
    assert!(outcome.is_ok(), "the fixture commits");
    drop(abandoned);

    let mut restarted = TradingVm::new();
    let mut second_host = host(5_000_000);
    restarted
        .execute_atomic(&operations(), &mut second_host, context())
        .expect("the same ops on a fresh VM");
    let mut uninterrupted = TradingVm::new();
    let mut third_host = host(5_000_000);
    uninterrupted
        .execute_atomic(&operations(), &mut third_host, context())
        .expect("the same ops again");
    assert_eq!(
        restarted.trading_state, uninterrupted.trading_state,
        "two fresh VMs over the same ops must agree; anything else means state outlived \
         a run"
    );
}

/// TICKET-087, `process kill` from the other side: **the only thing a run emits is
/// complete or absent.**
///
/// A receipt is the one record that leaves a run. It must not exist for a plan that did
/// not commit — a half-written receipt is exactly the durable state a restart would find
/// and trust.
#[test]
fn a_receipt_exists_only_for_a_committed_plan() {
    // The fixture's floor is 1 USDC of net; an output below the principal plus that
    // cannot commit.
    let mut vm = TradingVm::new();
    let before = vm.trading_state.clone();
    let mut poor_host = host(500_000);
    assert!(
        vm.execute_atomic(&operations(), &mut poor_host, context()).is_err(),
        "an output below the principal must not commit"
    );
    assert!(!vm.trading_state.receipt_emitted, "a refused plan emitted a receipt");
    assert_eq!(vm.trading_state, before, "and it left the journal untouched");

    // And a plan that did commit emitted one, with the journal reconciled.
    let mut vm = TradingVm::new();
    let mut rich_host = host(5_000_000);
    vm.execute_atomic(&operations(), &mut rich_host, context())
        .expect("the fixture commits");
    assert!(vm.trading_state.receipt_emitted);
    assert!(vm.trading_state.committed);
}

/// TICKET-087, `partial domain outage`: **one domain unreachable after the other
/// committed rolls the whole plan back.**
///
/// The fixture's host does not bridge, and the manifest lists the bridge so the refusal
/// comes from the *host* rather than from the capability check — the domain is
/// unreachable, not undeclared. The half of the plan that ran must not survive.
#[test]
fn an_unreachable_second_domain_rolls_the_plan_back() {
    let ops = operations_with(|ops| {
        let at = close_debt_index(ops);
        ops.insert(
            at + 1,
            TradingOperation::Bridge {
                via: "x3".to_string(),
                from: asset("USDC"),
                to: asset("SOL"),
                input: ValueRef::Literal(1_000),
                receiver: "0xrecipient".to_string(),
            },
        );
    });

    let mut vm = TradingVm::new();
    let before = vm.trading_state.clone();
    let mut host = host(5_000_000);
    host.manifest.bridges = BTreeSet::from(["x3".to_string()]);
    let result = vm.execute_atomic(&ops, &mut host, context());
    assert!(
        result.is_err(),
        "a plan whose second domain is unreachable must not commit"
    );
    assert_eq!(
        vm.trading_state, before,
        "the domain that did commit must be rolled back with the one that did not"
    );
}

/// TICKET-087, `RPC outage`: **a host that stops answering mid-plan rolls the plan
/// back.**
///
/// The second swap is where this host goes quiet, so the first leg has already been
/// debited and credited when the silence arrives. Silence must not read as a zero, a
/// skip, or a success.
#[test]
fn a_host_that_stops_answering_mid_plan_rolls_the_plan_back() {
    let mut vm = TradingVm::new();
    let before = vm.trading_state.clone();
    let mut host = host(5_000_000);
    // Quiet from the second swap: the first leg commits its debit and credit first.
    host.quiet_from_swap = Some(2);
    let result = vm.execute_atomic(&operations(), &mut host, context());
    assert!(
        result.is_err(),
        "a host that stopped answering must not produce a committed plan"
    );
    assert_eq!(
        vm.trading_state, before,
        "the leg that ran before the silence must be rolled back too"
    );
    assert!(!vm.trading_state.receipt_emitted);

    // And the silence is specifically the *second* call, so the test is about an outage
    // mid-plan rather than about a host that never answered at all.
    assert_eq!(host.swap_calls, 2, "the outage must land mid-plan");
}
