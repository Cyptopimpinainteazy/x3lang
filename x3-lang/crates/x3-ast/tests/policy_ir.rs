use std::collections::BTreeSet;
use x3_lang_ast::policy_ir::*;
use x3_lang_ast::{AssetId, ChainRef};
use x3_lang_common::Symbol;

fn asset(chain: &str, decimals: u8) -> AssetId {
    AssetId {
        vm_family: Symbol::new("evm"),
        chain: ChainRef::new(Symbol::new(chain)),
        canonical_id: Symbol::new("token"),
        symbol: Symbol::new("USD"),
        decimals,
    }
}
fn parent() -> Grant {
    Grant {
        subject: "parent".into(),
        parent: None,
        scope: "sandbox:fixture".into(),
        resource: Resource::EvmCall {
            chain_id: 84532,
            executor: [1; 20],
            target: [2; 20],
            selector: [3; 4],
            calldata_hash: [4; 32],
        },
        amount: AmountLimit::Asset {
            asset: asset("base-sepolia", 6),
            max_base_units: 100,
        },
        expires_at: 1000,
        delegation_depth: 2,
        requirements: BTreeSet::from([Requirement::ExactCodeIdentity, Requirement::AtomicRollback]),
    }
}
fn child() -> Grant {
    let mut g = parent();
    g.subject = "child".into();
    g.parent = Some("parent".into());
    g.expires_at = 900;
    g.delegation_depth = 1;
    g.amount = AmountLimit::Asset {
        asset: asset("base-sepolia", 6),
        max_base_units: 50,
    };
    g
}
#[test]
fn bounded_child_and_added_requirement() {
    let p = parent();
    let mut c = child();
    assert_eq!(check_delegation(&p, &c, 100), Ok(()));
    c.requirements.insert(Requirement::AuthenticatedFinality);
    assert_eq!(check_delegation(&p, &c, 100), Ok(()));
}
#[test]
fn deny_expanded_resource_and_context() {
    let p = parent();
    for r in [
        Resource::EvmCall {
            chain_id: 1,
            executor: [1; 20],
            target: [2; 20],
            selector: [3; 4],
            calldata_hash: [4; 32],
        },
        Resource::EvmCall {
            chain_id: 84532,
            executor: [9; 20],
            target: [2; 20],
            selector: [3; 4],
            calldata_hash: [4; 32],
        },
        Resource::EvmCall {
            chain_id: 84532,
            executor: [1; 20],
            target: [9; 20],
            selector: [3; 4],
            calldata_hash: [4; 32],
        },
        Resource::EvmCall {
            chain_id: 84532,
            executor: [1; 20],
            target: [2; 20],
            selector: [9; 4],
            calldata_hash: [4; 32],
        },
        Resource::EvmCall {
            chain_id: 84532,
            executor: [1; 20],
            target: [2; 20],
            selector: [3; 4],
            calldata_hash: [9; 32],
        },
        Resource::Tool {
            adapter: "read".into(),
            artifact_hash: [4; 32],
        },
        Resource::NearBscWindow {
            window_id: [4; 32],
            operation: SettlementOperation::Claim,
        },
    ] {
        let mut c = child();
        c.resource = r;
        assert_eq!(check_delegation(&p, &c, 100), Err(PolicyError::Resource));
    }
    let mut c = child();
    c.scope = "production".into();
    assert_eq!(check_delegation(&p, &c, 100), Err(PolicyError::Resource));
}
#[test]
fn deny_amount_or_asset_expansion() {
    let p = parent();
    for limit in [
        AmountLimit::Asset {
            asset: asset("base-sepolia", 6),
            max_base_units: 101,
        },
        AmountLimit::Asset {
            asset: asset("ethereum", 6),
            max_base_units: 50,
        },
        AmountLimit::Asset {
            asset: asset("base-sepolia", 18),
            max_base_units: 50,
        },
        AmountLimit::NoAsset,
    ] {
        let mut c = child();
        c.amount = limit;
        assert_eq!(check_delegation(&p, &c, 100), Err(PolicyError::Amount));
    }
}
#[test]
fn deny_expiry_depth_lineage_and_removed_requirement() {
    let p = parent();
    let mut c = child();
    c.expires_at = 1001;
    assert_eq!(check_delegation(&p, &c, 100), Err(PolicyError::Expiry));
    assert_eq!(check_delegation(&p, &child(), 1000), Err(PolicyError::Expiry));
    let mut c = child();
    c.delegation_depth = 2;
    assert_eq!(check_delegation(&p, &c, 100), Err(PolicyError::Depth));
    let mut c = child();
    c.parent = Some("wrong".into());
    assert_eq!(check_delegation(&p, &c, 100), Err(PolicyError::Identity));
    let mut c = child();
    c.subject = "parent".into();
    assert_eq!(check_delegation(&p, &c, 100), Err(PolicyError::Identity));
    let mut c = child();
    c.requirements.remove(&Requirement::AtomicRollback);
    assert_eq!(check_delegation(&p, &c, 100), Err(PolicyError::Requirements));
}
#[test]
fn unsupported_requirements_are_not_silently_dropped() {
    let mut g = child();
    g.requirements.insert(Requirement::AuthenticatedFinality);
    assert_eq!(
        check_support(&g, &parent().requirements, 100),
        Err(PolicyError::Requirements)
    );
    assert_eq!(check_support(&g, &g.requirements, 100), Ok(()));
    assert_eq!(check_support(&g, &g.requirements, 900), Err(PolicyError::Expiry));
}
#[test]
fn malformed_grants_refused() {
    let mut g = child();
    g.scope = "".into();
    assert_eq!(check_support(&g, &g.requirements, 100), Err(PolicyError::Invalid));
    let mut g = child();
    g.resource = Resource::Tool {
        adapter: "".into(),
        artifact_hash: [0; 32],
    };
    assert_eq!(check_support(&g, &g.requirements, 100), Err(PolicyError::Invalid));
    let mut g = child();
    g.subject = "bad identity".into();
    assert_eq!(check_support(&g, &g.requirements, 100), Err(PolicyError::Invalid));
}
