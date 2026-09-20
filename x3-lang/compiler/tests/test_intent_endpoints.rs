//! Each endpoint of an intent names the account on **its own** chain (TICKET-072).
//!
//! A lock's `from` was reported as the *payee*, on the reading that the intent parser put the
//! `from` endpoint's `receiver` into it while the atomic-swap path put `"sender"` there. The
//! two are the same quantity: the account on the from-side chain whose funds are held. The
//! destination side's account is the `to` endpoint's own receiver, which is what a release
//! carries. These tests pin that, so the next reader has the evidence rather than the
//! reading — the consuming side agrees: `typechecker.py` validates `from.receiver` against
//! `from.chain`, and a host debits the field a lock names.

use x3_lang_compiler::ir::Operation;
use x3_lang_compiler::semantic::CompilationMode;

/// An intent whose two endpoints name the same chain but **different** accounts, which is
/// how the corpus already writes them (`arb_scope.x3`, `arb_solana_eth.x3`).
fn intent(receiver_from: &str, receiver_to: &str) -> String {
    format!(
        r#"intent probe {{
    from ethereum.USDC amount 1_000 receiver {receiver_from}
    to ethereum.WETH receiver {receiver_to}
    route {{
        swap uniswap ethereum.USDC -> ethereum.WETH amount 1_000 min_output 1
    }}
    require slippage <= 50
    on_fail refund ethereum.USDC to sender
}}
"#
    )
}

fn lowered(source: &str) -> Vec<Operation> {
    let (_, ir, outcome) =
        x3_lang_compiler::check_source_diagnostics_with_mode(source, CompilationMode::Dev).expect("it must lower");
    assert!(outcome.errors.is_empty(), "it must check: {:?}", outcome.errors);
    ir.operations
}

fn lock_and_release(ops: &[Operation]) -> (String, Vec<String>) {
    let mut lock_from = None;
    let mut releases = Vec::new();
    for op in ops {
        match op {
            Operation::Lock { from, .. } => lock_from = Some(from.clone()),
            Operation::Release { to, .. } => releases.push(to.clone()),
            _ => {}
        }
    }
    (lock_from.expect("the from endpoint lowers to a lock"), releases)
}

/// The two endpoints' receivers are different values, and each lands where its own chain's
/// account belongs: the from-side account on the lock (the payer), the to-side on a release
/// (the payee). A test that used one address for both could not tell the two apart.
#[test]
fn the_two_endpoints_receivers_land_on_their_own_sides() {
    let (lock_from, releases) = lock_and_release(&lowered(&intent("0xA1", "0xA2")));

    assert_eq!(
        lock_from, "0xA1",
        "the lock names the from endpoint's account — the payer whose funds are escrowed, \
         which is what a host debits"
    );
    assert!(
        releases.iter().any(|to| to == "0xA2"),
        "and the to endpoint's account is the payee a release names: {releases:?}"
    );
    assert_ne!(
        lock_from, "0xA2",
        "if the lock carried the destination's account, a host would debit the wrong party — \
         which is what TICKET-072 reported and this pins as not happening"
    );
}

/// An endpoint that states no receiver falls back to the keyword `"sender"`, which is the
/// same quantity spelled as a role rather than an address. Both spellings reach the same
/// field, which is why the field's doc names the quantity and not the spelling.
#[test]
fn an_endpoint_without_a_receiver_names_the_sender() {
    let source = "intent probe {\n    from ethereum.USDC amount 1_000\n    to ethereum.WETH\n    \
                  route {\n        swap uniswap ethereum.USDC -> ethereum.WETH amount 1_000 \
                  min_output 1\n    }\n    require slippage <= 50\n    on_fail refund \
                  ethereum.USDC to sender\n}\n";
    let (lock_from, releases) = lock_and_release(&lowered(source));
    assert_eq!(lock_from, "sender", "the payer defaults to the role, not to an address");
    assert!(
        !releases.is_empty(),
        "and the destination side still lowers to a release"
    );
}
