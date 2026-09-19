//! Cross-domain obligation netting — spec PHASE 22.
//!
//! The surface, which is a grammar sketch rather than Rust and is fenced as text
//! so the doctest runner does not try to compile it:
//!
//! ```text
//! netting book_a {
//!     consent alice;
//!     consent bob;
//!     alice owes 500 ethereum.USDC to bob;
//!     bob owes 300 ethereum.USDC to alice;
//! }
//! ```
//!
//! A book of obligations between named parties. The phase's premise is that the
//! obligations can be discharged by what *remains* after they are offset, so less
//! value moves than the sum of what was promised. Its listed benefits — less
//! capital movement, fewer transactions, lower fees, lower liquidity demand, lower
//! settlement risk — are all consequences of one measured quantity, so this module
//! measures it rather than asserting it: every group reports the gross movement and
//! the transfer count before and after.
//!
//! ## The invariant that makes netting safe
//!
//! Netting changes *who pays whom*. The property that must survive is that nobody's
//! **net position** changes: for every party, what it pays out minus what it takes
//! in is the same after netting as before, in every `(domain, asset)` group. If that
//! holds, netting has not moved value between parties — it has only removed
//! circular movement. `nets()` computes the positions, `Book::preserves_net_positions`
//! checks the two against each other, and the tests assert it on every case,
//! including the ones that are refused (a refused group keeps its obligations, so it
//! trivially preserves them, and the tests check that instead of skipping it).
//!
//! ## Where the phase says "where cryptographically valid"
//!
//! Two obligations are only offsets of each other if settling one discharges the
//! other. That is true inside one ledger and false across two, and false between two
//! assets. So obligations are netted **one `(domain, asset)` group at a time**, and
//! the module refuses rather than approximates:
//!
//! - **unlike assets are never offset.** `alice owes 5 ethereum.ETH to bob; bob owes
//!   5 ethereum.USDC to alice;` would need a price to offset, and this compiler does
//!   not have one. `compiler/src/hedge.rs` refuses the same claim in a hedge for the
//!   same reason, and this module quotes that reason rather than inventing a rate.
//! - **unlike domains are never offset.** An obligation to deliver on Ethereum is not
//!   a payment on X3; treating one as the other is a bridge's job, and a bridge
//!   carries its own trust assumptions that a netting analysis has no business
//!   importing. Where a pair of parties owes each other on two ledgers, the report
//!   names the pair and says the two were not combined.
//! - **a party that did not consent is never netted.** Netting rewrites an
//!   obligation's counterparty, which is a change to what that party agreed to.
//!   `consent` is the opt-in, the same way `allow intent_fusion` is the opt-in a
//!   non-consenting intent is never internalized through (`compiler/src/fusion.rs`).
//!
//! ## What is not implemented
//!
//! The residual transfers are decided and reported; nothing executes them. A
//! coordinator would take the residual set and settle it, and the parties here are
//! symbols rather than accounts, so there is nothing for the VM to debit. Both the
//! IR verifier and the emitter refuse a book for that reason, `check` and `build`
//! agree, and TICKET-071 tracks the executor.

use std::collections::{BTreeMap, BTreeSet};

use x3_lang_ast::ast::{Item, NettingDecl, ObligationDecl, Program};
use x3_lang_common::{ErrorAccumulator, Span, X3Error};

/// One obligation as written: `debtor` owes `amount` of `domain.asset` to `creditor`.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct Obligation {
    pub debtor: String,
    pub creditor: String,
    pub domain: String,
    pub asset: String,
    pub amount: u128,
}

impl Obligation {
    /// The `(domain, asset)` group this obligation can be offset inside.
    pub fn group(&self) -> String {
        format!("{}.{}", self.domain, self.asset)
    }
}

/// One transfer left standing after a group's obligations were offset.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct Transfer {
    pub debtor: String,
    pub creditor: String,
    pub amount: u128,
}

/// What netting one `(domain, asset)` group decided.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Outcome {
    /// The obligations were offset. `gross_before`/`gross_after` are summed over
    /// the group's own units, so the two are comparable and the difference is the
    /// movement netting removed.
    Netted {
        transfers: Vec<Transfer>,
        gross_before: u128,
        gross_after: u128,
    },
    /// The group was not offset, and this is why.
    Refused(String),
}

/// One `(domain, asset)` group of a book.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Group {
    pub domain: String,
    pub asset: String,
    pub obligations: Vec<Obligation>,
    pub outcome: Outcome,
}

impl Group {
    /// Gross movement before netting: everything the group promised to move.
    pub fn gross_before(&self) -> u128 {
        self.obligations.iter().map(|obligation| obligation.amount).sum()
    }

    /// The transfers that remain — the obligations themselves when the group was
    /// refused, because a refused group is not netted at all.
    pub fn transfers(&self) -> Vec<Transfer> {
        match &self.outcome {
            Outcome::Netted { transfers, .. } => transfers.clone(),
            Outcome::Refused(_) => self
                .obligations
                .iter()
                .map(|obligation| Transfer {
                    debtor: obligation.debtor.clone(),
                    creditor: obligation.creditor.clone(),
                    amount: obligation.amount,
                })
                .collect(),
        }
    }
}

/// A pair of parties that owes each other on two ledgers, or in two assets — named
/// so the report can say what it did not combine rather than leaving the reader to
/// assume it combined everything.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct Uncombined {
    pub party: String,
    pub counterparty: String,
    /// The two groups, in the order the analysis reached them.
    pub groups: (String, String),
    /// Why they were not offset.
    pub reason: String,
}

/// A whole analysed book.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Book {
    pub name: String,
    /// Every party that owes or is owed, sorted.
    pub parties: Vec<String>,
    /// One group per `(domain, asset)`, in a deterministic order.
    pub groups: Vec<Group>,
    /// Pairs that owed each other in unlike units, and were not combined.
    pub uncombined: Vec<Uncombined>,
}

impl Book {
    /// Every party's net position per group, taken over the obligations as written:
    /// `(domain.asset, party) -> inflow − outflow`.
    pub fn net_positions(&self) -> BTreeMap<(String, String), i128> {
        let mut positions: BTreeMap<(String, String), i128> = BTreeMap::new();
        for group in &self.groups {
            for obligation in &group.obligations {
                *positions
                    .entry((obligation.group(), obligation.debtor.clone()))
                    .or_default() -= obligation.amount as i128;
                *positions
                    .entry((obligation.group(), obligation.creditor.clone()))
                    .or_default() += obligation.amount as i128;
            }
        }
        positions
    }

    /// Every party's net position per group, taken over the residual transfers.
    pub fn net_positions_after(&self) -> BTreeMap<(String, String), i128> {
        let mut positions: BTreeMap<(String, String), i128> = BTreeMap::new();
        for group in &self.groups {
            for transfer in group.transfers() {
                *positions
                    .entry((group_key(&group.domain, &group.asset), transfer.debtor.clone()))
                    .or_default() -= transfer.amount as i128;
                *positions
                    .entry((group_key(&group.domain, &group.asset), transfer.creditor.clone()))
                    .or_default() += transfer.amount as i128;
            }
        }
        // A party that netted to zero has no entry on the transfer side; the
        // comparison is over the union of both maps, so the zeroes are filled in
        // rather than treated as missing.
        positions
    }

    /// Whether netting left every party's net position alone — the property that
    /// makes offsetting safe rather than a redistribution. Returns the first party
    /// whose position moved, with both figures, when it did.
    pub fn preserves_net_positions(&self) -> Result<(), String> {
        let before = self.net_positions();
        let after = self.net_positions_after();
        let keys: BTreeSet<&(String, String)> = before.keys().chain(after.keys()).collect();
        for key in keys {
            let was = before.get(key).copied().unwrap_or(0);
            let now = after.get(key).copied().unwrap_or(0);
            if was != now {
                return Err(format!(
                    "'{}' had a net position of {was} in {} before netting and {now} after",
                    key.1, key.0
                ));
            }
        }
        Ok(())
    }

    /// Total gross movement across every group, before and after.
    pub fn gross(&self) -> (u128, u128) {
        self.groups.iter().fold((0, 0), |(before, after), group| {
            (
                before + group.gross_before(),
                after + group.transfers().iter().map(|transfer| transfer.amount).sum::<u128>(),
            )
        })
    }
}

/// Decide a book from its own declaration.
pub fn book(decl: &NettingDecl) -> Result<Book, String> {
    let consented: BTreeSet<&str> = decl.consent.iter().map(|party| party.as_str()).collect();

    if consented.is_empty() {
        return Err(format!(
            "the netting book '{}' declares no `consent`: netting changes who pays whom, and a \
             compiler does not decide that for parties that have not agreed",
            decl.name.as_str()
        ));
    }

    let mut obligations: Vec<Obligation> = Vec::with_capacity(decl.obligations.len());
    for (index, declaration) in decl.obligations.iter().enumerate() {
        obligations.push(read_obligation(declaration, index)?);
    }

    let parties: BTreeSet<String> = obligations
        .iter()
        .flat_map(|obligation| [obligation.debtor.clone(), obligation.creditor.clone()])
        .collect();

    // A consent for a party that owes and is owed nothing is a typo, and it is
    // worth saying so: the author expected that party to take part and it does not.
    for party in &decl.consent {
        if !parties.contains(party.as_str()) {
            return Err(format!(
                "'{}' consented to netting in book '{}' but owes and is owed nothing in it, so \
                 there is no obligation of theirs to offset",
                party.as_str(),
                decl.name.as_str()
            ));
        }
    }

    // Consent is a property of the party, not of the group: a party that owes in
    // one asset and is owed in another has to have agreed once, and its absence is
    // reported with the obligation it would have rewritten.
    for obligation in &obligations {
        for party in [&obligation.debtor, &obligation.creditor] {
            if !consented.contains(party.as_str()) {
                return Err(format!(
                    "the netting book '{}' offsets '{}' but '{}' did not consent: netting rewrites \
                     who pays whom, and that is not a decision a compiler may make for a party \
                     that has not agreed. Write `consent {};` in the book, or remove the \
                     obligation",
                    decl.name.as_str(),
                    party,
                    party,
                    party
                ));
            }
        }
    }

    let mut by_group: BTreeMap<String, Vec<Obligation>> = BTreeMap::new();
    for obligation in obligations {
        // A party cannot owe itself. The two sides cancel in isolation, so netting
        // it against the book would delete an obligation that may be a wash trade
        // the participant meant to record, and that is not the analysis's call.
        if obligation.debtor == obligation.creditor {
            return Err(format!(
                "the netting book '{}' has '{}' owing {} {} to itself; a self-obligation is not an \
                 obligation between parties to offset",
                decl.name.as_str(),
                obligation.debtor,
                obligation.amount,
                obligation.group()
            ));
        }
        by_group.entry(obligation.group()).or_default().push(obligation);
    }

    let mut groups: Vec<Group> = Vec::with_capacity(by_group.len());
    for (key, mut group_obligations) in by_group {
        group_obligations.sort();
        let (domain, asset) = split_key(&key);
        let outcome = net_group(&group_obligations);
        groups.push(Group {
            domain,
            asset,
            obligations: group_obligations,
            outcome,
        });
    }

    let uncombined = uncombined_pairs(&groups);
    let parties: Vec<String> = parties.into_iter().collect();

    let analysed = Book {
        name: decl.name.as_str().to_string(),
        parties,
        groups,
        uncombined,
    };

    // The invariant is checked here, not only in the tests: if a change to the
    // offsetting ever moved a party's position, the compilation fails rather than
    // reporting a saving that came out of somebody's balance.
    if let Err(reason) = analysed.preserves_net_positions() {
        return Err(format!(
            "netting the book '{}' would have changed a participant's position, so it was refused: \
             {reason}",
            decl.name.as_str()
        ));
    }
    Ok(analysed)
}

/// Read one declared obligation, refusing the shapes that are not obligations.
fn read_obligation(declaration: &ObligationDecl, index: usize) -> Result<Obligation, String> {
    let position = index + 1;
    if declaration.amount == 0 {
        return Err(format!(
            "obligation {position} in the book ('{}' owes '{}') is for zero {}: an obligation to \
             move nothing has no net position, and netting it would report a transfer that is not \
             there",
            declaration.debtor.as_str(),
            declaration.creditor.as_str(),
            declaration.asset.name.as_str()
        ));
    }
    Ok(Obligation {
        debtor: declaration.debtor.as_str().to_string(),
        creditor: declaration.creditor.as_str().to_string(),
        domain: declaration.asset.chain.as_str().to_string(),
        asset: declaration.asset.name.as_str().to_string(),
        amount: declaration.amount,
    })
}

/// Offset one group's obligations, one `(domain, asset)` at a time.
///
/// Every obligation adds to one party's outflow and one party's inflow, so the
/// group's positions always sum to zero and the residual set always balances. What
/// the residual set is *not* allowed to do is change any position — `book` checks
/// that separately, over the whole book.
fn net_group(obligations: &[Obligation]) -> Outcome {
    let mut positions: BTreeMap<&str, i128> = BTreeMap::new();
    for obligation in obligations {
        *positions.entry(obligation.debtor.as_str()).or_default() -= obligation.amount as i128;
        *positions.entry(obligation.creditor.as_str()).or_default() += obligation.amount as i128;
    }

    let mut debtors: Vec<(&str, u128)> = Vec::new();
    let mut creditors: Vec<(&str, u128)> = Vec::new();
    for (party, position) in &positions {
        match position {
            // A party whose obligations cancel exactly has nothing to move. Leaving
            // it out is the whole saving in the pass-through case the phase's own
            // example writes.
            0 => {}
            position if *position < 0 => match u128::try_from(-*position) {
                Ok(amount) => debtors.push((party, amount)),
                Err(_) => {
                    return Outcome::Refused(format!(
                        "the net position of '{party}' does not fit the amount type, so the group \
                         was not offset"
                    ))
                }
            },
            position => match u128::try_from(*position) {
                Ok(amount) => creditors.push((party, amount)),
                Err(_) => {
                    return Outcome::Refused(format!(
                        "the net position of '{party}' does not fit the amount type, so the group \
                         was not offset"
                    ))
                }
            },
        }
    }

    let owed: u128 = debtors.iter().map(|(_, amount)| *amount).sum();
    let due: u128 = creditors.iter().map(|(_, amount)| *amount).sum();
    if owed != due {
        // Unreachable while the positions are computed from the same obligations,
        // and kept because "the residual balances" is the claim the transfers make.
        return Outcome::Refused(format!(
            "the group's residual does not balance: {owed} would move out and {due} in, so the \
             group was not offset"
        ));
    }

    // Deterministic settlement order: the largest obligation first, and parties by
    // name where two are equal, so the transfer list is a function of the
    // obligations rather than of the order they were written in.
    debtors.sort_by(|left, right| right.1.cmp(&left.1).then(left.0.cmp(right.0)));
    creditors.sort_by(|left, right| right.1.cmp(&left.1).then(left.0.cmp(right.0)));

    let mut transfers: Vec<Transfer> = Vec::new();
    let mut debtor_index = 0usize;
    let mut creditor_index = 0usize;
    while debtor_index < debtors.len() && creditor_index < creditors.len() {
        let (debtor, owed) = debtors[debtor_index];
        let (creditor, due) = creditors[creditor_index];
        let amount = owed.min(due);
        if amount > 0 {
            transfers.push(Transfer {
                debtor: debtor.to_string(),
                creditor: creditor.to_string(),
                amount,
            });
        }
        match owed.cmp(&due) {
            std::cmp::Ordering::Equal => {
                debtor_index += 1;
                creditor_index += 1;
            }
            std::cmp::Ordering::Greater => {
                debtors[debtor_index].1 = owed - due;
                creditor_index += 1;
            }
            std::cmp::Ordering::Less => {
                creditors[creditor_index].1 = due - owed;
                debtor_index += 1;
            }
        }
    }

    let gross_before: u128 = obligations.iter().map(|obligation| obligation.amount).sum();
    let gross_after: u128 = transfers.iter().map(|transfer| transfer.amount).sum();
    Outcome::Netted {
        transfers,
        gross_before,
        gross_after,
    }
}

/// Pairs of parties that owe each other in unlike units, named so the report can
/// say what it declined to combine.
fn uncombined_pairs(groups: &[Group]) -> Vec<Uncombined> {
    let mut found: Vec<Uncombined> = Vec::new();
    for (index, left) in groups.iter().enumerate() {
        for right in groups.iter().skip(index + 1) {
            for one in &left.obligations {
                for other in &right.obligations {
                    let opposite = one.debtor == other.creditor && one.creditor == other.debtor;
                    let same = one.debtor == other.debtor && one.creditor == other.creditor;
                    if !opposite && !same {
                        continue;
                    }
                    let (left_key, right_key) = (one.group(), other.group());
                    let reason = if one.domain != other.domain {
                        format!(
                            "one is owed on {} and the other on {}: a delivery on one ledger is not \
                             a payment on another, so offsetting them would be a bridge's claim \
                             rather than a netting one",
                            one.domain, other.domain
                        )
                    } else {
                        format!(
                            "one is in {} and the other in {}: offsetting two assets needs a price, \
                             and this compiler does not have one",
                            one.asset, other.asset
                        )
                    };
                    found.push(Uncombined {
                        party: one.debtor.clone(),
                        counterparty: one.creditor.clone(),
                        groups: (left_key, right_key),
                        reason,
                    });
                }
            }
        }
    }
    found.sort();
    found.dedup();
    found
}

fn group_key(domain: &str, asset: &str) -> String {
    format!("{domain}.{asset}")
}

/// Split a `domain.asset` group key back into its parts.
fn split_key(key: &str) -> (String, String) {
    match key.split_once('.') {
        Some((domain, asset)) => (domain.to_string(), asset.to_string()),
        None => (key.to_string(), String::new()),
    }
}

/// Report every book in a program, or an error naming the first thing that is not
/// an obligation book.
pub fn books(program: &Program) -> Result<Vec<Book>, String> {
    let mut books = Vec::new();
    for item in &program.items {
        if let Item::Netting(decl) = &item.node {
            books.push(book(decl)?);
        }
    }
    Ok(books)
}

/// Check every `netting` book in a program, in the same layer as the hedge,
/// liquidation and rebalance checks — before anything is lowered.
pub fn verify(program: &Program, acc: &mut ErrorAccumulator) {
    for item in &program.items {
        let Item::Netting(decl) = &item.node else {
            continue;
        };
        if let Err(reason) = book(decl) {
            acc.add_error(X3Error::SemanticError {
                message: reason,
                span: Span::DUMMY,
            });
        }
    }
}
