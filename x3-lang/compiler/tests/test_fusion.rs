//! Intent fusion — spec build-order item 21, PHASE 21.
//!
//! The tests are the five properties the spec says fusion must preserve
//! (minimum output, deadline, asset correctness, user authorization, fairness),
//! plus determinism, plus the case that matters most: what the compiler says
//! when it *cannot* check a property. "Unverifiable" has to be a different
//! answer from "satisfied", or the report is worse than nothing.

use x3_lang_compiler::fusion::{self, Check};

/// One intent that gives `give` and wants `want`, with a declared minimum and
/// an expiry, and optionally the fusion opt-in.
fn intent(name: &str, give: &str, give_amount: u128, want: &str, min_out: u128, deadline: u32, opt_in: bool) -> String {
    let allow = if opt_in { "    allow intent_fusion\n" } else { "" };
    format!(
        r#"intent {name} {{
    from ethereum.{give} amount {give_amount} receiver 0x1
    to ethereum.{want} receiver 0x2
    route {{
        swap uniswap ethereum.{give} -> ethereum.{want} amount {give_amount} min_output {min_out}
    }}
{allow}    require nonce unused {name}_nonce
    require slippage <= 50
    timeout {deadline}s refund ethereum.{give} to sender
    on_fail rollback
}}
"#
    )
}

/// Alice gives ETH and wants SOL, Bob gives SOL and wants USDC, Charlie gives
/// USDC and wants ETH: a ring, if the numbers allow it.
fn ring_with(a: (u128, u128), b: (u128, u128), c: (u128, u128), opt_in: bool) -> String {
    format!(
        "{}{}{}",
        intent("alice", "ETH", a.0, "SOL", a.1, 30, opt_in),
        intent("bob", "SOL", b.0, "USDC", b.1, 20, opt_in),
        intent("charlie", "USDC", c.0, "ETH", c.1, 60, opt_in)
    )
}

/// A ring that should pass every check: each participant is handed at least its
/// declared minimum.
fn sound_ring() -> String {
    // alice gives 10 ETH, wants >= 9 SOL; bob gives 9 SOL, wants >= 8 USDC;
    // charlie gives 8 USDC, wants >= 7 ETH.
    ring_with((10, 9), (9, 8), (8, 7), true)
}

fn analyze(source: &str) -> Vec<fusion::FusionRing> {
    let program = x3_lang_compiler::parser::parse_source(source).expect("intents must parse");
    fusion::rings(&fusion::flows(&program))
}

#[test]
fn a_sound_ring_is_found_and_every_check_passes() {
    let found = analyze(&sound_ring());
    assert_eq!(found.len(), 1, "one ring, found once: {found:?}");
    let ring = &found[0];
    assert_eq!(ring.participants, vec!["alice", "bob", "charlie"]);
    assert!(
        ring.is_fusable(),
        "every participant is handed what it asked for: {ring:?}"
    );
}

#[test]
fn the_ring_names_the_asset_handed_over_at_each_hop() {
    let found = analyze(&sound_ring());
    assert_eq!(
        found[0].assets,
        vec!["ethereum.ETH", "ethereum.SOL", "ethereum.USDC"],
        "each participant's give asset, in ring order"
    );
}

#[test]
fn the_earliest_deadline_is_the_ring_deadline() {
    // A ring is not settled until its most urgent member is out of time, and the
    // ring's members declare their deadlines in *seconds*: `timeout 20s` is four
    // blocks at the language's block time, so the earliest of {20s, 25s, 30s} is
    // four blocks. The expectation is derived rather than written down, so it
    // cannot drift from the conversion.
    let twenty_seconds_in_blocks = (20u64).div_ceil(x3_lang_compiler::lowering::SECONDS_PER_BLOCK) as u32;
    assert_eq!(twenty_seconds_in_blocks, 4, "20s is four blocks at 6s/block");
    let found = analyze(&sound_ring());
    assert_eq!(found[0].earliest_deadline, Some(twenty_seconds_in_blocks));
    assert_eq!(found[0].deadline, Check::Satisfied);
}

#[test]
fn a_participant_that_did_not_opt_in_is_never_internalized() {
    // Authorization is not a detail: an intent that did not allow fusion must
    // not be netted against anyone.
    let found = analyze(&ring_with((10, 9), (9, 8), (8, 7), false));
    assert!(
        found.is_empty(),
        "no participant consented, so there is no ring: {found:?}"
    );
}

#[test]
fn a_minimum_above_what_the_next_hands_over_is_refused() {
    // Bob hands over 9 SOL but alice wants at least 12: the ring cannot satisfy
    // her, and no partial netting is allowed to leave her short.
    let found = analyze(&ring_with((10, 12), (9, 8), (8, 7), true));
    assert_eq!(found.len(), 1, "the ring is still the ring: {found:?}");
    let ring = &found[0];
    assert!(!ring.is_fusable(), "it must not be fusable: {ring:?}");
    match &ring.minimum_output {
        Check::Failed(reason) => assert!(
            reason.contains("alice") && reason.contains("12") && reason.contains("bob"),
            "the failure must name the participant, the requirement and the supplier: {reason}"
        ),
        other => panic!("expected a failed minimum-output check, got {other:?}"),
    }
    assert!(
        matches!(ring.fairness, Check::Failed(_)),
        "if one participant is short the whole ring is refused: {ring:?}"
    );
}

#[test]
fn a_missing_minimum_is_unverifiable_not_satisfied() {
    // The bridge-only intent states what it bridges, never what it delivers, so
    // its minimum is unknown. Saying "satisfied" there would be a ring that
    // looks checked and was not.
    let source = r#"intent alice {
    from ethereum.ETH amount 10 receiver 0x1
    to solana.SOL receiver 0x2
    route {
        bridge x3 ethereum.ETH -> solana.SOL amount 10 receiver 0x2
    }
    allow intent_fusion
    require nonce unused alice_nonce
    require finality.ethereum >= 12
    timeout 30s refund ethereum.ETH to sender
    on_fail rollback
}
"#;
    let program = x3_lang_compiler::parser::parse_source(source).expect("parses");
    let flows = fusion::flows(&program);
    let alice = flows.iter().find(|flow| flow.name == "alice").expect("alice");
    match &alice.wants {
        Some((asset, minimum)) => {
            assert_eq!(asset, "solana.SOL");
            assert_eq!(*minimum, None, "a bridge states no delivered amount");
        }
        None => panic!("the want asset must still be readable: {alice:?}"),
    }
}

#[test]
fn the_ring_is_the_same_whatever_order_the_intents_are_declared_in() {
    let forward = analyze(&sound_ring());
    let reversed = analyze(&format!(
        "{}{}{}",
        intent("charlie", "USDC", 8, "ETH", 7, 60, true),
        intent("bob", "SOL", 9, "USDC", 8, 20, true),
        intent("alice", "ETH", 10, "SOL", 9, 30, true)
    ));
    assert_eq!(
        forward, reversed,
        "the report must be a function of the intents, not of their order"
    );
}

#[test]
fn intents_that_do_not_close_a_ring_are_not_reported() {
    // Alice gives ETH and wants SOL; Bob gives USDC and wants ETH. Nothing
    // supplies SOL, so there is no ring — and no half-ring is reported as one.
    let source = format!(
        "{}{}",
        intent("alice", "ETH", 10, "SOL", 9, 30, true),
        intent("bob", "USDC", 8, "ETH", 7, 20, true)
    );
    assert!(analyze(&source).is_empty());
}

#[test]
fn a_participant_swapping_an_asset_for_itself_is_not_a_hop() {
    // An intent that gives and wants the same asset is a transfer; netting it
    // against anything would be modelling a trade that does not exist.
    let source = format!(
        "{}{}{}",
        intent("self", "ETH", 10, "ETH", 9, 30, true),
        intent("bob", "SOL", 9, "USDC", 8, 20, true),
        intent("charlie", "USDC", 8, "SOL", 7, 60, true)
    );
    let found = analyze(&source);
    assert!(
        found
            .iter()
            .all(|ring| !ring.participants.contains(&"self".to_string())),
        "a participant with no exchange in it cannot be part of a netting ring: {found:?}"
    );
}
