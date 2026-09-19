//! Static route profitability — spec PHASE 36.
//!
//! "Where feasible, the compiler should detect obviously impossible economics",
//! with the phase's own warning: *do not pretend static estimates guarantee runtime
//! profitability*.
//!
//! The comparison is between two things the program itself declares:
//!
//! - the profit floor it claims (`require profit >= N`), and
//! - the fees its own `venue` declarations state (`fee_bps`), applied to what each
//!   leg promises at minimum (`min_output`).
//!
//! A venue charges its fee on what the swap delivers, and `min_output` is the least
//! the leg may deliver, so `min_output × fee_bps / 10_000` is a *floor* on that
//! leg's fee: the leg cannot cost less than that and still satisfy its own bound.
//! When the declared floor is below the sum of those floors, the program cannot
//! satisfy its declared minimum profit under its own numbers, and the warning names
//! each part rather than asserting a verdict.
//!
//! What this is not: a quote. It knows no prices, so it can never say a route *is*
//! profitable — only that a program's own declarations cannot add up. Assets are
//! never converted: a declared fee in one asset and a floor in another are reported
//! as not compared rather than compared through an invented rate.

use x3_lang_ast::ast::{AssetRef, Expression, Item, Program, RequireGuard, Statement};
use x3_lang_common::{Span, X3Error};

/// One declared cost, and where it came from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeclaredFee {
    pub asset: String,
    pub amount: u128,
    pub because: String,
}

/// What the declarations say about a program's own floor.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Verdict {
    /// The declared fees cannot be below the declared floor's asset total.
    CannotSatisfy {
        asset: String,
        floor: u128,
        cost: u128,
        parts: Vec<DeclaredFee>,
    },
    /// The floor is at or above the declared fees — which says nothing about
    /// whether the route will really earn it.
    Satisfiable {
        asset: String,
        floor: u128,
        cost: u128,
    },
    NotAnalysed(String),
}

/// Analyse every intent's declared floor against its declared venue fees.
pub fn analyse(program: &Program) -> Verdict {
    // `venue <name> { … fee_bps N … asset_out chain.ASSET }`, by name.
    let venues: Vec<(&str, u32, String)> = program
        .items
        .iter()
        .filter_map(|item| match &item.node {
            Item::VenueDecl(venue) => Some((venue.name.as_str(), venue.fee_bps, asset_key(&venue.asset_out))),
            _ => None,
        })
        .collect();

    let mut not_analysed = Vec::new();
    for item in &program.items {
        let Item::IntentDecl(intent) = &item.node else {
            continue;
        };

        let floor = match declared_floor(&intent.body.stmts) {
            Some((amount, stated_asset)) => (amount, stated_asset),
            None => {
                not_analysed.push(format!(
                    "intent '{}' declares no `require profit >= <n>` floor",
                    intent.name.as_str()
                ));
                continue;
            }
        };
        let delivered = delivered_asset(&intent.body.stmts);

        let mut all_statements = Vec::new();
        walk(&intent.body.stmts, &mut all_statements);
        let mut parts = Vec::new();
        for statement in all_statements {
            let Statement::Swap {
                to, dex, min_output, ..
            } = statement
            else {
                continue;
            };
            let Some(venue_name) = dex.as_ref().and_then(venue_name_of) else {
                continue;
            };
            let Some((_, fee_bps, asset_out)) = venues.iter().find(|(name, _, _)| *name == venue_name) else {
                continue;
            };
            let Some(minimum) = min_output.as_ref().and_then(literal_int) else {
                continue;
            };
            let amount = minimum.saturating_mul(u128::from(*fee_bps)) / 10_000;
            parts.push(DeclaredFee {
                asset: asset_out.clone(),
                amount,
                because: format!(
                    "venue '{venue_name}' declares {fee_bps} bps, applied to the leg's minimum output \
                     of {minimum} {}",
                    asset_key(to)
                ),
            });
        }

        if parts.is_empty() {
            not_analysed.push(format!(
                "intent '{}' has no leg whose venue declares a `fee_bps` and states a literal \
                 `min_output`, so no declared cost can be compared with its floor",
                intent.name.as_str()
            ));
            continue;
        }

        // The floor is in the asset the intent delivers, unless the guard names one.
        let floor_asset = floor.1.clone().or_else(|| delivered.clone()).unwrap_or_default();
        let cost_assets: Vec<&str> = parts.iter().map(|part| part.asset.as_str()).collect();
        let same_asset = cost_assets.iter().all(|asset| *asset == floor_asset);
        if floor_asset.is_empty() || !same_asset {
            not_analysed.push(format!(
                "intent '{}' declares its floor in {} and its fees in {}; assets are not converted, so \
                 the two are not compared",
                intent.name.as_str(),
                if floor_asset.is_empty() {
                    "no asset"
                } else {
                    floor_asset.as_str()
                },
                cost_assets.join(", ")
            ));
            continue;
        }

        let cost: u128 = parts.iter().map(|part| part.amount).fold(0u128, u128::saturating_add);
        if cost > floor.0 {
            return Verdict::CannotSatisfy {
                asset: floor_asset,
                floor: floor.0,
                cost,
                parts,
            };
        }
        return Verdict::Satisfiable {
            asset: floor_asset,
            floor: floor.0,
            cost,
        };
    }

    if not_analysed.is_empty() {
        Verdict::NotAnalysed("the program declares no intent to analyse".to_string())
    } else {
        Verdict::NotAnalysed(not_analysed.join("; "))
    }
}

/// The warning a `CannotSatisfy` verdict produces, if any.
pub fn warnings(program: &Program) -> Vec<X3Error> {
    match analyse(program) {
        Verdict::CannotSatisfy {
            asset,
            floor,
            cost,
            parts,
        } => {
            let parts: Vec<String> = parts
                .iter()
                .map(|part| format!("{} ({})", part.amount, part.because))
                .collect();
            vec![X3Error::SemanticError {
                message: format!(
                    "route cannot satisfy declared minimum profit: the program requires at least \
                     {floor} {asset} but the fees its own venue declarations state already come to \
                     {cost} {asset} — {}.\nThis is a comparison of declarations, not a quote: it says \
                     the program's own numbers cannot add up, and says nothing about whether a route \
                     would really earn the floor (PHASE 36)",
                    parts.join(", ")
                ),
                span: Span::DUMMY,
            }]
        }
        _ => Vec::new(),
    }
}

/// Every statement of a body, including the ones inside blocks.
///
/// The route block is lowered to an `Atomic` statement, so a scan that only looked
/// at the intent's own statements would find no legs at all — which is exactly what
/// its first version reported ("has no leg whose venue declares a fee_bps").
fn walk<'a>(statements: &'a [Statement], out: &mut Vec<&'a Statement>) {
    for statement in statements {
        out.push(statement);
        match statement {
            Statement::Atomic(block) => walk(&block.body.stmts, out),
            Statement::If {
                then_block, else_block, ..
            } => {
                walk(&then_block.stmts, out);
                if let Some(else_block) = else_block {
                    walk(&else_block.stmts, out);
                }
            }
            Statement::While { body, .. } | Statement::For { body, .. } | Statement::Loop(body) => {
                walk(&body.stmts, out);
            }
            _ => {}
        }
    }
}

/// `require profit >= <n>`, with the asset the guard names when it names one.
fn declared_floor(statements: &[Statement]) -> Option<(u128, Option<String>)> {
    let mut all = Vec::new();
    walk(statements, &mut all);
    all.into_iter().find_map(|statement| {
        let Statement::Require(RequireGuard {
            kind,
            subject,
            comparison,
            value,
        }) = statement
        else {
            return None;
        };
        if *kind != x3_lang_ast::ast::RequireKind::Profit {
            return None;
        }
        // A ceiling is not a floor: `require profit <= 5` bounds how much the
        // program may earn, and comparing that with costs answers nothing.
        if !comparison.is_some_and(|op| op.is_lower_bound()) {
            return None;
        }
        value
            .as_ref()
            .and_then(literal_int)
            .map(|amount| (amount, subject.as_ref().map(|name| name.as_str().to_string())))
    })
}

fn delivered_asset(statements: &[Statement]) -> Option<String> {
    let mut all = Vec::new();
    walk(statements, &mut all);
    all.into_iter().find_map(|statement| match statement {
        Statement::Swap { to, .. } | Statement::Bridge { to, .. } => Some(asset_key(to)),
        _ => None,
    })
}

fn asset_key(asset: &AssetRef) -> String {
    format!("{}.{}", asset.chain.as_str(), asset.name.as_str())
}

/// The venue a leg names. The parser renders `swap <venue> …` as a string literal
/// (`LiteralExpr::String`) and a venue written elsewhere as an identifier, so both
/// spellings count as naming one.
fn venue_name_of(expression: &Expression) -> Option<&str> {
    match expression {
        Expression::Ident(name) => Some(name.as_str()),
        Expression::Literal(x3_lang_ast::ast::LiteralExpr::String(name)) => Some(name.as_str()),
        _ => None,
    }
}

fn literal_int(expression: &Expression) -> Option<u128> {
    match expression {
        Expression::Literal(x3_lang_ast::ast::LiteralExpr::Int { value, .. }) => Some(*value),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn program(source: &str) -> Program {
        crate::parser::parse_source(source).expect("the fixture must parse")
    }

    const VENUE: &str = "venue costly_pool {\n    kind pool\n    chain ethereum\n    domain evm\n    \
                         asset_in ethereum.USDC\n    asset_out ethereum.USDC\n    fee_bps 500\n    \
                         liquidity 1_000_000\n    slippage_bps 5\n    latency_ms 10\n    \
                         finality_blocks 12\n    risk 3\n    proof source_lock_proof\n}\n\n";

    fn intent(floor: u128) -> String {
        format!(
            "intent route_probe {{\n    from ethereum.USDC amount 1_000\n    to ethereum.USDC\n    \
             route {{\n        swap costly_pool ethereum.USDC -> ethereum.USDC amount 1_000 \
             min_output 1_000\n    }}\n    require profit >= {floor}\n    on_fail refund \
             ethereum.USDC to sender\n}}\n"
        )
    }

    #[test]
    fn a_floor_below_the_declared_fees_cannot_be_satisfied() {
        // 500 bps of at least 1_000 is 50: a floor of 10 is below the fee the
        // program's own venue declares.
        let verdict = analyse(&program(&format!("{VENUE}{}", intent(10))));
        let Verdict::CannotSatisfy {
            asset,
            floor,
            cost,
            parts,
        } = verdict
        else {
            panic!("expected a refusal, got {verdict:?}");
        };
        assert_eq!((asset.as_str(), floor, cost), ("ethereum.USDC", 10, 50));
        assert_eq!(parts.len(), 1, "{parts:?}");
        assert!(parts[0].because.contains("costly_pool"), "{parts:?}");
    }

    #[test]
    fn a_floor_above_the_declared_fees_is_not_warned_about() {
        let verdict = analyse(&program(&format!("{VENUE}{}", intent(500))));
        assert_eq!(
            verdict,
            Verdict::Satisfiable {
                asset: "ethereum.USDC".to_string(),
                floor: 500,
                cost: 50,
            }
        );
        assert!(
            warnings(&program(&format!("{VENUE}{}", intent(500)))).is_empty(),
            "a satisfiable floor produces no warning"
        );
    }

    #[test]
    fn the_warning_names_every_part_and_admits_what_it_is_not() {
        let program = program(&format!("{VENUE}{}", intent(10)));
        let warnings = warnings(&program);
        assert_eq!(warnings.len(), 1, "{warnings:?}");
        let message = format!("{}", warnings[0]);
        assert!(
            message.contains("route cannot satisfy declared minimum profit"),
            "{message}"
        );
        assert!(
            message.contains("10 ethereum.USDC") && message.contains("50 ethereum.USDC"),
            "{message}"
        );
        assert!(
            message.contains("not a quote") && message.contains("says nothing about whether"),
            "the warning must not claim more than it knows: {message}"
        );
    }

    #[test]
    fn assets_that_differ_are_not_compared_through_an_invented_rate() {
        let venue = VENUE.replace("asset_out ethereum.USDC", "asset_out solana.SOL");
        let verdict = analyse(&program(&format!("{venue}{}", intent(10))));
        let Verdict::NotAnalysed(reason) = verdict else {
            panic!("a cross-asset comparison must not be made: {verdict:?}");
        };
        assert!(reason.contains("assets are not converted"), "{reason}");
    }

    #[test]
    fn a_ceiling_is_not_read_as_a_floor() {
        let source = format!(
            "{VENUE}{}",
            intent(10).replace("require profit >= 10", "require profit <= 10")
        );
        let verdict = analyse(&program(&source));
        assert!(
            matches!(verdict, Verdict::NotAnalysed(_)),
            "`require profit <= 10` bounds earnings, it does not claim a floor: {verdict:?}"
        );
    }
}
