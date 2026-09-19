//! Versioned JSON boundary for validated intents emitted by the Python MVP.
//!
//! The Python validator remains responsible for parsing the user-facing DSL.
//! Rust must still validate the boundary before converting it into the
//! compiler's canonical intent draft, so untrusted JSON cannot bypass the
//! semantic path.

use serde::{Deserialize, Serialize};
use serde_json::Value;
use x3_lang_common::X3Error;

use crate::intent_emit::{IntentSpecDraft, SourceConstraint};
use crate::ir::{FailureAction, Operation, RequireKind, X3IR};

pub const VALIDATED_INTENT_SCHEMA_VERSION: u32 = 1;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ValidatedIntentV1 {
    pub schema_version: u32,
    pub intent: String,
    pub from: Endpoint,
    pub to: Endpoint,
    #[serde(default)]
    pub path: Vec<RouteStep>,
    #[serde(default)]
    pub requires: Vec<Requirement>,
    #[serde(default)]
    pub policies: Policies,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Endpoint {
    pub chain: String,
    pub asset: String,
    pub amount: Option<Value>,
    pub receiver: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct RouteStep {
    #[serde(rename = "type")]
    pub step_type: String,
    #[serde(flatten)]
    pub fields: std::collections::BTreeMap<String, Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Requirement {
    pub kind: String,
    #[serde(default)]
    pub chain: Option<String>,
    #[serde(default)]
    pub op: Option<String>,
    #[serde(default)]
    pub value: Option<Value>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct Policies {
    #[serde(default)]
    pub timeout: Option<TimeoutPolicy>,
    #[serde(default)]
    pub on_fail: Option<Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct TimeoutPolicy {
    pub duration: String,
    #[serde(default)]
    pub action: Option<Value>,
}

/// Parse and validate the exact envelope emitted by Python after typechecking.
pub fn parse_validated_intent_json(input: &str) -> Result<ValidatedIntentV1, X3Error> {
    let intent: ValidatedIntentV1 = serde_json::from_str(input).map_err(|error| X3Error::ParseError {
        message: format!("invalid validated intent JSON: {error}"),
        span: Default::default(),
        expected: vec!["validated intent schema v1".to_string()],
        found: input.to_string(),
    })?;
    validate_validated_intent(&intent)?;
    Ok(intent)
}

pub fn validate_validated_intent(intent: &ValidatedIntentV1) -> Result<(), X3Error> {
    if intent.schema_version != VALIDATED_INTENT_SCHEMA_VERSION {
        return Err(semantic_error(format!(
            "unsupported validated intent schema version {}",
            intent.schema_version
        )));
    }
    non_empty("intent", &intent.intent)?;
    validate_endpoint("from", &intent.from, true)?;
    validate_endpoint("to", &intent.to, false)?;
    if intent.path.is_empty() {
        return Err(semantic_error("validated intent path must not be empty"));
    }
    for (index, step) in intent.path.iter().enumerate() {
        if step.step_type != "swap" && step.step_type != "bridge" {
            return Err(semantic_error(format!(
                "path[{index}] has unsupported step type {}",
                step.step_type
            )));
        }
    }
    for requirement in &intent.requires {
        non_empty("requirement.kind", &requirement.kind)?;
        if let Some(value) = &requirement.value {
            if value.is_null() {
                return Err(semantic_error("requirement.value must not be null"));
            }
        }
    }
    if let Some(timeout) = &intent.policies.timeout {
        non_empty("policies.timeout.duration", &timeout.duration)?;
    }
    Ok(())
}

/// Convert the validated Python envelope to the compiler's canonical draft.
pub fn to_intent_spec_draft(intent: &ValidatedIntentV1) -> Result<IntentSpecDraft, X3Error> {
    validate_validated_intent(intent)?;
    let source_amount = required_amount("from.amount", intent.from.amount.as_ref())?;
    let mut draft = IntentSpecDraft::new(
        &intent.intent,
        &intent.from.chain,
        &intent.from.asset,
        source_amount,
        intent.from.receiver.as_deref().unwrap_or("unknown"),
        &intent.to.chain,
        &intent.to.asset,
        intent.to.receiver.as_deref().unwrap_or("unknown"),
    );
    for requirement in &intent.requires {
        let value = requirement.value.as_ref().map(value_to_string).unwrap_or_default();
        draft = draft.with_constraint(&requirement.kind, value);
    }
    if let Some(timeout) = &intent.policies.timeout {
        let seconds = IntentSpecDraft::timeout_from_arg(&timeout.duration)
            .ok_or_else(|| semantic_error("invalid timeout duration"))?;
        draft = draft.with_timeout_secs(seconds);
    }
    Ok(draft)
}

/// Seconds per block assumed for the production chains (matches the 6s/block
/// convention used by `lowering.rs`'s `MAX_TIMEOUT_BLOCKS`).
const SECONDS_PER_BLOCK: u64 = 6;

/// Lower a validated Python envelope directly into X3IR, mirroring the
/// AST -> IR lowering in `lowering.rs` for `Item::AtomicSwap`/`Item::Bridge`.
///
/// Unlike [`to_intent_spec_draft`], this retains the full multi-hop
/// `path[]` route (swap/bridge steps), which `IntentSpecDraft` cannot
/// represent, so it is the correct entry point for full VM execution.
pub fn to_ir(intent: &ValidatedIntentV1) -> Result<X3IR, X3Error> {
    validate_validated_intent(intent)?;

    let mut ir = X3IR::new();

    // Surface any `nonce` requirement into program metadata so the
    // replay-protection semantic check (`verify_replay_and_expiry`) sees it,
    // matching how `lowering.rs` handles `Statement::Require` with
    // `RequireKind::Nonce`.
    for requirement in &intent.requires {
        if requirement.kind.eq_ignore_ascii_case("nonce") {
            if let Some(value) = &requirement.value {
                ir.metadata.nonce = Some(value_to_string(value));
            }
        }
    }

    // Matching `lowering.rs`'s AST -> IR pattern (see `Item::Bridge` /
    // `Item::AtomicSwap`): the endpoint `Lock`/`Release` sit OUTSIDE the
    // atomic block, and so do `Require`/`OnFail`/`OnTimeout`. Only the
    // cross-chain `Swap`/`Bridge` route steps belong inside
    // `AtomicBegin`/`AtomicEnd`.
    let source_amount = required_amount("from.amount", intent.from.amount.as_ref())?;
    ir.push(Operation::Lock {
        chain: intent.from.chain.to_ascii_lowercase(),
        asset: intent.from.asset.clone(),
        amount: source_amount,
        from: "sender".to_string(),
    });
    ir.push(Operation::Release {
        chain: intent.to.chain.to_ascii_lowercase(),
        asset: intent.to.asset.clone(),
        to: intent.to.receiver.clone().unwrap_or_else(|| "receiver".to_string()),
    });

    ir.push(Operation::AtomicBegin);

    let mut running_amount = source_amount;
    for (index, step) in intent.path.iter().enumerate() {
        match step.step_type.as_str() {
            "swap" => {
                let from_chain = field_nested_chain(step, "from_ref")
                    .or_else(|| field_string(step, "from_chain"))
                    .or_else(|| field_string(step, "chain"))
                    .unwrap_or_else(|| intent.from.chain.to_ascii_lowercase());
                let from_asset = field_string(step, "from")
                    .ok_or_else(|| semantic_error(format!("path[{index}] swap missing 'from' asset")))?;
                let to_asset = field_string(step, "to")
                    .ok_or_else(|| semantic_error(format!("path[{index}] swap missing 'to' asset")))?;
                // A step may omit `amount` when it consumes the output of the
                // previous step; carry the running amount forward in that case.
                let input_amount = field_amount(step, "amount").unwrap_or(running_amount);
                let min_output = field_amount(step, "min_output").unwrap_or(0);
                let dex = field_string(step, "dex");
                // `Operation::Swap` carries the destination chain explicitly
                // (`ethereum.DAI -> solana.SOL` is a cross-chain swap, and a
                // reader that only knew `from_chain` could not tell). Same
                // derivation as the bridge step below.
                let to_chain = field_nested_chain(step, "to_ref")
                    .or_else(|| field_string(step, "to_chain"))
                    .unwrap_or_else(|| intent.to.chain.to_ascii_lowercase());
                running_amount = input_amount;
                ir.push(Operation::Swap {
                    from_chain: from_chain.to_ascii_lowercase(),
                    from_asset,
                    to_chain: to_chain.to_ascii_lowercase(),
                    to_asset,
                    input_amount,
                    min_output,
                    dex,
                });
            }
            "bridge" => {
                let via = field_string(step, "via")
                    .ok_or_else(|| semantic_error(format!("path[{index}] bridge missing 'via'")))?;
                let from_chain = field_nested_chain(step, "from_ref")
                    .or_else(|| field_string(step, "from_chain"))
                    .or_else(|| field_string(step, "chain"))
                    .unwrap_or_else(|| intent.from.chain.to_ascii_lowercase());
                let to_chain = field_nested_chain(step, "to_ref")
                    .or_else(|| field_string(step, "to_chain"))
                    .unwrap_or_else(|| intent.to.chain.to_ascii_lowercase());
                let asset = field_string(step, "asset")
                    .unwrap_or_else(|| intent.from.asset.clone());
                let to_asset = field_string(step, "to_asset").unwrap_or_else(|| asset.clone());
                let amount = field_amount(step, "amount").unwrap_or(running_amount);
                let receiver = field_string(step, "receiver")
                    .or_else(|| intent.to.receiver.clone())
                    .unwrap_or_else(|| "receiver".to_string());
                running_amount = amount;
                ir.push(Operation::Bridge {
                    via: via.to_ascii_lowercase(),
                    from_chain: from_chain.to_ascii_lowercase(),
                    from_asset: asset,
                    to_chain: to_chain.to_ascii_lowercase(),
                    to_asset,
                    amount,
                    receiver,
                    source_finality_proof: Vec::new(),
                    transfer_proof: Vec::new(),
                });
            }
            other => {
                return Err(semantic_error(format!("path[{index}] has unsupported step type {other}")));
            }
        }
    }

    ir.push(Operation::AtomicEnd);

    // Require guards (nonce excluded — surfaced via `ir.metadata.nonce`
    // above since it carries a replay-protection token, not a boolean
    // condition with a meaningful truth value) sit AFTER the atomic block,
    // matching `lowering.rs`'s `Statement::Require` placement.
    for requirement in &intent.requires {
        if requirement.kind.eq_ignore_ascii_case("nonce") {
            continue;
        }
        ir.push(Operation::Require {
            kind: requirement_kind_to_ir(&requirement.kind),
            subject: requirement.chain.clone(),
            condition: requirement_to_condition(requirement),
            error_msg: None,
            comparison: requirement_comparison(requirement),
        });
    }

    if let Some(on_fail) = &intent.policies.on_fail {
        let action = value_to_failure_action(on_fail).unwrap_or_else(|| default_refund_action(intent));
        ir.push(Operation::OnFail { action });
    }

    if let Some(timeout) = &intent.policies.timeout {
        let seconds = IntentSpecDraft::timeout_from_arg(&timeout.duration)
            .ok_or_else(|| semantic_error("invalid timeout duration"))?;
        let duration_blocks = seconds_to_blocks(seconds);
        let action = timeout
            .action
            .as_ref()
            .and_then(value_to_failure_action)
            .unwrap_or_else(|| default_refund_action(intent));
        ir.push(Operation::OnTimeout {
            duration_blocks,
            action: action.clone(),
        });
        // Mirror `lowering.rs`'s `Statement::OnTimeout` handling: a refund
        // failure action also emits a concrete `Release` op so the refund
        // is actually executed by the VM, not just recorded as metadata.
        if let FailureAction::Refund { chain, asset, to } = action {
            ir.push(Operation::Release { chain, asset, to });
        }
    }

    Ok(ir)
}

fn seconds_to_blocks(seconds: u64) -> u32 {
    let blocks = seconds.div_ceil(SECONDS_PER_BLOCK).max(1);
    blocks.min(u32::MAX as u64) as u32
}

fn default_refund_action(intent: &ValidatedIntentV1) -> FailureAction {
    FailureAction::Refund {
        chain: intent.from.chain.to_ascii_lowercase(),
        asset: intent.from.asset.clone(),
        to: intent.from.receiver.clone().unwrap_or_else(|| "sender".to_string()),
    }
}

fn value_to_failure_action(value: &Value) -> Option<FailureAction> {
    let obj = value.as_object()?;
    match obj.get("type").and_then(Value::as_str) {
        Some("refund") => Some(FailureAction::Refund {
            chain: obj.get("chain").and_then(Value::as_str)?.to_ascii_lowercase(),
            asset: obj.get("asset").and_then(Value::as_str)?.to_string(),
            to: obj.get("to").and_then(Value::as_str).unwrap_or("sender").to_string(),
        }),
        Some("rollback") => Some(FailureAction::Rollback),
        Some("halt") => Some(FailureAction::Halt),
        Some("quarantine") => Some(FailureAction::Quarantine),
        _ => None,
    }
}

fn requirement_kind_to_ir(kind: &str) -> RequireKind {
    match kind.to_ascii_lowercase().as_str() {
        "canonical_supply" => RequireKind::CanonicalSupply,
        "nonce" => RequireKind::NonceUnused,
        "bridge_liquidity" => RequireKind::BridgeLiquidity,
        "slippage" => RequireKind::SlippageTolerance,
        "profit" => RequireKind::ProfitThreshold,
        "finality" => RequireKind::Finality,
        "route_score" => RequireKind::RouteScore,
        "solver_bond" => RequireKind::SolverBond,
        "relayer_quorum" => RequireKind::RelayerQuorum,
        "proof" | "proof_complete" => RequireKind::ProofComplete,
        "refund_path" => RequireKind::RefundPath,
        "finality_explicit" => RequireKind::FinalityExplicit,
        "vm_supported" => RequireKind::VmSupported,
        "mainnet_safe" => RequireKind::MainnetSafe,
        other => RequireKind::Custom(other.to_string()),
    }
}

fn requirement_to_condition(requirement: &Requirement) -> crate::ir::Condition {
    let op = requirement.op.as_deref().unwrap_or(">=");
    let value = requirement
        .value
        .as_ref()
        .map(value_to_string)
        .unwrap_or_default();
    crate::ir::Condition::Expression {
        expr: format!("{op} {value}").trim().to_string(),
    }
}

/// The IR carries the comparison a guard makes, because `slippage <= 50` and
/// `slippage >= 50` are opposite claims and a check that reads one as the other
/// is reading a direction nobody wrote. The intent JSON states the operator as a
/// string; map it onto the same enum the parsed form uses so intent-built guards
/// get the same direction checks as hand-written `.x3` ones.
fn requirement_comparison(requirement: &Requirement) -> Option<crate::ir::ComparisonOp> {
    use crate::ir::ComparisonOp;
    match requirement.op.as_deref()?.trim() {
        "<" => Some(ComparisonOp::Less),
        "<=" => Some(ComparisonOp::LessOrEqual),
        ">" => Some(ComparisonOp::Greater),
        ">=" => Some(ComparisonOp::GreaterOrEqual),
        "==" | "=" => Some(ComparisonOp::Equal),
        "!=" => Some(ComparisonOp::NotEqual),
        _ => None,
    }
}

fn field_string(step: &RouteStep, key: &str) -> Option<String> {
    step.fields.get(key).map(value_to_string)
}

fn field_nested_chain(step: &RouteStep, key: &str) -> Option<String> {
    step.fields
        .get(key)?
        .as_object()?
        .get("chain")
        .and_then(Value::as_str)
        .map(str::to_string)
}

fn field_amount(step: &RouteStep, key: &str) -> Option<u128> {
    let value = step.fields.get(key)?;
    let text = value_to_string(value);
    text.parse::<f64>().ok().map(|n| n as u128).or_else(|| text.parse::<u128>().ok())
}

fn validate_endpoint(name: &str, endpoint: &Endpoint, amount_required: bool) -> Result<(), X3Error> {
    non_empty(&format!("{name}.chain"), &endpoint.chain)?;
    non_empty(&format!("{name}.asset"), &endpoint.asset)?;
    if amount_required {
        required_amount(&format!("{name}.amount"), endpoint.amount.as_ref())?;
    }
    if let Some(receiver) = &endpoint.receiver {
        non_empty(&format!("{name}.receiver"), receiver)?;
    }
    Ok(())
}

fn required_amount(name: &str, value: Option<&Value>) -> Result<u128, X3Error> {
    let value = value.ok_or_else(|| semantic_error(format!("{name} is required")))?;
    let text = match value {
        Value::String(value) => value.clone(),
        Value::Number(value) => value.to_string(),
        _ => return Err(semantic_error(format!("{name} must be a positive integer"))),
    };
    let amount = text
        .parse::<u128>()
        .map_err(|_| semantic_error(format!("{name} must be a positive integer")))?;
    if amount == 0 {
        return Err(semantic_error(format!("{name} must be positive")));
    }
    Ok(amount)
}

fn value_to_string(value: &Value) -> String {
    match value {
        Value::String(value) => value.clone(),
        other => other.to_string(),
    }
}

fn non_empty(name: &str, value: &str) -> Result<(), X3Error> {
    if value.trim().is_empty() {
        return Err(semantic_error(format!("{name} must not be empty")));
    }
    Ok(())
}

fn semantic_error(message: impl Into<String>) -> X3Error {
    X3Error::SemanticError {
        message: message.into(),
        span: Default::default(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn valid_json() -> &'static str {
        r#"{
          "schema_version": 1,
          "intent": "bridge",
          "from": {"chain":"solana","asset":"USDC","amount":"10","receiver":"alice"},
          "to": {"chain":"ethereum","asset":"USDC","amount":null,"receiver":"bob"},
          "path": [{"type":"bridge","via":"X3"}],
          "requires": [{"kind":"finality","chain":"solana","op":">=","value":"32"}],
          "policies": {"timeout":{"duration":"30s"}}
        }"#
    }

    #[test]
    fn parses_python_envelope_and_builds_draft() {
        let parsed = parse_validated_intent_json(valid_json()).expect("valid envelope");
        let draft = to_intent_spec_draft(&parsed).expect("draft");
        assert_eq!(draft.source_amount, 10);
        assert_eq!(draft.timeout_secs, 30);
    }

    #[test]
    fn rejects_schema_version_and_zero_amount() {
        let mut value: Value = serde_json::from_str(valid_json()).unwrap();
        value["schema_version"] = 2.into();
        assert!(parse_validated_intent_json(&value.to_string()).is_err());
        value["schema_version"] = 1.into();
        value["from"]["amount"] = "0".into();
        assert!(parse_validated_intent_json(&value.to_string()).is_err());
    }

    fn bridge_with_nonce_json() -> String {
        let mut value: Value = serde_json::from_str(valid_json()).unwrap();
        value["requires"].as_array_mut().unwrap().push(serde_json::json!({
            "kind": "nonce",
            "chain": "solana",
            "op": "==",
            "value": "unused_test_nonce_1"
        }));
        value["path"][0]["from_ref"] = serde_json::json!({"chain":"solana","asset":"USDC"});
        value["path"][0]["to_ref"] = serde_json::json!({"chain":"ethereum","asset":"USDC"});
        value.to_string()
    }

    /// The endpoint `Lock`/`Release` and the `Require`/`OnTimeout` guards
    /// must sit OUTSIDE the atomic block (matching `lowering.rs`'s
    /// `Item::Bridge` AST -> IR pattern) — only `Swap`/`Bridge` route steps
    /// belong inside `AtomicBegin`/`AtomicEnd`. This is what makes the
    /// VM's `ON_TIMEOUT` deadline register resolve correctly instead of
    /// panicking with `X3_TIMEOUT` at deadline 0.
    #[test]
    fn to_ir_places_lock_release_and_guards_outside_atomic_block() {
        let intent = parse_validated_intent_json(&bridge_with_nonce_json()).expect("valid envelope");
        let ir = to_ir(&intent).expect("lowers to ir");

        assert_eq!(ir.metadata.nonce.as_deref(), Some("unused_test_nonce_1"));

        let begin_idx = ir
            .operations
            .iter()
            .position(|op| matches!(op, Operation::AtomicBegin))
            .expect("has AtomicBegin");
        let end_idx = ir
            .operations
            .iter()
            .position(|op| matches!(op, Operation::AtomicEnd))
            .expect("has AtomicEnd");
        assert!(begin_idx < end_idx);

        for (idx, op) in ir.operations.iter().enumerate() {
            let inside_atomic = idx > begin_idx && idx < end_idx;
            match op {
                Operation::Bridge { .. } | Operation::Swap { .. } => {
                    assert!(inside_atomic, "swap/bridge ops must be inside the atomic block");
                }
                Operation::Lock { .. } | Operation::Release { .. } => {
                    assert!(!inside_atomic, "lock/release ops must be outside the atomic block");
                }
                Operation::Require { .. } | Operation::OnTimeout { .. } | Operation::OnFail { .. } => {
                    assert!(!inside_atomic, "guards/timeout/onfail must be outside the atomic block");
                }
                _ => {}
            }
        }
    }

    #[test]
    fn to_ir_passes_semantic_check_and_executes() {
        let intent = parse_validated_intent_json(&bridge_with_nonce_json()).expect("valid envelope");
        let ir = to_ir(&intent).expect("lowers to ir");
        crate::check_ir(&ir).expect("semantic check should pass with nonce + timeout present");
    }

    #[test]
    fn to_ir_missing_nonce_fails_semantic_check() {
        // `valid_json()` has a bridge path but no nonce requirement — the
        // replay-protection rule must reject it at the semantic stage.
        let intent = parse_validated_intent_json(valid_json()).expect("valid envelope");
        let ir = to_ir(&intent).expect("lowers to ir");
        let errors = crate::check_ir(&ir).expect_err("missing nonce should fail semantic check");
        assert!(errors.iter().any(|e| e.to_string().contains("nonce")));
    }

    /// A swap lowered from intent JSON must carry the destination chain, and a
    /// guard must carry the comparison it makes: `slippage <= 50` and
    /// `slippage >= 50` are opposite claims, and the IR used to be built with
    /// `comparison: None` because the field did not exist yet.
    #[test]
    fn swap_carries_to_chain_and_guards_carry_their_comparison() {
        let mut value: Value = serde_json::from_str(valid_json()).unwrap();
        value["path"] = serde_json::json!([{
            "type": "swap",
            "from_ref": {"chain": "ethereum", "asset": "DAI"},
            "to_ref": {"chain": "solana", "asset": "SOL"},
            "from": "DAI",
            "to": "SOL",
            "amount": "10",
            "min_output": "9"
        }]);
        value["requires"] = serde_json::json!([
            {"kind": "nonce", "chain": "solana", "op": "==", "value": "n1"},
            {"kind": "slippage", "chain": "ethereum", "op": "<=", "value": "50"}
        ]);
        let intent =
            parse_validated_intent_json(&value.to_string()).expect("valid envelope");
        let ir = to_ir(&intent).expect("lowers to ir");

        let swap = ir
            .operations
            .iter()
            .find_map(|op| match op {
                Operation::Swap {
                    from_chain,
                    to_chain,
                    to_asset,
                    ..
                } => Some((from_chain.clone(), to_chain.clone(), to_asset.clone())),
                _ => None,
            })
            .expect("swap op");
        assert_eq!(swap.0, "ethereum");
        assert_eq!(swap.1, "solana", "the destination chain must not be dropped");
        assert_eq!(swap.2, "SOL");

        let comparison = ir
            .operations
            .iter()
            .find_map(|op| match op {
                Operation::Require {
                    kind,
                    comparison,
                    ..
                } if matches!(kind, RequireKind::SlippageTolerance) => *comparison,
                _ => None,
            })
            .expect("slippage guard with a comparison");
        assert_eq!(
            comparison,
            crate::ir::ComparisonOp::LessOrEqual,
            "`<=` must not be read as `>=`"
        );
        assert!(comparison.is_upper_bound());
    }
}
