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
//! Both sides are taken at the value the program itself commits to: the input it
//! spends (`amount`), the least it accepts back (`min_output`) and the fee the venue
//! declares on that output. `net at the declared minimum = min_output − amount −
//! fees`, and the program is warned when its own floor is above that. The figure is
//! a *minimum*, not a prediction: a route that delivers more than `min_output` can
//! net more, which is why this is a warning about declarations and never a claim
//! that the route is unprofitable.
//!
//! A floor compared against the fee *alone* would be unsound — a large gross can
//! absorb a large fee — so the net is what is compared.
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
    /// The fee the venue's declaration implies on the leg's minimum output.
    pub amount: u128,
    /// What the leg nets at that same minimum: `min_output − spent − fee`, saturating
    /// at zero because a negative net is reported as zero here and shown in the
    /// sentence.
    pub net_at_minimum: u128,
    pub minimum_output: u128,
    pub spent: u128,
    pub because: String,
}

/// What the declarations say about a program's own floor.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Verdict {
    /// At the output the program itself promises at minimum, its declared costs
    /// leave less than the floor it claims.
    CannotSatisfy {
        asset: String,
        floor: u128,
        /// The net at the declared minimum: `min_output − spent − fees`.
        net_at_minimum: u128,
        cost: u128,
        parts: Vec<DeclaredFee>,
    },
    /// The floor is at or below the net the leg's own declared minimum leaves —
    /// which says nothing about whether the route will really earn it.
    Satisfiable {
        asset: String,
        floor: u128,
        net_at_minimum: u128,
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
                from,
                to,
                route,
                dex,
                min_output,
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
            // The step's input amount. The parser stores it in `Statement::Swap`'s
            // `route` field, whose name is wrong — the field holds `amount <expr>`
            // (see the ticket on the field) — so it is read here with that said out
            // loud rather than silently.
            let Some(spent) = route.as_ref().and_then(literal_int) else {
                continue;
            };
            // Same-asset legs only: comparing what is spent with what is returned
            // needs one asset, and prices are a host fact.
            if asset_key(to) != asset_key(from) {
                continue;
            }
            let fee = minimum.saturating_mul(u128::from(*fee_bps)) / 10_000;
            let gross = minimum.saturating_sub(spent);
            let net = gross.saturating_sub(fee);
            parts.push(DeclaredFee {
                asset: asset_out.clone(),
                amount: fee,
                net_at_minimum: net,
                minimum_output: minimum,
                spent,
                because: format!(
                    "venue '{venue_name}' declares {fee_bps} bps on the leg's minimum output of \
                     {minimum} {}: after spending {spent} and paying at least {fee}, the leg nets \
                     {net}",
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
        // The *net* at the declared minimum is what the floor is about: comparing
        // the floor with the fee alone would warn about a route whose gross absorbs
        // it, which is the false positive this check must not produce.
        let net_at_minimum: u128 = parts.iter().map(|part| part.net_at_minimum).min().unwrap_or(0);
        if net_at_minimum < floor.0 {
            return Verdict::CannotSatisfy {
                asset: floor_asset,
                floor: floor.0,
                net_at_minimum,
                cost,
                parts,
            };
        }
        return Verdict::Satisfiable {
            asset: floor_asset,
            floor: floor.0,
            net_at_minimum,
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
            net_at_minimum,
            cost,
            parts,
        } => {
            let parts: Vec<String> = parts
                .iter()
                .map(|part| format!("{} ({})", part.amount, part.because))
                .collect();
            vec![X3Error::SemanticError {
                message: format!(
                    "route cannot satisfy its declared minimum profit at the output it itself \
                     promises: the program requires at least {floor} {asset}, and at the least its \
                     legs may deliver it nets {net_at_minimum} {asset} after {cost} {asset} of fees — \
                     {}.\nThis is a comparison of the program's own declarations, not a quote: a route \
                     that delivers more than its minimum can net more, and nothing here says whether \
                     one will (PHASE 36)",
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

    fn intent(spent: u128, min_output: u128, floor: u128) -> String {
        format!(
            "intent route_probe {{\n    from ethereum.USDC amount {spent}\n    to ethereum.USDC\n    \
             route {{\n        swap costly_pool ethereum.USDC -> ethereum.USDC amount {spent} \
             min_output {min_output}\n    }}\n    require profit >= {floor}\n    on_fail refund \
             ethereum.USDC to sender\n}}\n"
        )
    }

    #[test]
    fn a_floor_above_what_the_declared_minimum_nets_cannot_be_satisfied() {
        // Spending 1_000 and accepting at least 1_000 back, minus 500 bps of that
        // 1_000 (50), nets nothing at the leg's own minimum: a floor of 10 is above it.
        let verdict = analyse(&program(&format!("{VENUE}{}", intent(1_000, 1_000, 10))));
        let Verdict::CannotSatisfy {
            asset,
            floor,
            net_at_minimum,
            cost,
            parts,
        } = verdict
        else {
            panic!("expected a refusal, got {verdict:?}");
        };
        assert_eq!(
            (asset.as_str(), floor, net_at_minimum, cost),
            ("ethereum.USDC", 10, 0, 50)
        );
        assert_eq!(parts.len(), 1, "{parts:?}");
        assert!(parts[0].because.contains("costly_pool"), "{parts:?}");
    }

    #[test]
    fn a_floor_at_or_below_what_the_declared_minimum_nets_is_not_warned_about() {
        // 2_000 − 1_000 − 100 of fees nets 900, so a floor of 500 holds.
        let program = program(&format!("{VENUE}{}", intent(1_000, 2_000, 500)));
        assert_eq!(
            analyse(&program),
            Verdict::Satisfiable {
                asset: "ethereum.USDC".to_string(),
                floor: 500,
                net_at_minimum: 900,
                cost: 100,
            }
        );
        assert!(warnings(&program).is_empty(), "a satisfiable floor produces no warning");
    }

    #[test]
    fn a_fee_larger_than_the_floor_is_not_enough_to_warn() {
        // The soundness property, and the reason the net is compared rather than the
        // fee: 500 bps of a 20_000 minimum is a 1_000 fee — above the 900 floor — but
        // the leg still nets 18_000 at its own minimum, so its declared numbers add
        // up. Comparing the floor with the fee alone would warn here.
        let program = program(&format!("{VENUE}{}", intent(1_000, 20_000, 900)));
        let Verdict::Satisfiable {
            net_at_minimum, cost, ..
        } = analyse(&program)
        else {
            panic!("a large gross absorbs the fee: {:?}", analyse(&program));
        };
        assert_eq!((cost, net_at_minimum), (1_000, 18_000));
        assert!(
            warnings(&program).is_empty(),
            "a route whose minimum nets far above its floor must not be warned about"
        );
    }

    #[test]
    fn the_warning_names_the_net_the_floor_and_the_fee_and_admits_what_it_is_not() {
        let program = program(&format!("{VENUE}{}", intent(1_000, 2_000, 1_000)));
        let warnings = warnings(&program);
        assert_eq!(warnings.len(), 1, "{warnings:?}");
        let message = format!("{}", warnings[0]);
        assert!(
            message.contains("promises"),
            "the claim is about the declared minimum: {message}"
        );
        assert!(message.contains("at least 1000 ethereum.USDC"), "{message}");
        assert!(message.contains("nets 900 ethereum.USDC"), "{message}");
        assert!(message.contains("100 ethereum.USDC of fees"), "{message}");
        assert!(
            message.contains("not a quote") && message.contains("can net more"),
            "the warning must not claim more than it knows: {message}"
        );
    }

    #[test]
    fn assets_that_differ_are_not_compared_through_an_invented_rate() {
        let venue = VENUE.replace("asset_out ethereum.USDC", "asset_out solana.SOL");
        let verdict = analyse(&program(&format!("{venue}{}", intent(1_000, 2_000, 10))));
        let Verdict::NotAnalysed(reason) = verdict else {
            panic!("a cross-asset comparison must not be made: {verdict:?}");
        };
        assert!(reason.contains("assets are not converted"), "{reason}");
    }

    #[test]
    fn a_leg_that_crosses_assets_is_not_compared_with_a_same_asset_floor() {
        // `swap costly_pool ethereum.USDC -> ethereum.ETH`: what is spent and what is
        // returned are different assets, so the net is not a number this compiler can
        // compute without a price.
        let cross = intent(1_000, 2_000, 10).replace(
            "swap costly_pool ethereum.USDC -> ethereum.USDC",
            "swap costly_pool ethereum.USDC -> ethereum.ETH",
        );
        let verdict = analyse(&program(&format!("{VENUE}{cross}")));
        let Verdict::NotAnalysed(reason) = verdict else {
            panic!("a cross-asset leg must not be compared: {verdict:?}");
        };
        assert!(reason.contains("no leg whose venue declares"), "{reason}");
    }

    #[test]
    fn a_ceiling_is_not_read_as_a_floor() {
        let source = format!(
            "{VENUE}{}",
            intent(1_000, 1_000, 10).replace("require profit >= 10", "require profit <= 10")
        );
        let verdict = analyse(&program(&source));
        assert!(
            matches!(verdict, Verdict::NotAnalysed(_)),
            "`require profit <= 10` bounds earnings, it does not claim a floor: {verdict:?}"
        );
    }
}
