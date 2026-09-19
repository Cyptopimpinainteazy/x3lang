//! Strategy modules — spec build-order item 22, PHASE 23.
//!
//! A module declares eight things: inputs, outputs, effects, guarantees,
//! permissions, required domains, a risk profile, and resource bounds. Eight
//! required lines would be decoration if nothing read them, so each check here
//! is one declaration being *used*:
//!
//! - **inputs, outputs, domains, risk, bounds** — present, non-empty, in range.
//!   A module that bounds nothing has not bounded anything.
//! - **effects** — every declared effect must be discharged by `execute`. A
//!   module that claims to swap and does not swap is claiming work it does not
//!   do.
//! - **guarantees** — the same, and the diagnostic says what to add. A guarantee
//!   nobody can point at a statement for is a promise with nothing behind it.
//! - **permissions** — the body may not exceed them: a body touching more than
//!   one chain needs `cross_domain`, one that opts into netting needs
//!   `intent_fusion`. The direction matters: declaring more than you use is
//!   fine (it is a capability), doing more than you declared is not.
//! - **domains** — every chain the body touches must be declared. A module that
//!   reaches a chain it never listed has not declared its requirements.
//!
//! ## What a module body cannot say, and why that is reported rather than hidden
//!
//! A module body is the general statement language, which has no `borrow` or
//! `repay`. So `borrow`/`repay` effects and the `debt_closed` guarantee have no
//! statement that can discharge them here, and a module declaring them is told
//! exactly that instead of being quietly accepted. Debt lives on an atomic
//! trade, which has the statements for it.
//!
//! `flash_capital` is in the same position from the other side: nothing in a
//! module body can *require* it, so declaring it is a capability the compiler
//! cannot currently test. That is recorded in TICKET-040 rather than papered
//! over with a check that always passes.

use std::collections::BTreeSet;

use x3_lang_ast::ast::{SplitRecipient, Statement, StrategyPermission};
use x3_lang_common::{ErrorAccumulator, X3Error};

use x3_lang_ast::ast::{Item, Program};

use x3_lang_ast::trading::{TradeEffect, TradeGuarantee};

/// Largest basis-point figure any risk field may carry.
const MAX_BPS: u32 = 10_000;

fn err(message: String) -> X3Error {
    X3Error::SemanticError {
        message,
        span: x3_lang_common::Span::DUMMY,
    }
}

/// Every statement of a body, including the ones inside blocks.
fn walk<'a>(statements: &'a [Statement], out: &mut Vec<&'a Statement>) {
    for statement in statements {
        out.push(statement);
        match statement {
            Statement::Atomic(block) => walk(&block.body.stmts, out),
            Statement::If {
                then_block, else_block, ..
            } => {
                walk(&then_block.stmts, out);
                if let Some(block) = else_block {
                    walk(&block.stmts, out);
                }
            }
            Statement::Loop(block) => walk(&block.stmts, out),
            Statement::While { body, .. } | Statement::For { body, .. } => walk(&body.stmts, out),
            _ => {}
        }
    }
}

/// The chains a body's statements name.
fn chains(statements: &[&Statement]) -> BTreeSet<String> {
    let mut found = BTreeSet::new();
    for statement in statements {
        match statement {
            Statement::Lock { chain, .. } | Statement::Release { chain, .. } => {
                found.insert(chain.as_str().to_string());
            }
            Statement::Mint { asset, .. } | Statement::Burn { asset, .. } => {
                found.insert(asset.chain.as_str().to_string());
            }
            Statement::Swap { from, to, .. } => {
                found.insert(from.chain.as_str().to_string());
                found.insert(to.chain.as_str().to_string());
            }
            Statement::Bridge { from, to, .. } => {
                found.insert(from.chain.as_str().to_string());
                found.insert(to.chain.as_str().to_string());
            }
            _ => {}
        }
    }
    found
}

/// Verify every strategy module in a program.
pub fn verify_strategy_modules(program: &Program, acc: &mut ErrorAccumulator) {
    for item in &program.items {
        let Item::Strategy(module) = &item.node else {
            continue;
        };
        let name = module.name.as_str();
        let mut statements = Vec::new();
        walk(&module.body, &mut statements);

        // ── the declarations that are simply required ──────────────────────
        if module.inputs.is_empty() {
            acc.add_error(err(format!(
                "strategy '{name}' declares no input; a reusable module that does not say what it is \
                 handed cannot be instantiated"
            )));
        }
        for input in &module.inputs {
            if input.amount.is_none() {
                acc.add_error(err(format!(
                    "strategy '{name}' input {}.{} states no amount; capital nobody bounded is capital \
                     nobody agreed to",
                    input.asset.chain.as_str(),
                    input.asset.name.as_str()
                )));
            }
        }
        if module.outputs.is_empty() {
            acc.add_error(err(format!(
                "strategy '{name}' declares no output; a module that does not say what it produces \
                 cannot be composed"
            )));
        }
        if module.domains.is_empty() {
            acc.add_error(err(format!(
                "strategy '{name}' declares no required domains; the plan cannot know which VMs it \
                 needs"
            )));
        }
        match &module.risk {
            None => acc.add_error(err(format!(
                "strategy '{name}' declares no risk profile; the bounds it accepts are part of the \
                 module, not an operator's setting"
            ))),
            Some(risk) => {
                if risk.max_slippage_bps > MAX_BPS {
                    acc.add_error(err(format!(
                        "strategy '{name}' declares max_slippage_bps {}, above {MAX_BPS}",
                        risk.max_slippage_bps
                    )));
                }
                if risk.max_total_fee_bps > MAX_BPS {
                    acc.add_error(err(format!(
                        "strategy '{name}' declares max_total_fee_bps {}, above {MAX_BPS}",
                        risk.max_total_fee_bps
                    )));
                }
            }
        }
        match (
            module.max_steps.as_ref().and_then(expression_to_u128),
            module.max_gas.as_ref().and_then(expression_to_u128),
        ) {
            (Some(steps), _) if steps > 0 => {}
            (Some(_), _) => acc.add_error(err(format!("strategy '{name}' bounds max_steps at zero"))),
            (None, _) => acc.add_error(err(format!(
                "strategy '{name}' declares no max_steps bound; an unbounded module is not a bounded \
                 strategy"
            ))),
        }
        if module.max_gas.is_none() {
            acc.add_error(err(format!(
                "strategy '{name}' declares no max_gas bound; resource bounds are part of the module"
            )));
        }

        // ── effects, discharged by the body ───────────────────────────────
        for effect in &module.effects {
            match effect_is_discharged(*effect, &statements) {
                Discharge::Yes => {}
                Discharge::No => acc.add_error(err(format!(
                    "strategy '{name}' declares effect '{}' but nothing in `execute` performs it",
                    effect.as_str()
                ))),
                Discharge::NoStatementForm => acc.add_error(err(format!(
                    "strategy '{name}' declares effect '{}', and a module body has no statement that \
                     could discharge it; debt effects belong to an atomic trade, which has the \
                     statements for them",
                    effect.as_str()
                ))),
            }
        }

        // ── guarantees, discharged by the body ────────────────────────────
        for guarantee in &module.guarantees {
            match guarantee_is_discharged(*guarantee, &statements) {
                Discharge::Yes => {}
                Discharge::No => acc.add_error(err(format!(
                    "strategy '{name}' declares guarantee '{}' but nothing in `execute` discharges \
                     it; {}",
                    guarantee.as_str(),
                    guarantee_requirement(*guarantee)
                ))),
                Discharge::NoStatementForm => acc.add_error(err(format!(
                    "strategy '{name}' declares guarantee '{}', and a module body has no statement \
                     that could discharge it; declare it on an atomic trade instead",
                    guarantee.as_str()
                ))),
            }
        }

        // ── permissions: the body may not exceed them ─────────────────────
        let body_chains = chains(&statements);
        if body_chains.len() > 1 && !module.permissions.contains(&StrategyPermission::CrossDomain) {
            acc.add_error(err(format!(
                "strategy '{name}' `execute` touches {} chains ({}) but the module does not declare \
                 the `cross_domain` permission",
                body_chains.len(),
                body_chains.iter().cloned().collect::<Vec<_>>().join(", ")
            )));
        }
        if statements
            .iter()
            .any(|statement| matches!(statement, Statement::Allow { .. }))
            && !module.permissions.contains(&StrategyPermission::IntentFusion)
        {
            acc.add_error(err(format!(
                "strategy '{name}' `execute` opts into intent fusion but the module does not declare \
                 the `intent_fusion` permission"
            )));
        }

        // ── required domains: every chain the body reaches must be declared ─
        let declared: BTreeSet<String> = module
            .domains
            .iter()
            .map(|domain| domain.as_str().to_string())
            .collect();
        for chain in &body_chains {
            if !declared.contains(chain) {
                acc.add_error(err(format!(
                    "strategy '{name}' `execute` touches chain '{chain}' but the module's domains do \
                     not include it; a module that reaches a chain it never listed has not declared \
                     its requirements"
                )));
            }
        }

        // ── the licence, the split, and the royalty between them ───────────
        //
        // PHASE 24's constraint is that licensing must never compromise
        // deterministic execution. The licence lowers to a *record*, not to an
        // instruction that can fail, so it cannot change what a program does —
        // and the check that it pays the author is a compile-time comparison of
        // two declared numbers rather than a run-time branch.
        if let Some(split) = &module.split {
            let total: u32 = split.shares.iter().map(|(_, bps)| *bps).sum();
            if total != 10_000 {
                acc.add_error(err(format!(
                    "strategy '{name}' profit split totals {total} bps, not 10_000; a split that does \
                     not add up is distributing something it does not have, or leaving part of the \
                     profit unassigned"
                )));
            }
            let mut seen: Vec<&str> = Vec::new();
            for (recipient, bps) in &split.shares {
                if seen.contains(&recipient.as_str()) {
                    acc.add_error(err(format!(
                        "strategy '{name}' profit split names '{}' twice; the shares would be \
                         ambiguous",
                        recipient.as_str()
                    )));
                }
                seen.push(recipient.as_str());
                if *bps == 0 {
                    acc.add_error(err(format!(
                        "strategy '{name}' profit split gives '{}' nothing; leave the recipient out \
                         rather than writing a zero share",
                        recipient.as_str()
                    )));
                }
            }

            // PHASE 25: distribution happens after final net profit is known, so
            // a split needs a profit floor to distribute. Without one there is no
            // "net profit" the split applies to.
            let has_floor = statements.iter().any(|statement| match statement {
                Statement::Require(guard) => {
                    guard.kind == x3_lang_ast::ast::RequireKind::Profit
                        && guard.comparison.is_some_and(|op| op.is_lower_bound())
                }
                _ => false,
            });
            if !has_floor {
                acc.add_error(err(format!(
                    "strategy '{name}' splits profit but asserts no profit floor; distribution \
                     happens after final net profit is known, so the body has to say what that is \
                     (`require profit >= <amount>`)"
                )));
            }

            // The royalty the licence promises must be a share the split pays.
            // A royalty the split does not pay is a promise the artifact does
            // not keep.
            if let Some(license) = &module.license {
                let paid = split
                    .shares
                    .iter()
                    .find(|(recipient, _)| *recipient == SplitRecipient::StrategyAuthor)
                    .map(|(_, bps)| *bps)
                    .unwrap_or(0);
                if paid < license.profit_share_bps {
                    acc.add_error(err(format!(
                        "strategy '{name}' licence grants the author {} bps of profit but the split \
                         pays {} bps; the royalty has to be a share the split actually pays",
                        license.profit_share_bps, paid
                    )));
                }
            }
        } else if let Some(license) = &module.license {
            // A licence that grants a profit share and no split to pay it from
            // is the same unkept promise, one step earlier.
            if license.profit_share_bps > 0 {
                acc.add_error(err(format!(
                    "strategy '{name}' licence grants the author {} bps of profit but the module \
                     declares no `split profit`; there is nothing for the royalty to be paid from",
                    license.profit_share_bps
                )));
            }
        }

        if let Some(license) = &module.license {
            if license.profit_share_bps > MAX_BPS {
                acc.add_error(err(format!(
                    "strategy '{name}' licence share {} bps exceeds {MAX_BPS}",
                    license.profit_share_bps
                )));
            }
            if license.executions == Some(0) {
                acc.add_error(err(format!(
                    "strategy '{name}' licence grants zero executions; that is not a licence to run \
                     the module"
                )));
            }
        }

        // ── the declared risk profile must bound the body's own guards ─────
        if let Some(risk) = &module.risk {
            for statement in &statements {
                if let Statement::Require(guard) = statement {
                    if guard.kind == x3_lang_ast::ast::RequireKind::Slippage {
                        if let Some(bound) = guard.comparison.and_then(|op| {
                            op.is_upper_bound()
                                // In the one unit a slippage bound is written in:
                                // basis points, the same unit as the profile's field
                                // (TICKET-054).
                                .then(|| {
                                    guard
                                        .value
                                        .as_ref()
                                        .and_then(crate::semantic::slippage_bps_from_expr)
                                        .map(u128::from)
                                })
                                .flatten()
                        }) {
                            if bound > u128::from(risk.max_slippage_bps) {
                                acc.add_error(err(format!(
                                    "strategy '{name}' `execute` relies on `require slippage <= \
                                     {bound}`, above the module's declared max_slippage_bps {}; the \
                                     risk profile has to bound what the body accepts",
                                    risk.max_slippage_bps
                                )));
                            }
                        }
                    }
                }
            }
        }
    }
}

/// Whether a declaration is discharged by a body.
enum Discharge {
    Yes,
    No,
    /// The body has no statement form for it at all.
    NoStatementForm,
}

fn effect_is_discharged(effect: TradeEffect, statements: &[&Statement]) -> Discharge {
    match effect {
        TradeEffect::Swap => {
            if statements.iter().any(|s| matches!(s, Statement::Swap { .. })) {
                Discharge::Yes
            } else {
                Discharge::No
            }
        }
        TradeEffect::Bridge => {
            if statements.iter().any(|s| matches!(s, Statement::Bridge { .. })) {
                Discharge::Yes
            } else {
                Discharge::No
            }
        }
        TradeEffect::Borrow | TradeEffect::Repay => Discharge::NoStatementForm,
    }
}

fn guarantee_is_discharged(guarantee: TradeGuarantee, statements: &[&Statement]) -> Discharge {
    match guarantee {
        TradeGuarantee::MinProfit => {
            // A profit guard that names a floor. Written with a ceiling it says
            // the opposite, and reading it as a floor would answer a question
            // nobody asked — the same distinction the guard's own comparison
            // exists to preserve.
            let discharged = statements.iter().any(|statement| match statement {
                Statement::Require(guard) => {
                    guard.kind == x3_lang_ast::ast::RequireKind::Profit
                        && guard.comparison.is_some_and(|op| op.is_lower_bound())
                }
                _ => false,
            });
            if discharged {
                Discharge::Yes
            } else {
                Discharge::No
            }
        }
        TradeGuarantee::Solvent => {
            let discharged = statements.iter().any(|statement| match statement {
                Statement::Require(guard) => {
                    guard.kind == x3_lang_ast::ast::RequireKind::InvariantCheck
                        && guard
                            .subject
                            .as_ref()
                            .is_some_and(|subject| subject.as_str() == "solvent")
                }
                _ => false,
            });
            if discharged {
                Discharge::Yes
            } else {
                Discharge::No
            }
        }
        TradeGuarantee::DebtClosed => Discharge::NoStatementForm,
    }
}

/// What the author has to add to discharge a guarantee. Mirrors the trading
/// verifier's wording so the same guarantee reads the same way in both places.
fn guarantee_requirement(guarantee: TradeGuarantee) -> &'static str {
    match guarantee {
        TradeGuarantee::DebtClosed => "add `require all_debts_repaid`",
        TradeGuarantee::MinProfit => "add `require profit >= <amount>`",
        TradeGuarantee::Solvent => "add `require invariant solvent == ...`",
    }
}

fn expression_to_u128(expr: &x3_lang_ast::ast::Expression) -> Option<u128> {
    match expr {
        x3_lang_ast::ast::Expression::Literal(x3_lang_ast::ast::LiteralExpr::Int { value, .. }) => Some(*value),
        _ => None,
    }
}
