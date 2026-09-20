//! Portfolio rebalancing — spec PHASE 11.
//!
//! `rebalance portfolio { BTC = 40%; ETH = 25%; … minimize { fees; … } atomic; }` is a
//! target portfolio plus the things the plan would minimise. Two properties are
//! decidable from the declaration itself:
//!
//! - **the weights are a portfolio**: at least two assets, none of them zero, no
//!   asset named twice, and the weights sum to exactly 100%. A declaration that sums
//!   to 95% is not a portfolio — it is 5% of nothing — and it is refused with the
//!   figures;
//! - **every `minimize` target is one the optimizer can rank.** The vocabulary is the
//!   objective's (`objective::criterion_for`), so the reason a target cannot be used
//!   is stated once, in that module, rather than twice with two different wordings.
//!   `external_liquidity` is the case the phase's own example writes: the graph does
//!   not distinguish liquidity a ring supplies from liquidity it has to borrow, so
//!   there is nothing to minimise and the target is refused — recording it would be a
//!   label nothing acts on, which is the defect PHASE 15's own comment names.
//!
//! The phase's last sentence — "compiler should *eventually* be able to generate the
//! transaction graph automatically" — is not implemented, so a declaration is decided
//! but no artifact is emitted for it (TICKET-070). The targets travel in
//! `Operation::Rebalance`, which is where the decided portfolio is visible today.
//!
//! The `minimize` set is ordered and kept as written: the optimizer ranks *one*
//! metric (PHASE 15: "multi-objective optimization must be deterministic"), so the
//! first target is the criterion a generated plan would be ranked by and the rest are
//! recorded. Combining them is not implemented, and the module does not pretend
//! otherwise.

use x3_lang_ast::ast::{Item, ObjectiveMetric, Program, RebalanceDecl};
use x3_lang_common::{ErrorAccumulator, Span, X3Error};

/// A portfolio whose weights add up, with the assets and the criterion.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Portfolio {
    pub name: String,
    /// `chain.ASSET` and what is held of it now, in the asset's own units, in the order
    /// written. Empty means the program stated no holdings, which is a different fact from
    /// holding nothing and is why the artifact says which it is.
    pub holdings: Vec<(String, u128)>,
    /// `chain.ASSET` and its weight in percent, in the order written.
    pub weights: Vec<(String, u32)>,
    /// The metric a generated plan would be ranked by: the first target.
    pub criterion: ObjectiveMetric,
    /// Every target, in the order written.
    pub minimize: Vec<ObjectiveMetric>,
}

impl Portfolio {
    /// The sum of the weights — 100 for a valid portfolio.
    pub fn total_percent(&self) -> u32 {
        self.weights.iter().map(|(_, percent)| *percent).sum()
    }
}

/// Decide a rebalance declaration from its own numbers.
pub fn portfolio(decl: &RebalanceDecl) -> Result<Portfolio, String> {
    if decl.weights.len() < 2 {
        return Err(format!(
            "the rebalance '{}' names {} asset(s); a portfolio is at least two, and one asset with a \
             weight is an allocation rather than a rebalance",
            decl.name.as_str(),
            decl.weights.len()
        ));
    }
    // A holding named twice is the same defect as a weight named twice: the second would
    // replace the first and one of them would not be in force.
    let mut held: Vec<String> = Vec::new();
    for (asset, _amount) in &decl.holdings {
        let key = format!("{}.{}", asset.chain.as_str(), asset.name.as_str());
        if held.contains(&key) {
            return Err(format!(
                "the rebalance '{}' states what it holds of '{key}' twice; the second amount would \
                 replace the first, so one of them would not be in force",
                decl.name.as_str()
            ));
        }
        held.push(key);
    }
    let mut seen: Vec<String> = Vec::new();
    for (asset, percent) in &decl.weights {
        let key = format!("{}.{}", asset.chain.as_str(), asset.name.as_str());
        if seen.contains(&key) {
            return Err(format!(
                "the rebalance '{}' names '{key}' twice; the second weight would replace the first, \
                 so one of them would not be in force",
                decl.name.as_str()
            ));
        }
        if *percent == 0 {
            return Err(format!(
                "the rebalance '{}' gives '{key}' a weight of zero; a target of nothing is not a \
                 target — leave the asset out instead",
                decl.name.as_str()
            ));
        }
        if *percent > 100 {
            return Err(format!(
                "the rebalance '{}' gives '{key}' {percent}%, which is more than the whole portfolio",
                decl.name.as_str()
            ));
        }
        seen.push(key);
    }
    let weights: Vec<(String, u32)> = decl
        .weights
        .iter()
        .map(|(asset, percent)| (format!("{}.{}", asset.chain.as_str(), asset.name.as_str()), *percent))
        .collect();
    let total: u32 = weights.iter().map(|(_, percent)| *percent).sum();
    if total != 100 {
        let listed: Vec<String> = weights
            .iter()
            .map(|(asset, percent)| format!("{asset} {percent}%"))
            .collect();
        return Err(format!(
            "the rebalance '{}' sums to {total}%, not 100%: {} — the remainder is a position nobody \
             declared",
            decl.name.as_str(),
            listed.join(", ")
        ));
    }

    if decl.minimize.is_empty() {
        return Err(format!(
            "the rebalance '{}' says nothing to minimize; a rebalance without a criterion could be \
             executed in any way at all, and the phase asks for the plan's targets to be stated",
            decl.name.as_str()
        ));
    }
    for metric in &decl.minimize {
        // The reason a metric cannot be ranked is stated once, in the objective
        // module, and reused here: two wordings of one rule drift.
        if let Err(reason) = crate::objective::criterion_for(*metric) {
            return Err(format!(
                "the rebalance '{}' asks to minimize {}, which this compiler cannot rank: {reason}",
                decl.name.as_str(),
                metric.name()
            ));
        }
    }

    Ok(Portfolio {
        name: decl.name.as_str().to_string(),
        holdings: decl
            .holdings
            .iter()
            .map(|(asset, amount)| (format!("{}.{}", asset.chain.as_str(), asset.name.as_str()), *amount))
            .collect(),
        weights,
        criterion: decl.minimize[0],
        minimize: decl.minimize.clone(),
    })
}

/// Verify every rebalance in a program.
pub fn verify(program: &Program, acc: &mut ErrorAccumulator) {
    for item in &program.items {
        let Item::Rebalance(decl) = &item.node else {
            continue;
        };
        if let Err(reason) = portfolio(decl) {
            acc.add_error(err(reason));
        }
    }
}

fn err(message: impl Into<String>) -> X3Error {
    X3Error::SemanticError {
        message: message.into(),
        span: Span::DUMMY,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SOUND: &str = "rebalance portfolio {\n    BTC = 40%;\n    ETH = 25%;\n    SOL = 15%;\n    \
                         X3 = 10%;\n    USDC = 10%;\n\n    minimize {\n        fees;\n        \
                         slippage;\n    }\n\n    atomic;\n}\n";

    fn analysed(source: &str) -> Result<Portfolio, String> {
        let program = crate::parser::parse_source(source).expect("the fixture must parse");
        let Item::Rebalance(decl) = &program.items[0].node else {
            panic!("the fixture is a rebalance");
        };
        portfolio(decl)
    }

    #[test]
    fn a_portfolio_that_adds_up_reports_its_targets_and_criterion() {
        let portfolio = analysed(SOUND).expect("the fixture is a portfolio");
        assert_eq!(portfolio.name, "portfolio");
        assert_eq!(portfolio.total_percent(), 100);
        assert_eq!(portfolio.weights.len(), 5);
        assert_eq!(portfolio.criterion, ObjectiveMetric::MinimizeFees);
        assert_eq!(
            portfolio.minimize,
            vec![ObjectiveMetric::MinimizeFees, ObjectiveMetric::MinimizeSlippage]
        );
    }

    #[test]
    fn weights_that_do_not_sum_to_a_whole_are_refused_with_the_figures() {
        let source = SOUND.replace("SOL = 15%", "SOL = 5%");
        let reason = analysed(&source).expect_err("90% is not a portfolio");
        assert!(
            reason.contains("sums to 90%, not 100%") && reason.contains("BTC 40%") && reason.contains("USDC 10%"),
            "{reason}"
        );
    }

    #[test]
    fn a_zero_weight_and_a_duplicate_asset_are_refused() {
        let zero = SOUND.replace("USDC = 10%", "USDC = 0%");
        assert!(
            analysed(&zero)
                .expect_err("a zero weight is not a target")
                .contains("a target of nothing is not a target"),
            "{zero}"
        );

        let duplicate = SOUND.replace("ETH = 25%", "BTC = 25%");
        assert!(
            analysed(&duplicate)
                .expect_err("the same asset twice")
                .contains("names 'unknown.BTC' twice"),
            "{duplicate}"
        );
    }

    #[test]
    fn one_asset_is_an_allocation_rather_than_a_rebalance() {
        let source = "rebalance only {\n    BTC = 100%;\n    minimize {\n        fees;\n    }\n    \
                      atomic;\n}\n";
        assert!(
            analysed(source)
                .expect_err("one asset")
                .contains("a portfolio is at least two"),
            "{source}"
        );
    }

    #[test]
    fn a_target_the_optimizer_cannot_rank_is_refused_with_the_objectives_reason() {
        // The phase's own example writes `external_liquidity_usage`; the metric the
        // optimizer knows is `external_liquidity`, and it cannot rank even that one.
        let unknown = SOUND.replace("        fees;", "        external_liquidity_usage;");
        let program = crate::parser::parse_source(&unknown);
        let message = format!("{}", program.expect_err("unknown target"));
        assert!(
            message.contains("unknown 'minimize' target 'external_liquidity_usage'")
                && message.contains("minimize external_liquidity"),
            "{message}"
        );

        let unrankable = SOUND.replace("        fees;", "        external_liquidity;");
        let reason = analysed(&unrankable).expect_err("the graph cannot rank it");
        assert!(
            reason.contains("cannot rank") && reason.contains("does not distinguish liquidity the ring supplies"),
            "{reason}"
        );
    }

    #[test]
    fn a_rebalance_without_atomic_is_refused() {
        let source = SOUND.replace("    atomic;\n", "");
        let message = format!(
            "{}",
            crate::parser::parse_source(&source).expect_err("a rebalance has to be atomic")
        );
        assert!(
            message.contains("has to say `atomic;`") && message.contains("off-target"),
            "{message}"
        );
    }

    #[test]
    fn a_rebalance_without_a_criterion_is_refused() {
        let source = SOUND.replace("    minimize {\n        fees;\n        slippage;\n    }\n", "");
        assert!(
            analysed(&source)
                .expect_err("no criterion")
                .contains("says nothing to minimize"),
            "{source}"
        );
    }

    #[test]
    fn a_fractional_weight_is_refused_rather_than_rounded() {
        let source = SOUND.replace("BTC = 40%", "BTC = 39.5%");
        let message = format!(
            "{}",
            crate::parser::parse_source(&source).expect_err("fractional weights are not expressible")
        );
        assert!(
            message.contains("fractional") && message.contains("not representable"),
            "{message}"
        );
    }
}
