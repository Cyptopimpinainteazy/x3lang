use x3_lang_compiler::{compile_source, evm_capability::compile_evm_action_source};
const SOURCE: &str = include_str!("fixtures/pir_evm_action.x3");
fn data() -> Vec<u8> {
    let mut out = vec![0x60, 0xfe, 0x47, 0xb1];
    out.extend_from_slice(&[0; 31]);
    out.push(7);
    out
}
#[test]
fn exact_action_abi() {
    let p = compile_evm_action_source(SOURCE, "sandbox:pir06", 100, 2, 1, &data()).unwrap();
    assert_eq!(p.abi.len(), 352);
    assert_eq!(&p.abi[32 + 12..64], &[0x11; 20]);
    assert_eq!(&p.abi[64 + 12..96], &[0x22; 20]);
    assert_eq!(&p.abi[96..100], &data()[..4]);
    assert_eq!(&p.abi[160..192], &[0; 32]);
    assert_eq!(&p.abi[192 + 12..224], &[0x33; 20]);
    assert_eq!(&p.abi[224..256], &p.policy_hash);
    assert_eq!(p.abi[287], 2);
    assert_eq!(p.abi[319], 1);
    assert_ne!(p.action_id, [0; 32]);
}
#[test]
fn refuses_unsupported_or_changed_request() {
    for s in [
        SOURCE.replace("84532", "1"),
        SOURCE.replace("parent none", "parent \"root\""),
        SOURCE.replace("depth 0", "depth 1"),
        SOURCE.replace("requirements []", "requirements [atomic_rollback]"),
        SOURCE.replace("no_asset;", "asset \"ETH\";"),
        SOURCE.replace("0x3333333333333333333333333333333333333333", "bad"),
    ] {
        assert!(
            compile_evm_action_source(&s, "sandbox:pir06", 100, 2, 1, &data()).is_err(),
            "{s}"
        );
    }
    let mut changed = data();
    changed[35] = 8;
    assert!(compile_evm_action_source(SOURCE, "sandbox:pir06", 100, 2, 1, &changed).is_err());
    assert!(compile_evm_action_source(SOURCE, "other", 100, 2, 1, &data()).is_err());
    assert!(compile_evm_action_source(SOURCE, "sandbox:pir06", 1000, 2, 1, &data()).is_err());
    assert!(compile_evm_action_source(SOURCE, "sandbox:pir06", 100, 0, 1, &data()).is_err());
    assert!(compile_source(SOURCE).is_err());
}
#[test]
fn context_and_source_bound() {
    let p = compile_evm_action_source(SOURCE, "sandbox:pir06", 100, 2, 1, &data()).unwrap();
    let q = compile_evm_action_source(SOURCE, "sandbox:pir06", 100, 2, 2, &data()).unwrap();
    assert_eq!(p.policy_hash, q.policy_hash);
    assert_ne!(p.action_id, q.action_id);
    let q = compile_evm_action_source(&format!("// review\n{SOURCE}"), "sandbox:pir06", 100, 2, 1, &data()).unwrap();
    assert_ne!(p.policy_hash, q.policy_hash);
    assert_ne!(p.action_id, q.action_id);
}
