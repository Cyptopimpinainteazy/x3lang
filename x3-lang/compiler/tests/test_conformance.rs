use serde::Deserialize;
use std::fs;
use std::path::{Path, PathBuf};
use x3_lang_common::X3Error;
use x3_lang_compiler::check_source;
use x3_lang_compiler::diagnostic::DiagnosticCode;

#[derive(Debug, Deserialize)]
struct Manifest {
    cases: Vec<Case>,
}

#[derive(Debug, Deserialize)]
struct Case {
    name: String,
    path: String,
    expect: String,
    code: Option<String>,
}

fn conformance_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("compiler crate must live under x3-lang")
        .join("tests")
        .join("conformance")
}

fn diagnostic_code_for_error(error: &X3Error) -> Option<DiagnosticCode> {
    match error {
        X3Error::LexerError { .. } | X3Error::ParseError { .. } => Some(DiagnosticCode::UnexpectedToken),
        X3Error::NameError { .. } => Some(DiagnosticCode::UndefinedSymbol),
        X3Error::TypeError { .. } => Some(DiagnosticCode::IncompatibleTypes),
        _ => None,
    }
}

#[test]
fn manifest_cases_match_authoritative_compiler_behavior() {
    let root = conformance_root();
    let manifest_text = fs::read_to_string(root.join("manifest.json")).expect("read conformance manifest");
    let manifest: Manifest = serde_json::from_str(&manifest_text).expect("parse conformance manifest");

    assert!(!manifest.cases.is_empty(), "conformance manifest must not be empty");

    for case in manifest.cases {
        let source = fs::read_to_string(root.join(&case.path))
            .unwrap_or_else(|e| panic!("{}: failed to read {}: {e}", case.name, case.path));
        let result = check_source(&source);

        match case.expect.as_str() {
            "accept" => match result {
                Ok((_program, _ir, errors)) => {
                    assert!(
                        errors.is_empty(),
                        "{}: expected acceptance, got semantic errors: {errors:?}",
                        case.name
                    );
                }
                Err(error) => panic!("{}: expected acceptance, got compiler error: {error:?}", case.name),
            },
            "reject" => {
                let observed_code = match result {
                    Err(error) => diagnostic_code_for_error(&error).map(|code| code.as_str().to_owned()),
                    Ok((_program, _ir, errors)) => {
                        assert!(
                            !errors.is_empty(),
                            "{}: expected rejection, but compiler accepted source",
                            case.name
                        );
                        errors
                            .iter()
                            .find_map(diagnostic_code_for_error)
                            .map(|code| code.as_str().to_owned())
                    }
                };

                if let Some(expected_code) = case.code.as_deref() {
                    assert_eq!(
                        observed_code.as_deref(),
                        Some(expected_code),
                        "{}: wrong diagnostic code",
                        case.name
                    );
                }
            }
            other => panic!("{}: unsupported expectation {other:?}", case.name),
        }
    }
}
