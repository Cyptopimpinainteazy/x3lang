//! How a host reports a measurement, and how the VM reads it.
//!
//! The wire form is the part that decides whether an economic floor can be enforced at
//! all: a guard compares a measurement in basis points, so the reply has to say that it
//! *is* a measurement and which unit it is in. Everything here is about the fail-closed
//! direction — an opaque reply, a truncated record and an unknown unit all have to leave
//! a guard with nothing to compare, rather than with bytes that mean something else.

use x3_lang_vm::spec::opcodes::{
    measured_reply, read_measured_replies, read_measured_reply, CAPABILITY_REPLY_MEASURED_TAG,
    MEASURED_UNIT_PROFIT_BPS, MEASURED_UNIT_SLIPPAGE_BPS,
};

#[test]
fn a_reply_round_trips_one_measurement() {
    let reply = measured_reply(MEASURED_UNIT_PROFIT_BPS, 137);
    assert_eq!(read_measured_reply(&reply), Some((MEASURED_UNIT_PROFIT_BPS, 137)));
}

#[test]
fn one_reply_can_answer_both_questions_a_plan_asks() {
    // A trade's profit floor and its slippage ceiling are about the same call, so a host
    // answers once: the reader takes a *sequence*, in order.
    let mut reply = measured_reply(MEASURED_UNIT_PROFIT_BPS, 120);
    reply.extend_from_slice(&measured_reply(MEASURED_UNIT_SLIPPAGE_BPS, 4));
    assert_eq!(
        read_measured_replies(&reply),
        vec![(MEASURED_UNIT_PROFIT_BPS, 120), (MEASURED_UNIT_SLIPPAGE_BPS, 4),],
        "both measurements, in the order they were written"
    );
    // And a reply carrying two answers is not a single measurement, which is what stops a
    // caller from reading the first and believing it has the whole reply.
    assert_eq!(read_measured_reply(&reply), None);
}

#[test]
fn an_opaque_reply_reports_nothing() {
    // The dry-run adapter's echo is the ordinary case: it says what it did, not what the
    // market did.
    assert!(read_measured_replies(b"dry-run-multi_hop_swap:1000:ethereum.USDC").is_empty());
    assert!(read_measured_replies(&[]).is_empty());
}

#[test]
fn a_truncated_record_is_not_a_measurement() {
    // Fail closed: half a number is not a number, and a guard that compared one would be
    // comparing the bytes of whatever followed in the reply.
    let mut short = Vec::new();
    short.push(CAPABILITY_REPLY_MEASURED_TAG);
    short.push(MEASURED_UNIT_PROFIT_BPS);
    short.extend_from_slice(&[1, 2, 3]);
    assert!(read_measured_replies(&short).is_empty());
}

#[test]
fn an_unknown_unit_is_reported_so_the_vm_can_ignore_it() {
    // The reader returns what the reply said; filtering by unit is the VM's job, because
    // a host answering a profit guard with a slippage is a fact about the reply rather
    // than about the wire form.
    let reply = measured_reply(9, 42);
    assert_eq!(read_measured_replies(&reply), vec![(9, 42)]);
}

#[test]
fn a_measurement_is_not_read_from_the_middle_of_an_opaque_reply() {
    // The tag is only a tag at the start: a reply that merely contains the byte is not a
    // measurement, or any binary payload could claim one.
    let mut reply = b"prefix".to_vec();
    reply.extend_from_slice(&measured_reply(MEASURED_UNIT_PROFIT_BPS, 999));
    assert!(read_measured_replies(&reply).is_empty());
}
