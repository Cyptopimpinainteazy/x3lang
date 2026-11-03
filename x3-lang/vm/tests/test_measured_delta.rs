//! A hedge's delta bound is judged against what the **venue** reports (TICKET-068).
//!
//! A hedge asks each leg's venue for an order, so the delta is the quantity that call is
//! about — and it is its own unit rather than a reuse of the profit or the slippage, because
//! a venue that answered a hedge with a profit would be answering a different question. The
//! two properties worth pinning are that the delta arrives at all, and that nothing *else*
//! arrives as a delta.

use x3_lang_vm::bridge::{BridgeAdapter, DryRunBridge};
use x3_lang_vm::spec::opcodes;

#[test]
fn a_stated_delta_is_reported_by_the_venue_order_call() {
    let bridge = DryRunBridge::with_outcome(None, None, Some(250));
    let reply = bridge.venue_order(b"order").expect("a dry run answers");
    assert_eq!(
        opcodes::read_measured_replies(&reply),
        vec![(opcodes::MEASURED_UNIT_DELTA_BPS, 250)],
        "the delta must arrive in the reply to the call the hedge made, in its own unit"
    );
}

/// A venue that measured nothing reports nothing, and the guard then refuses. A dry run that
/// echoed a number to satisfy the bound would be inventing the measurement the bound exists
/// to require.
#[test]
fn an_unstated_delta_is_not_reported_at_all() {
    let bridge = DryRunBridge::default();
    let reply = bridge.venue_order(b"order").expect("a dry run answers");
    assert!(
        opcodes::read_measured_replies(&reply).is_empty(),
        "nothing stated is nothing measured, not zero: {reply:?}"
    );
}

/// The separation that makes the unit code load-bearing: a quantity is only ever read against
/// the guard that asked for it.
///
/// A stated profit reaches the venue order as **a profit**, because a liquidation's floor
/// follows the orders that seized the collateral (TICKET-100). What must never happen is that
/// it arrives as a delta — a guard about what the hedge left open, compared against a number
/// about what the trade earned, is the units mismatch the measured modes exist to prevent.
#[test]
fn a_stated_profit_reaches_a_venue_order_as_a_profit_and_never_as_a_delta() {
    let bridge = DryRunBridge::with_measurement(500, 3);
    let reply = bridge.venue_order(b"order").expect("a dry run answers");
    let carried = opcodes::read_measured_replies(&reply);

    assert!(
        carried
            .iter()
            .all(|(unit, _)| *unit != opcodes::MEASURED_UNIT_DELTA_BPS),
        "a stated profit must never arrive as a delta: {carried:?}"
    );
    assert_eq!(
        carried,
        vec![(opcodes::MEASURED_UNIT_PROFIT_BPS, 500)],
        "it arrives as the profit, which is the quantity a liquidation's floor is about"
    );
    assert!(
        carried
            .iter()
            .all(|(unit, _)| *unit != opcodes::MEASURED_UNIT_SLIPPAGE_BPS),
        "and a venue order does not answer a slippage guard — a liquidation states its ceiling \
         as a program-written constraint, which the compiler checks: {carried:?}"
    );

    // A plan's call reports both, and the delta is still the only thing that answers a delta.
    let swap_reply = bridge.multi_hop_swap(b"path", 1).expect("a dry run answers");
    assert_eq!(
        opcodes::read_measured_replies(&swap_reply),
        vec![
            (opcodes::MEASURED_UNIT_PROFIT_BPS, 500),
            (opcodes::MEASURED_UNIT_SLIPPAGE_BPS, 3),
        ],
        "the profit and the slippage belong to the call a plan makes"
    );

    let hedging = DryRunBridge::with_outcome(None, None, Some(7));
    assert_eq!(
        opcodes::read_measured_replies(&hedging.venue_order(b"order").expect("a dry run answers")),
        vec![(opcodes::MEASURED_UNIT_DELTA_BPS, 7)],
        "and a delta reaches only the call a hedge makes"
    );
}

/// The unit code survives the round trip, and zero is the profit — which is what an artifact
/// written before the field existed carries.
#[test]
fn the_unit_code_round_trips_and_zero_still_means_the_profit() {
    for (mode, code) in [
        (
            opcodes::REQUIRE_COMPARE_MEASURED_PROFIT,
            opcodes::MEASURED_UNIT_CODE_PROFIT_BPS,
        ),
        (
            opcodes::REQUIRE_COMPARE_MEASURED_PROFIT,
            opcodes::MEASURED_UNIT_CODE_DELTA_BPS,
        ),
    ] {
        let flags = opcodes::require_flags_measured(mode, opcodes::GUARD_OP_LE, code);
        assert_eq!(opcodes::require_measured_unit_code(flags), code);
        assert_eq!(opcodes::require_comparison(flags), mode);
        assert_eq!(opcodes::require_guard_operator(flags), opcodes::GUARD_OP_LE);
    }

    let before_the_field = opcodes::require_flags(opcodes::REQUIRE_COMPARE_MEASURED_PROFIT, opcodes::GUARD_OP_GE);
    assert_eq!(
        opcodes::require_measured_unit_code(before_the_field),
        opcodes::MEASURED_UNIT_CODE_PROFIT_BPS,
        "an artifact emitted before the field means the profit, not nothing"
    );
    assert!(opcodes::is_known_measured_unit_code(
        opcodes::MEASURED_UNIT_CODE_PROFIT_BPS
    ));
    assert!(opcodes::is_known_measured_unit_code(
        opcodes::MEASURED_UNIT_CODE_DELTA_BPS
    ));
    assert!(!opcodes::is_known_measured_unit_code(5));
}

/// A measurement the language does not define is not a measurement the VM accepts: an unknown
/// unit byte reaches no field, so a guard cannot pass on it.
#[test]
fn an_unknown_unit_is_not_a_measurement() {
    let reply = opcodes::measured_reply(0x7F, 1_000);
    assert_eq!(
        opcodes::read_measured_replies(&reply),
        vec![(0x7F, 1_000)],
        "the reply is read as the record it is — the VM is what decides no field takes it"
    );
}
