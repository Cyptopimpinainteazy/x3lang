//! Versioning, policy-strength, and canonical-commitment tests.

use std::collections::{BTreeMap, BTreeSet};

use x3_lang_compiler::ir::{
    AssetKey, CompiledTradingPolicy, CostKind, StateBindingMode, SubmissionProfile,
};
use x3_lang_vm::economic::{
    CanonicalCommitment, EconomicError, EconomicPlan, EconomicPolicy, EconomicSnapshot,
};

struct ControlledCommitment {
    domain: &'static [u8],
}

impl CanonicalCommitment for ControlledCommitment {
    fn domain(&self) -> &'static [u8] {
        self.domain
    }

    fn canonical_bytes(&self) -> Result<Vec<u8>, EconomicError> {
        Ok(b"fixed-payload".to_vec())
    }
}

fn asset(symbol: &str) -> AssetKey {
    AssetKey {
        vm_family: "evm".to_string(),
        chain: "ethereum".to_string(),
        canonical_id: format!("0x{symbol}"),
        symbol: symbol.to_string(),
        decimals: 6,
    }
}

fn fixture_policy() -> EconomicPolicy {
    EconomicPolicy {
        version: 1,
        policy_id: "policy-v1".to_string(),
        chain: "ethereum".to_string(),
        settlement_asset: asset("USDC"),
        minimum_net_profit: 10_000,
        max_total_cost: 1_000,
        max_slippage_bps: 30,
        max_price_impact_bps: 20,
        max_mev_leakage_bps: 10,
        quote_freshness_blocks: 3,
        deadline_blocks: 10,
        submission_profile: SubmissionProfile::Private,
        state_binding: StateBindingMode::Exact,
        allowed_cost_kinds: BTreeSet::from([CostKind::Gas, CostKind::LiquidityFee]),
        allow_mint: false,
        allow_burn: false,
    }
}

fn fixture_snapshot() -> EconomicSnapshot {
    EconomicSnapshot {
        version: 1,
        chain: "ethereum".to_string(),
        vm_family: "evm".to_string(),
        block_height: 100,
        block_hash: Some([1u8; 32]),
        host_state_commitment: [2u8; 32],
        oracle_observations: BTreeMap::from([("USDC/USD".to_string(), 1_000_000u128)]),
        oracle_provenance: BTreeMap::from([("USDC/USD".to_string(), "oracle-1".to_string())]),
        quote_observation_blocks: BTreeMap::from([("quote-1".to_string(), 99u64)]),
        balances: BTreeMap::from([(asset("USDC"), 1_000_000u128)]),
        debt_state: BTreeMap::new(),
        route_commitment: [3u8; 32],
        operations_commitment: [4u8; 32],
        expected_outputs: BTreeMap::from([("swap-0".to_string(), 990_000u128)]),
        predicted_costs: BTreeMap::from([(CostKind::Gas, 500u128)]),
        observed_at_unix_seconds: 1_800_000_000,
    }
}

fn fixture_plan(snapshot: &EconomicSnapshot) -> EconomicPlan {
    EconomicPlan {
        version: 1,
        artifact_hash: [5u8; 32],
        operations: Vec::new(),
        asset_bindings: BTreeMap::from([("settlement".to_string(), asset("USDC"))]),
        amount_bindings: BTreeMap::from([("input".to_string(), 1_000_000u128)]),
        venues: BTreeSet::from(["uniswap-v3".to_string()]),
        providers: BTreeSet::new(),
        minimum_outputs: BTreeMap::from([(0u32, 990_000u128)]),
        deadlines: BTreeMap::from([(0u32, 110u64)]),
        expected_costs: BTreeMap::from([(CostKind::Gas, 500u128)]),
        expected_value_flow_commitment: [6u8; 32],
        snapshot_commitment: snapshot.commitment().unwrap(),
        policy_commitment: fixture_policy().commitment().unwrap(),
    }
}

#[test]
fn economic_commitments_are_deterministic_and_domain_separated() {
    let snapshot = fixture_snapshot();
    assert_eq!(
        snapshot.commitment().unwrap(),
        snapshot.commitment().unwrap()
    );
    assert_ne!(
        snapshot.commitment().unwrap(),
        fixture_plan(&snapshot).commitment().unwrap()
    );
}

#[test]
fn identical_canonical_bytes_are_domain_separated_with_stable_snapshot_vector() {
    let snapshot = fixture_snapshot();
    let plan = fixture_plan(&snapshot);
    let snapshot_control = ControlledCommitment {
        domain: snapshot.domain(),
    };
    let plan_control = ControlledCommitment {
        domain: plan.domain(),
    };

    assert_ne!(
        snapshot_control.commitment().unwrap(),
        plan_control.commitment().unwrap()
    );
    assert_eq!(
        snapshot_control.commitment().unwrap(),
        [
            0xd0, 0x28, 0x39, 0x63, 0xc0, 0x38, 0x86, 0xe2, 0x15, 0x06, 0x4f, 0x28, 0xe1, 0xa4,
            0x6c, 0x5e, 0x5c, 0x23, 0x1f, 0xf2, 0x73, 0x8b, 0x59, 0x72, 0x4b, 0xe5, 0x76, 0xdb,
            0x23, 0x02, 0x25, 0xc8,
        ]
    );
}

#[test]
fn runtime_policy_cannot_weaken_compiled_policy() {
    let compiled = fixture_policy();
    let mut runtime = compiled.clone();
    runtime.max_total_cost += 1;
    assert_eq!(
        runtime.validate_not_weaker_than(&compiled),
        Err(EconomicError::PolicyWeakening("max_total_cost")),
    );
}

#[test]
fn unknown_versions_fail_closed() {
    let mut snapshot = fixture_snapshot();
    snapshot.version = u16::MAX;
    assert_eq!(
        snapshot.validate_version(),
        Err(EconomicError::UnsupportedVersion {
            object: "snapshot",
            version: u16::MAX,
        }),
    );
}

#[test]
fn legacy_private_flag_must_match_submission_profile() {
    let policy = CompiledTradingPolicy {
        policy_id: "legacy-policy".to_string(),
        policy_version: 1,
        chain: "ethereum".to_string(),
        max_slippage_bps: 30,
        max_gas: 1_000,
        max_flash_fee_bps: 10,
        deadline_blocks: 10,
        require_private_submission: true,
        minimum_net_profit: Some(10),
        max_total_cost: 1_000,
        max_price_impact_bps: 20,
        max_mev_leakage_bps: 10,
        quote_freshness_blocks: 3,
        submission_profile: SubmissionProfile::Public,
        state_binding: StateBindingMode::Exact,
        allowed_cost_kinds: BTreeSet::from([CostKind::Gas]),
        allow_mint: false,
        allow_burn: false,
    };

    assert!(!policy.submission_profile_is_consistent());
}
