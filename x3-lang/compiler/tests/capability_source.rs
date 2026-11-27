use std::collections::BTreeSet;
use x3_lang_ast::policy_ir::{AmountLimit, Requirement, Resource};
use x3_lang_compiler::policy_encoding::encode_grant;
use x3_lang_compiler::{check_source, compile_capability_source, compile_source, parser::parse_source};

const SOURCE: &str = r#"capability {
    subject "child";
    parent "parent";
    scope "sandbox:fixture";
    tool "reader" "0404040404040404040404040404040404040404040404040404040404040404";
    no_asset;
    expires 1000;
    depth 1;
    requirements [exact_code_identity, atomic_rollback];
}"#;

fn support() -> BTreeSet<Requirement> {
    BTreeSet::from([Requirement::ExactCodeIdentity, Requirement::AtomicRollback])
}
#[test]
fn source_lowers_to_typed_grant_and_commitment() {
    let compiled = compile_capability_source(SOURCE, "sandbox:fixture", &support(), 100).unwrap();
    assert_eq!(compiled.grant.subject, "child");
    assert_eq!(compiled.grant.parent.as_deref(), Some("parent"));
    assert_eq!(
        compiled.grant.resource,
        Resource::Tool {
            adapter: "reader".into(),
            artifact_hash: [4; 32]
        }
    );
    assert_eq!(compiled.grant.amount, AmountLimit::NoAsset);
    assert_eq!(compiled.grant.expires_at, 1000);
    assert_eq!(compiled.grant.delegation_depth, 1);
    assert_eq!(compiled.grant.requirements, support());
    assert_eq!(compiled.commitment, encode_grant(&compiled.grant).unwrap());
}
#[test]
fn unsupported_context_and_expiry_denied() {
    assert!(compile_capability_source(SOURCE, "sandbox:other", &support(), 100).is_err());
    assert!(compile_capability_source(SOURCE, "sandbox:fixture", &BTreeSet::new(), 100).is_err());
    assert!(compile_capability_source(SOURCE, "sandbox:fixture", &support(), 1000).is_err());
}
#[test]
fn malformed_or_unhandled_source_refused() {
    for src in [
        SOURCE.replace("no_asset;", ""),
        SOURCE.replace("no_asset;", "asset \"ETH\";"),
        SOURCE.replace("depth 1;", "depth 256;"),
        SOURCE.replace("depth 1;", "depth 1; depth 9;"),
        SOURCE.replace("atomic_rollback", "unknown_requirement"),
        SOURCE.replace("atomic_rollback", "exact_code_identity"),
        SOURCE.replace(
            "0404040404040404040404040404040404040404040404040404040404040404",
            "xyz",
        ),
        format!("{SOURCE}\nfn other() {{}}"),
        SOURCE.replace("tool \"reader\"", "evm_call \"reader\""),
        SOURCE.replace("subject \"child\"", "subject \"bad identity\""),
    ] {
        assert!(
            compile_capability_source(&src, "sandbox:fixture", &support(), 100).is_err(),
            "{src}"
        );
    }
}
#[test]
fn exact_source_hash_and_semantic_commitment_are_distinct() {
    let a = compile_capability_source(SOURCE, "sandbox:fixture", &support(), 100).unwrap();
    let b = compile_capability_source(&format!("// comment\n{SOURCE}"), "sandbox:fixture", &support(), 100).unwrap();
    assert_eq!(a.commitment, b.commitment);
    assert_ne!(a.source_sha256, b.source_sha256);
    let c = compile_capability_source(
        &SOURCE.replace("depth 1;", "depth 0;"),
        "sandbox:fixture",
        &support(),
        100,
    )
    .unwrap();
    assert_ne!(a.commitment, c.commitment);
}
#[test]
fn legacy_executable_paths_cannot_silently_drop_capability() {
    assert!(parse_source(SOURCE).is_err());
    assert!(compile_source(SOURCE).is_err());
    assert!(check_source(SOURCE).is_err());
    assert!(compile_source(&format!("fn main() {{}}\n{SOURCE}")).is_err());
}
