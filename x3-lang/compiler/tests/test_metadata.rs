//! Marketplace metadata — spec build-order item 25, PHASE 26.
//!
//! PHASE 26 wants a compiled strategy to expose metadata without exposing source,
//! and lists the fields. Two of them cannot be filled at compile time, and the
//! tests below pin both halves: that the fields which *can* be real are, and that
//! the ones which cannot are labelled rather than invented.

use x3_lang_compiler::metadata::{strategy_metadata, RiskClass, METADATA_VERSION};
use x3_lang_compiler::semantic::CompilationMode;

/// A module, with the body and the optional sections supplied.
fn module(risk: &str, extra: &str, body: &str) -> String {
    format!(
        r#"strategy TriDexArb {{
    input ethereum.USDC amount 25_000_000 max 50_000_000
    output ethereum.ETH
    effects [swap]
    guarantees [min_profit]
    domains [ethereum]
{risk}{extra}    bounds {{ max_steps 10 max_gas 200_000 }}
    execute {{
{body}
    }}
}}
"#
    )
}

fn default_risk() -> String {
    "    risk { max_slippage_bps 50 max_total_fee_bps 8 }\n".to_string()
}

fn default_body() -> String {
    "        swap uniswap ethereum.USDC -> ethereum.ETH amount 1_000 min_output 1\n        \
     require slippage <= 50\n        require profit >= 5\n        \
     on_fail refund ethereum.USDC to sender"
        .to_string()
}

const LICENSE: &str = "    license { creator x3_strategy_author profit_share 3% }\n";

const SPLIT: &str = "    split profit {\n        97% -> trader\n        3% -> strategy_author\n    }\n";

/// Build the metadata for a source, as the CLI does: verify, compile, describe.
fn metadata(source: &str) -> Option<x3_lang_compiler::metadata::StrategyMetadata> {
    let (program, _, outcome) =
        x3_lang_compiler::check_source_diagnostics_with_mode(source, CompilationMode::Dev).expect("source must lower");
    assert!(outcome.errors.is_empty(), "unexpected errors: {:?}", outcome.errors);
    let bytecode = x3_lang_compiler::compile_source(source).expect("source must compile");
    strategy_metadata(&program, &bytecode)
}

fn json(source: &str) -> String {
    serde_json::to_string_pretty(&metadata(source).expect("a strategy module")).expect("serializes")
}

#[test]
fn the_document_names_its_version_and_the_module() {
    let found = metadata(&module(&default_risk(), "", &default_body())).expect("a module");
    assert_eq!(found.metadata_version, METADATA_VERSION);
    assert_eq!(found.strategy_id, "TriDexArb");
    assert_eq!(found.compiler_version, env!("CARGO_PKG_VERSION"));
}

#[test]
fn the_artifact_hash_is_real_and_deterministic() {
    let source = module(&default_risk(), "", &default_body());
    let first = metadata(&source).expect("a module").artifact_hash;
    let second = metadata(&source).expect("a module").artifact_hash;
    assert_eq!(first, second, "the hash must be a function of the artifact");
    assert!(first.starts_with("sha256:"), "the algorithm must be named: {first}");
    let hex = first.trim_start_matches("sha256:");
    assert_eq!(hex.len(), 64, "SHA-256 is 32 bytes of hex: {first}");
    assert!(hex.chars().all(|c| c.is_ascii_hexdigit()));
}

#[test]
fn the_hash_covers_the_artifact_and_not_the_source() {
    // A body change alters the bytecode, so the hash must change. A comment does
    // not reach the artifact, so the hash must not — the document identifies the
    // artifact, not the file it was written in.
    let base = module(&default_risk(), "", &default_body());
    let changed_body = module(
        &default_risk(),
        "",
        &default_body().replace("amount 1_000", "amount 1_001"),
    );
    let commented = format!("// a comment that is not part of the artifact\n{base}");

    let base_hash = metadata(&base).expect("a module").artifact_hash;
    let changed_hash = metadata(&changed_body).expect("a module").artifact_hash;
    let commented_hash = metadata(&commented).expect("a module").artifact_hash;
    assert_ne!(base_hash, changed_hash, "an artifact change must change the hash");
    assert_eq!(
        base_hash, commented_hash,
        "a comment is not part of the artifact and must not change its identity"
    );
}

#[test]
fn the_two_fields_a_compiler_cannot_fill_are_null_and_labelled() {
    let source = module(&default_risk(), &format!("{LICENSE}{SPLIT}"), &default_body());
    let found = metadata(&source).expect("a module");
    assert!(
        found.artifact_signature.is_none(),
        "a compiler is not a signing authority"
    );
    assert!(found.receipt_references.is_empty(), "receipts accrue at run time");
    let notes = found.notes.join(" ");
    assert!(
        notes.contains("artifact_signature is null"),
        "the gap must be explained: {notes}"
    );
    assert!(
        notes.contains("receipt_references is empty"),
        "and so must this one: {notes}"
    );
}

#[test]
fn the_document_exposes_no_source() {
    // PHASE 26: "expose metadata without necessarily exposing source". The body's
    // text must not appear anywhere in the document.
    let body = default_body();
    let source = module(&default_risk(), &format!("{LICENSE}{SPLIT}"), &body);
    let document = json(&source);
    assert!(
        !document.contains("uniswap"),
        "the venue is route logic and must not leak: {document}"
    );
    assert!(
        !document.contains("swap uniswap"),
        "no statement of the body may appear: {document}"
    );
}

#[test]
fn capital_reports_the_declared_minimum_and_maximum() {
    let found = metadata(&module(&default_risk(), "", &default_body())).expect("a module");
    assert_eq!(found.capital.len(), 1);
    assert_eq!(found.capital[0].asset, "ethereum.USDC");
    assert_eq!(found.capital[0].minimum, 25_000_000);
    assert_eq!(found.capital[0].maximum, Some(50_000_000));
}

#[test]
fn an_unstated_maximum_is_null_rather_than_invented() {
    let source = module(&default_risk(), "", &default_body()).replace(" max 50_000_000", "");
    let found = metadata(&source).expect("a module");
    assert_eq!(found.capital[0].maximum, None);
    assert!(
        found.notes.join(" ").contains("null maximum"),
        "unstated must be labelled, not silently absent: {:?}",
        found.notes
    );
}

#[test]
fn the_risk_class_is_derived_from_the_declared_budget_and_says_so() {
    // The body's own guard has to fit inside the declared profile — the module
    // checks from item 22 refuse a body that relies on more slippage than the
    // module allows — so the low-risk fixture lowers its guard too.
    let low_body = default_body().replace("require slippage <= 50", "require slippage <= 5");
    let low = metadata(&module(
        "    risk { max_slippage_bps 10 max_total_fee_bps 5 }\n",
        "",
        &low_body,
    ))
    .expect("a module");
    assert_eq!(low.risk_class, RiskClass::Low);
    assert!(low.risk_class_basis.contains("15 bps"), "{}", low.risk_class_basis);

    let elevated = metadata(&module(
        "    risk { max_slippage_bps 400 max_total_fee_bps 200 }\n",
        "",
        &default_body(),
    ))
    .expect("a module");
    assert_eq!(elevated.risk_class, RiskClass::Elevated);
    assert!(
        elevated.risk_class_basis.contains("600 bps"),
        "{}",
        elevated.risk_class_basis
    );
}

#[test]
fn the_licence_terms_reach_the_document() {
    let source = module(&default_risk(), &format!("{LICENSE}{SPLIT}"), &default_body());
    let terms = metadata(&source).expect("a module").license.expect("a licence");
    assert_eq!(terms.author, "x3_strategy_author");
    assert_eq!(terms.royalty_bps, 300);
}

#[test]
fn an_unlicensed_module_says_it_is_unlicensed() {
    let found = metadata(&module(&default_risk(), "", &default_body())).expect("a module");
    assert!(found.license.is_none());
    assert!(
        found
            .notes
            .join(" ")
            .contains("unlicensed rather than licensed on unknown terms"),
        "absence must be stated, not implied: {:?}",
        found.notes
    );
}

#[test]
fn a_program_without_a_strategy_module_has_no_metadata() {
    let (program, _, _) = x3_lang_compiler::check_source_diagnostics_with_mode(
        "intent lonely {\n    from ethereum.USDC amount 1 receiver 0x1\n    to ethereum.ETH receiver 0x2\n    \
         route { swap uniswap ethereum.USDC -> ethereum.ETH amount 1 min_output 1 }\n    \
         require nonce unused lonely_1\n    require slippage <= 50\n    \
         timeout 30s refund ethereum.USDC to sender\n    on_fail rollback\n}\n",
        CompilationMode::Dev,
    )
    .expect("source must lower");
    assert!(
        strategy_metadata(&program, &[0x01, 0xFF]).is_none(),
        "metadata describes a module, and this program has none"
    );
}
