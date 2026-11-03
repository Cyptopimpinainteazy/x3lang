//! Marketplace metadata — spec build-order item 25, PHASE 26.
//!
//! "A compiled strategy should expose metadata without necessarily exposing
//! source." This builds that document from the module's own declarations and the
//! bytecode the compiler produced, and it is careful about two fields in
//! particular.
//!
//! **The artifact hash is real.** It is a SHA-256 over the emitted bytecode, and
//! it names its algorithm in the value, because a hash whose algorithm is not
//! stated cannot be checked by anyone. Two builds of the same source agree, and
//! a one-character change does not.
//!
//! **Two of PHASE 26's fields cannot be filled at compile time, and the document
//! says so instead of inventing them.** A compiled artifact has no signature —
//! signing needs a key, and binding one at compile time would make the compiler
//! a signing authority — and no receipt history, because receipts accrue at run
//! time. Both are `null` in the document, with the reason listed in `notes`. A
//! marketplace that wants them has to get them from the publisher or the runtime,
//! and the metadata's job is to be clear about which is which rather than to look
//! complete.

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use x3_lang_ast::ast::{Item, LiteralExpr, Program};
use x3_lang_ast::trading::TradeEffect;

/// The document's schema version.
///
/// PHASE 26 wants metadata a marketplace can consume; a consumer that cannot
/// tell one shape of document from another cannot consume it safely, so the
/// shape is versioned from the first release.
pub const METADATA_VERSION: u32 = 1;

/// Cost budget, in basis points, below which a module is a low-risk class.
const RISK_CLASS_LOW_MAX_BPS: u32 = 50;
/// Cost budget below which a module is a moderate-risk class.
const RISK_CLASS_MODERATE_MAX_BPS: u32 = 150;

/// How risky a module is, by its own declared cost budget.
///
/// Derived rather than declared, and derived from numbers the module already
/// states, so it cannot drift from the risk profile it describes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum RiskClass {
    Low,
    Moderate,
    Elevated,
}

/// Capital requirements, in the asset's own units.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CapitalRequirements {
    pub asset: String,
    /// What the module needs. Always stated; the verifier refuses an input
    /// without an amount.
    pub minimum: u128,
    /// What it will take at most, or `None` when the module does not say.
    pub maximum: Option<u128>,
}

/// The licence terms, as a consumer needs them.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LicenseTerms {
    pub author: String,
    pub royalty_bps: u32,
    pub executions: Option<u128>,
    pub expires_block: Option<u64>,
}

/// A compiled strategy's public description.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StrategyMetadata {
    pub metadata_version: u32,
    pub strategy_id: String,
    /// `sha256:<hex>` over the emitted bytecode.
    pub artifact_hash: String,
    pub compiler_version: String,
    pub risk_class: RiskClass,
    /// Why the class is what it is, so a consumer can disagree with the
    /// thresholds rather than with a verdict it cannot see.
    pub risk_class_basis: String,
    pub supported_chains: Vec<String>,
    pub capital: Vec<CapitalRequirements>,
    pub effects: Vec<String>,
    pub required_permissions: Vec<String>,
    pub license: Option<LicenseTerms>,
    /// Always `null` today: see the module documentation.
    pub artifact_signature: Option<String>,
    /// Always empty today: receipts accrue at run time.
    pub receipt_references: Vec<String>,
    /// The things this document does not contain, and why.
    pub notes: Vec<String>,
}

/// Build the metadata for a program's first strategy module, if it has one.
///
/// `bytecode` is what the compiler emitted for that program; the hash covers it
/// exactly, so the document identifies the artifact rather than the source.
pub fn strategy_metadata(program: &Program, bytecode: &[u8]) -> Option<StrategyMetadata> {
    let module = program.items.iter().find_map(|item| match &item.node {
        Item::Strategy(strategy) => Some(strategy),
        _ => None,
    })?;

    let declared_cost_budget = module
        .risk
        .as_ref()
        .map(|risk| risk.max_slippage_bps.saturating_add(risk.max_total_fee_bps))
        .unwrap_or(0);
    let (risk_class, thresholds) = if declared_cost_budget <= RISK_CLASS_LOW_MAX_BPS {
        (RiskClass::Low, format!("at or below {RISK_CLASS_LOW_MAX_BPS} bps"))
    } else if declared_cost_budget <= RISK_CLASS_MODERATE_MAX_BPS {
        (
            RiskClass::Moderate,
            format!("at or below {RISK_CLASS_MODERATE_MAX_BPS} bps"),
        )
    } else {
        (RiskClass::Elevated, format!("above {RISK_CLASS_MODERATE_MAX_BPS} bps"))
    };

    let capital: Vec<CapitalRequirements> = module
        .inputs
        .iter()
        .filter_map(|input| {
            let minimum = input.amount.as_ref().and_then(expression_to_u128)?;
            Some(CapitalRequirements {
                asset: format!("{}.{}", input.asset.chain.as_str(), input.asset.name.as_str()),
                minimum,
                maximum: input.max_amount.as_ref().and_then(expression_to_u128),
            })
        })
        .collect();

    let mut notes = vec![
        "artifact_signature is null: signing needs a key, and binding one at compile time would \
         make the compiler a signing authority. The publisher signs, not the compiler."
            .to_string(),
        "receipt_references is empty: receipts accrue at run time, so a freshly compiled artifact \
         has no history to reference."
            .to_string(),
        "this document contains no source text: it exposes the declarations, not the program.".to_string(),
    ];
    if module.license.is_none() {
        notes.push(
            "license is null: the module declares no licence, so it is unlicensed rather than \
             licensed on unknown terms."
                .to_string(),
        );
    }
    if capital.iter().any(|requirement| requirement.maximum.is_none()) {
        notes.push(
            "a capital requirement has a null maximum: the module did not say how much it will \
             take, which is unstated rather than unbounded."
                .to_string(),
        );
    }

    Some(StrategyMetadata {
        metadata_version: METADATA_VERSION,
        strategy_id: module.name.as_str().to_string(),
        artifact_hash: format!("sha256:{}", hex(&Sha256::digest(bytecode))),
        compiler_version: env!("CARGO_PKG_VERSION").to_string(),
        risk_class,
        risk_class_basis: format!(
            "declared cost budget of {declared_cost_budget} bps (max_slippage_bps {} + \
             max_total_fee_bps {}), {thresholds}",
            module.risk.as_ref().map(|risk| risk.max_slippage_bps).unwrap_or(0),
            module.risk.as_ref().map(|risk| risk.max_total_fee_bps).unwrap_or(0)
        ),
        supported_chains: module
            .domains
            .iter()
            .map(|domain| domain.as_str().to_string())
            .collect(),
        capital,
        effects: module
            .effects
            .iter()
            .map(|effect| effect_name(*effect).to_string())
            .collect(),
        required_permissions: module
            .permissions
            .iter()
            .map(|permission| permission.as_str().to_string())
            .collect(),
        license: module.license.as_ref().map(|license| LicenseTerms {
            author: license.creator.as_str().to_string(),
            royalty_bps: license.profit_share_bps,
            executions: license.executions,
            expires_block: license.expires_block,
        }),
        artifact_signature: None,
        receipt_references: Vec::new(),
        notes,
    })
}

fn effect_name(effect: TradeEffect) -> &'static str {
    effect.as_str()
}

fn expression_to_u128(expr: &x3_lang_ast::ast::Expression) -> Option<u128> {
    match expr {
        x3_lang_ast::ast::Expression::Literal(LiteralExpr::Int { value, .. }) => Some(*value),
        _ => None,
    }
}

fn hex(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        out.push_str(&format!("{byte:02x}"));
    }
    out
}
