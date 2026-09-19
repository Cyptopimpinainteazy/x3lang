//! Cross-domain obligation netting — spec build-order item 22, PHASE 22.
//!
//! The tests are the property the analysis rests on, plus the refusals the phase's
//! own "where cryptographically valid" clause turns into. The property is checked
//! on every case, including the refused ones: `preserves_net_positions` is the
//! reason netting is safe rather than a redistribution, so a case that showed only
//! the saving would be showing the wrong half.

use x3_lang_compiler::netting::{self, Outcome};

/// A book from a list of `(debtor, creditor, amount, chain.ASSET)` obligations,
/// with every party appearing in it consenting.
fn book(obligations: &[(&str, &str, u128, &str)]) -> String {
    let mut parties: Vec<&str> = Vec::new();
    for (debtor, creditor, _, _) in obligations {
        for party in [debtor, creditor] {
            if !parties.contains(party) {
                parties.push(party);
            }
        }
    }
    parties.sort_unstable();
    let mut source = String::from("netting book_a {\n");
    for party in parties {
        source.push_str(&format!("    consent {party};\n"));
    }
    for (debtor, creditor, amount, asset) in obligations {
        source.push_str(&format!("    {debtor} owes {amount} {asset} to {creditor};\n"));
    }
    source.push_str("}\n");
    source
}

fn analyse(source: &str) -> netting::Book {
    let program = x3_lang_compiler::parser::parse_source(source).expect("a book must parse");
    let mut found = netting::books(&program).expect("a consented, well-formed book must analyse");
    assert_eq!(found.len(), 1, "the test source declares one book");
    found.remove(0)
}

/// Every party's net position in the book's first group, sorted by name.
fn positions(analysed: &netting::Book) -> Vec<(String, i128)> {
    let after = analysed.net_positions_after();
    let group = format!("{}.{}", analysed.groups[0].domain, analysed.groups[0].asset);
    let mut found: Vec<(String, i128)> = analysed
        .parties
        .iter()
        .map(|party| {
            (
                party.clone(),
                after.get(&(group.clone(), party.clone())).copied().unwrap_or(0),
            )
        })
        .collect();
    found.sort();
    found
}

/// The error `netting::verify` reports for a book, as one string.
fn refusal(source: &str) -> String {
    let program = x3_lang_compiler::parser::parse_source(source).expect("a book must parse");
    let mut acc = x3_lang_common::ErrorAccumulator::new();
    netting::verify(&program, &mut acc);
    assert!(acc.has_errors(), "the book must be refused: {source}");
    acc.errors()
        .iter()
        .map(|error| format!("{error}"))
        .collect::<Vec<_>>()
        .join("\n")
}

#[test]
fn a_book_that_owes_both_ways_settles_only_the_difference() {
    // alice owes bob 500 and bob owes alice 300, so only 200 has to move.
    let analysed = analyse(&book(&[
        ("alice", "bob", 500, "ethereum.USDC"),
        ("bob", "alice", 300, "ethereum.USDC"),
    ]));
    assert_eq!(analysed.groups.len(), 1, "one group: {}", analysed.groups.len());
    let Outcome::Netted {
        transfers,
        gross_before,
        gross_after,
    } = &analysed.groups[0].outcome
    else {
        panic!("the group must be netted: {:?}", analysed.groups[0].outcome);
    };
    assert_eq!(*gross_before, 800, "both obligations were promised");
    assert_eq!(*gross_after, 200, "only the difference moves");
    assert_eq!(transfers.len(), 1, "one transfer replaces two: {transfers:?}");
    assert_eq!(transfers[0].debtor, "alice", "the net payer is alice");
    assert_eq!(transfers[0].creditor, "bob");
    assert_eq!(transfers[0].amount, 200);
    assert_eq!(
        analysed.preserves_net_positions(),
        Ok(()),
        "the difference moved, the positions did not"
    );
}

#[test]
fn the_magnitude_flips_with_the_direction_of_the_larger_obligation() {
    // The same book the other way round: bob is the net payer.
    let analysed = analyse(&book(&[
        ("alice", "bob", 300, "ethereum.USDC"),
        ("bob", "alice", 500, "ethereum.USDC"),
    ]));
    let transfers = analysed.groups[0].transfers();
    assert_eq!(transfers.len(), 1, "one transfer: {transfers:?}");
    assert_eq!(transfers[0].debtor, "bob");
    assert_eq!(transfers[0].creditor, "alice");
    assert_eq!(transfers[0].amount, 200);
}

#[test]
fn a_party_that_only_passes_value_through_drops_out_of_the_book() {
    // The phase's own four lines: A owes X and receives Y, so A is a conduit. Here
    // carol owes alice 100 and alice owes bob 100, so alice's position is exactly
    // zero and the residual settles carol's debt to bob directly.
    let analysed = analyse(&book(&[
        ("carol", "alice", 100, "ethereum.USDC"),
        ("alice", "bob", 100, "ethereum.USDC"),
    ]));
    let transfers = analysed.groups[0].transfers();
    assert_eq!(transfers.len(), 1, "one transfer replaces two: {transfers:?}");
    assert_eq!(transfers[0].debtor, "carol");
    assert_eq!(transfers[0].creditor, "bob");
    assert_eq!(transfers[0].amount, 100);
    assert_eq!(
        positions(&analysed),
        vec![
            ("alice".to_string(), 0),
            ("bob".to_string(), 100),
            ("carol".to_string(), -100),
        ],
        "alice's position is zero before and after, so removing her moved nothing"
    );
    assert_eq!(analysed.preserves_net_positions(), Ok(()));
}

#[test]
fn netting_leaves_every_partys_net_position_alone() {
    // A book with a cycle and a party on both sides. The open positions are
    // hand-computed once, then every party is compared before against after.
    let analysed = analyse(&book(&[
        ("alice", "bob", 500, "ethereum.USDC"),
        ("bob", "alice", 300, "ethereum.USDC"),
        ("carol", "alice", 120, "ethereum.USDC"),
        ("alice", "carol", 40, "ethereum.USDC"),
    ]));
    let before = analysed.net_positions();
    let after = analysed.net_positions_after();
    let group = "ethereum.USDC".to_string();
    let position = |map: &std::collections::BTreeMap<(String, String), i128>, party: &str| {
        *map.get(&(group.clone(), party.to_string())).unwrap_or(&0)
    };
    assert_eq!(position(&before, "alice"), -120);
    assert_eq!(position(&before, "bob"), 200);
    assert_eq!(position(&before, "carol"), -80);
    for party in ["alice", "bob", "carol"] {
        assert_eq!(
            position(&before, party),
            position(&after, party),
            "'{party}' must be in the same position after netting"
        );
    }
    assert_eq!(analysed.preserves_net_positions(), Ok(()));
}

#[test]
fn netting_never_moves_more_than_the_book_promised_and_never_adds_a_transfer() {
    let analysed = analyse(&book(&[
        ("alice", "bob", 500, "ethereum.USDC"),
        ("bob", "alice", 300, "ethereum.USDC"),
        ("carol", "alice", 120, "ethereum.USDC"),
        ("alice", "carol", 40, "ethereum.USDC"),
    ]));
    let (gross_before, gross_after) = analysed.gross();
    assert_eq!(gross_before, 960);
    assert_eq!(gross_after, 200);
    assert!(
        gross_after <= gross_before,
        "netting removes movement: {gross_after} <= {gross_before}"
    );
    for group in &analysed.groups {
        assert!(
            group.transfers().len() <= group.obligations.len(),
            "a group must not need more transfers than it had obligations: {group:?}"
        );
    }
}

#[test]
fn two_obligations_in_the_same_asset_and_pair_become_one_transfer() {
    // Two promises from alice to bob are one debt of their sum.
    let analysed = analyse(&book(&[
        ("alice", "bob", 300, "ethereum.USDC"),
        ("alice", "bob", 200, "ethereum.USDC"),
    ]));
    let transfers = analysed.groups[0].transfers();
    assert_eq!(transfers.len(), 1, "two promises, one transfer: {transfers:?}");
    assert_eq!(transfers[0].amount, 500);
}

#[test]
fn two_assets_are_never_offset_because_that_would_need_a_price() {
    // alice owes bob 5 ETH and bob owes alice 5 USDC. Offsetting them would say 5
    // ETH discharges 5 USDC, which is a claim about a price this compiler does not
    // have — the same claim a hedge refuses.
    let analysed = analyse(&book(&[
        ("alice", "bob", 5, "ethereum.ETH"),
        ("bob", "alice", 5, "ethereum.USDC"),
    ]));
    assert_eq!(analysed.groups.len(), 2, "one group per asset");
    let (gross_before, gross_after) = analysed.gross();
    assert_eq!(
        (gross_before, gross_after),
        (10, 10),
        "nothing was offset, so nothing was saved"
    );
    assert_eq!(analysed.uncombined.len(), 1, "the pair must be named");
    let pair = &analysed.uncombined[0];
    assert_eq!((pair.party.as_str(), pair.counterparty.as_str()), ("alice", "bob"));
    assert!(
        pair.reason.contains("price"),
        "the reason must be the price the compiler lacks: {}",
        pair.reason
    );
    assert_eq!(analysed.preserves_net_positions(), Ok(()));
}

#[test]
fn two_domains_are_never_offset_because_a_delivery_on_one_is_not_a_payment_on_the_other() {
    // The phase is called *cross-domain* netting, and this is where its "where
    // cryptographically valid" clause bites: the same asset name on two ledgers is
    // two different claims, and treating one as the other is a bridge's job.
    let analysed = analyse(&book(&[
        ("alice", "bob", 5, "ethereum.USDC"),
        ("bob", "alice", 5, "x3.USDC"),
    ]));
    assert_eq!(analysed.groups.len(), 2, "one group per ledger");
    let (gross_before, gross_after) = analysed.gross();
    assert_eq!((gross_before, gross_after), (10, 10), "nothing was offset");
    assert_eq!(analysed.uncombined.len(), 1, "the pair must be named");
    assert!(
        analysed.uncombined[0].reason.contains("ledger"),
        "the reason must name the two ledgers: {}",
        analysed.uncombined[0].reason
    );
    assert_eq!(analysed.preserves_net_positions(), Ok(()));
}

#[test]
fn the_transfers_do_not_depend_on_the_order_the_obligations_were_written_in() {
    let forward = analyse(&book(&[
        ("alice", "bob", 500, "ethereum.USDC"),
        ("bob", "alice", 300, "ethereum.USDC"),
        ("carol", "alice", 120, "ethereum.USDC"),
        ("alice", "carol", 40, "ethereum.USDC"),
    ]));
    let reversed = analyse(&book(&[
        ("alice", "carol", 40, "ethereum.USDC"),
        ("carol", "alice", 120, "ethereum.USDC"),
        ("bob", "alice", 300, "ethereum.USDC"),
        ("alice", "bob", 500, "ethereum.USDC"),
    ]));
    assert_eq!(
        forward.groups[0].transfers(),
        reversed.groups[0].transfers(),
        "the residual is a function of the obligations, not of their order"
    );
    assert_eq!(forward.net_positions(), reversed.net_positions());
}

#[test]
fn a_book_that_offsets_a_party_that_did_not_consent_is_refused() {
    // bob takes part but never agreed, and netting rewrites who bob pays.
    let source = "netting book_a {\n    consent alice;\n    alice owes 500 ethereum.USDC to \
                  bob;\n    bob owes 300 ethereum.USDC to alice;\n}\n";
    let message = refusal(source);
    assert!(
        message.contains("offsets 'bob' but 'bob' did not consent"),
        "the refusal must name the party and the reason: {message}"
    );
    assert!(
        message.contains("consent bob;"),
        "the refusal must say what to write: {message}"
    );
}

#[test]
fn a_book_with_no_consent_at_all_is_refused() {
    let source = "netting book_a {\n    alice owes 500 ethereum.USDC to bob;\n    bob owes 300 \
                  ethereum.USDC to alice;\n}\n";
    let message = refusal(source);
    assert!(
        message.contains("declares no `consent`"),
        "the refusal must be about the missing consent: {message}"
    );
}

#[test]
fn consent_for_a_party_with_no_obligation_is_refused() {
    // The author expected carol to take part and she does not.
    let source = "netting book_a {\n    consent alice;\n    consent bob;\n    consent carol;\n    \
                  alice owes 500 ethereum.USDC to bob;\n    bob owes 300 ethereum.USDC to \
                  alice;\n}\n";
    let message = refusal(source);
    assert!(
        message.contains("'carol' consented") && message.contains("owes and is owed nothing"),
        "the refusal must say carol has no obligation to offset: {message}"
    );
}

#[test]
fn an_obligation_to_move_nothing_is_refused() {
    let source = "netting book_a {\n    consent alice;\n    consent bob;\n    alice owes 0 \
                  ethereum.USDC to bob;\n}\n";
    let message = refusal(source);
    assert!(
        message.contains("zero"),
        "the refusal must be about the zero amount: {message}"
    );
}

#[test]
fn a_party_cannot_owe_itself() {
    let source = "netting book_a {\n    consent alice;\n    alice owes 5 ethereum.USDC to alice;\n}\n";
    let message = refusal(source);
    assert!(
        message.contains("to itself"),
        "the refusal must say the obligation is to itself: {message}"
    );
}

#[test]
fn a_book_with_no_obligation_is_refused_at_the_parser() {
    // Removing every obligation from a book should not silently produce an empty
    // analysis.
    let error = x3_lang_compiler::parser::parse_source("netting book_a {\n    consent alice;\n}\n")
        .expect_err("an empty book must not parse");
    let message = format!("{error}");
    assert!(
        message.contains("declares no obligation"),
        "the refusal must be about the missing obligations: {message}"
    );
}

#[test]
fn a_fractional_amount_is_refused_by_the_parser() {
    let error = x3_lang_compiler::parser::parse_source(
        "netting book_a {\n    consent alice;\n    consent bob;\n    alice owes 1.5 ethereum.USDC \
         to bob;\n}\n",
    )
    .expect_err("a fractional obligation must not parse");
    let message = format!("{error}");
    assert!(
        message.contains("whole number of base units"),
        "the refusal must say why an amount is whole: {message}"
    );
}

#[test]
fn an_obligation_missing_its_creditor_is_refused_by_the_parser() {
    let error = x3_lang_compiler::parser::parse_source(
        "netting book_a {\n    consent alice;\n    alice owes 5 ethereum.USDC;\n}\n",
    )
    .expect_err("an obligation without a creditor must not parse");
    let message = format!("{error}");
    assert!(
        message.contains("to <creditor>") || message.contains("`to`"),
        "the refusal must show the shape the clause needs: {message}"
    );
}
