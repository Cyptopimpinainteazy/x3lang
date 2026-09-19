//! The profit decomposition, on the path that decides whether a trade settles.
//!
//! PHASE 5's value is only worth having if the checks read it — a decomposition
//! computed alongside the check rather than inside it is the same defect as a
//! check nobody calls. These tests are that: the floor reads a signed net, the
//! reconciliation has teeth, and a loss cannot pass a floor of zero.

use x3_lang_compiler::ir::{AssetKey, CostKind};
use x3_lang_vm::profit::LedgerCost;
use x3_lang_vm::trading::{TradingExecError, TradingVm};

fn asset(symbol: &str) -> AssetKey {
    AssetKey {
        vm_family: "evm".to_string(),
        chain: "ethereum".to_string(),
        canonical_id: format!("0x{symbol}"),
        symbol: symbol.to_string(),
        decimals: 6,
    }
}

/// A VM with a settled position: `credits`, `debits` and a cost ledger whose
/// arithmetic agrees with the recorded delta.
fn vm_with(gross: u128, principal: u128, costs: &[LedgerCost]) -> TradingVm {
    let mut vm = TradingVm::new();
    let usdc = asset("USDC");
    let cost_total: u128 = costs.iter().map(|cost| cost.amount).sum();
    vm.trading_state.credits.insert(usdc.clone(), gross);
    vm.trading_state.debits.insert(usdc.clone(), principal);
    for cost in costs {
        vm.trading_state.cost_ledger.push(x3_lang_vm::trading::CommittedCost {
            asset: usdc.clone(),
            amount: cost.amount,
            kind: cost.kind.clone(),
        });
    }
    vm.trading_state
        .net_deltas
        .insert(usdc, gross as i128 - principal as i128 - cost_total as i128);
    vm
}

fn gas(amount: u128) -> LedgerCost {
    LedgerCost {
        amount,
        kind: CostKind::Gas.as_str().to_string(),
    }
}

#[test]
fn the_check_assembles_the_decomposition_and_it_reconciles() {
    let vm = vm_with(2_000, 0, &[gas(300)]);
    let profit = vm.profit(&asset("USDC")).expect("the accounting agrees with itself");
    assert_eq!(profit.gross, 2_000);
    assert_eq!(profit.gas, 300);
    assert_eq!(profit.net, 1_700);
    assert_eq!(profit.realized(), 1_700);
}

#[test]
fn a_decomposition_that_disagrees_with_the_recorded_delta_is_an_error() {
    // `net = credits - debits - costs` is an identity of this accounting. A
    // disagreement means a movement or a cost was written in one place and not
    // the other, and taking either side would be a guess — so it is refused.
    let mut vm = vm_with(2_000, 0, &[gas(300)]);
    vm.trading_state.net_deltas.insert(asset("USDC"), 9_999);
    match vm.profit(&asset("USDC")) {
        Err(TradingExecError::ProfitReconciliationMismatch {
            assembled, recorded, ..
        }) => {
            assert_eq!(assembled, 1_700);
            assert_eq!(recorded, 9_999);
        }
        other => panic!("a mismatch must be refused, got {other:?}"),
    }
}

#[test]
fn an_unrecorded_cost_kind_is_refused_rather_than_left_out_of_the_net() {
    // A cost the profit type cannot place would silently make the net larger.
    // The ledger accepts the entry here (it goes around `accrue_cost`, which is
    // where the allowlist lives) precisely to show that the profit assembly
    // refuses it independently rather than relying on that one gate.
    let mut vm = vm_with(2_000, 0, &[]);
    vm.trading_state.cost_ledger.push(x3_lang_vm::trading::CommittedCost {
        asset: asset("USDC"),
        amount: 50,
        kind: "invented_fee".to_string(),
    });
    match vm.profit(&asset("USDC")) {
        Err(TradingExecError::UnknownCostKind(kind)) => assert_eq!(kind, "invented_fee"),
        other => panic!("an unplaceable cost must be refused, got {other:?}"),
    }
}

#[test]
fn a_loss_is_negative_and_not_zero() {
    // The accessor this replaced clamped the delta at zero, so a losing trade
    // reported "profit 0".
    let vm = vm_with(1_000, 900, &[gas(500)]);
    assert_eq!(
        vm.net_profit(&asset("USDC")).expect("the accounting agrees"),
        -400,
        "a loss is a loss"
    );
}
