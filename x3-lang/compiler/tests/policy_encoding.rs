use std::collections::BTreeSet;
use x3_lang_ast::policy_ir::*;
use x3_lang_ast::{AssetId, ChainRef};
use x3_lang_common::Symbol;
use x3_lang_compiler::policy_encoding::*;

fn hex(b: &[u8]) -> String {
    b.iter().map(|v| format!("{v:02x}")).collect::<Vec<_>>().join("")
}
fn grant() -> Grant {
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
            asset: AssetId {
                vm_family: Symbol::new("evm"),
                chain: ChainRef::new(Symbol::new("base-sepolia")),
                canonical_id: Symbol::new("token"),
                symbol: Symbol::new("USD"),
                decimals: 6,
            },
            max_base_units: 100,
        },
        expires_at: 1000,
        delegation_depth: 2,
        requirements: BTreeSet::from([Requirement::ExactCodeIdentity, Requirement::AtomicRollback]),
    }
}
#[test]
fn python_binding_vectors() {
    let cases = [
        (
            BindingProfile::EvmAction,
            "6872573330be773c6d26bad352d5acf1cd4cc9461657e0d36aa4f73271f0e789",
        ),
        (
            BindingProfile::AgentAdmission,
            "475eb8d792028f685739c5786b8f27a3406a1edb1fb08651c6dedb46ea98fe82",
        ),
        (
            BindingProfile::NearBscWindow,
            "e1dc240594c666538ef1a4e5593dbc2388adce51bd868b769a8c93690af28b3e",
        ),
    ];
    for (p, expected) in cases {
        let r = encode_binding(
            p,
            b"fixture policy bytes",
            "sandbox:fixture",
            std::array::from_fn(|i| i as u8),
        )
        .unwrap();
        assert_eq!(hex(&r.digest), expected);
        assert_eq!(r.encoded.len(), 17 + 1 + p.name().len() + 1 + 15 + 64);
    }
}
#[test]
fn independent_grant_vector() {
    let r = encode_grant(&grant()).unwrap();
    assert_eq!(
        hex(&r.digest),
        "301bde48f2d974c394b66241448544e3428400498bd2375d0ae9c25a03951762"
    );
    assert_eq!(hex(&r.encoded),"58332f5049522f4752414e542f31000006706172656e7400000f73616e64626f783a66697874757265010000000000014a340101010101010101010101010101010101010101020202020202020202020202020202020202020203030303040404040404040404040404040404040404040404040404040404040404040401000365766d000c626173652d7365706f6c69610005746f6b656e0003555344060000000000000000000000000000006400000000000003e802020102");
}
#[test]
fn mutation_changes_commitment() {
    let g = grant();
    let original = encode_grant(&g).unwrap();
    let mut changed = vec![];
    let mut x = g.clone();
    x.subject = "other".into();
    changed.push(x);
    let mut x = g.clone();
    x.parent = Some("root".into());
    changed.push(x);
    let mut x = g.clone();
    x.scope = "sandbox:other".into();
    changed.push(x);
    let mut x = g.clone();
    x.expires_at += 1;
    changed.push(x);
    let mut x = g.clone();
    x.delegation_depth += 1;
    changed.push(x);
    let mut x = g.clone();
    x.requirements.insert(Requirement::AuthenticatedFinality);
    changed.push(x);
    let mut x = g.clone();
    x.requirements.clear();
    changed.push(x);
    let mut x = g.clone();
    if let Resource::EvmCall { chain_id, .. } = &mut x.resource {
        *chain_id = 1;
    }
    changed.push(x);
    let mut x = g.clone();
    if let Resource::EvmCall { executor, .. } = &mut x.resource {
        *executor = [8; 20];
    }
    changed.push(x);
    let mut x = g.clone();
    if let Resource::EvmCall { target, .. } = &mut x.resource {
        *target = [8; 20];
    }
    changed.push(x);
    let mut x = g.clone();
    if let Resource::EvmCall { selector, .. } = &mut x.resource {
        *selector = [8; 4];
    }
    changed.push(x);
    let mut x = g.clone();
    if let Resource::EvmCall { calldata_hash, .. } = &mut x.resource {
        *calldata_hash = [8; 32];
    }
    changed.push(x);
    let mut x = g.clone();
    if let AmountLimit::Asset { max_base_units, .. } = &mut x.amount {
        *max_base_units += 1;
    }
    changed.push(x);
    let mut x = g.clone();
    if let AmountLimit::Asset { asset, .. } = &mut x.amount {
        asset.decimals += 1;
    }
    changed.push(x);
    let mut x = g.clone();
    if let AmountLimit::Asset { asset, .. } = &mut x.amount {
        asset.symbol = Symbol::new("other");
    }
    changed.push(x);
    let mut x = g.clone();
    if let AmountLimit::Asset { asset, .. } = &mut x.amount {
        asset.vm_family = Symbol::new("svm");
    }
    changed.push(x);
    let mut x = g.clone();
    if let AmountLimit::Asset { asset, .. } = &mut x.amount {
        asset.chain = ChainRef::new(Symbol::new("other"));
    }
    changed.push(x);
    let mut x = g.clone();
    if let AmountLimit::Asset { asset, .. } = &mut x.amount {
        asset.canonical_id = Symbol::new("other");
    }
    changed.push(x);
    let mut x = g.clone();
    x.amount = AmountLimit::NoAsset;
    changed.push(x);
    for x in changed {
        assert_ne!(encode_grant(&x).unwrap().digest, original.digest);
    }
    let mut x = g.clone();
    x.requirements = BTreeSet::from([Requirement::AtomicRollback, Requirement::ExactCodeIdentity]);
    assert_eq!(encode_grant(&x).unwrap(), original);
}
#[test]
fn resource_variants_are_disjoint() {
    let mut g = grant();
    g.amount = AmountLimit::NoAsset;
    g.resource = Resource::Tool {
        adapter: "read".into(),
        artifact_hash: [7; 32],
    };
    let t = encode_grant(&g).unwrap();
    let mut digests = BTreeSet::from([t.digest]);
    for operation in [
        SettlementOperation::Lock,
        SettlementOperation::Claim,
        SettlementOperation::Refund,
    ] {
        g.resource = Resource::NearBscWindow {
            window_id: [7; 32],
            operation,
        };
        assert!(digests.insert(encode_grant(&g).unwrap().digest));
    }
}
#[test]
fn malformed_inputs_refused() {
    let p = BindingProfile::EvmAction;
    for s in ["", "bad scope", "é"] {
        assert!(encode_binding(p, b"x", s, [0; 32]).is_err());
    }
    assert!(encode_binding(p, b"", "ok", [0; 32]).is_err());
    assert!(encode_binding(p, &vec![0; 65537], "ok", [0; 32]).is_err());
    assert!(encode_binding(p, b"x", &"a".repeat(129), [0; 32]).is_err());
    let mut g = grant();
    g.expires_at = 0;
    assert!(encode_grant(&g).is_err());
    let mut g = grant();
    g.parent = Some("parent".into());
    assert!(encode_grant(&g).is_err());
    let mut g = grant();
    if let AmountLimit::Asset { asset, .. } = &mut g.amount {
        asset.symbol = Symbol::new(&"a".repeat(257));
    }
    assert!(encode_grant(&g).is_err());
}
