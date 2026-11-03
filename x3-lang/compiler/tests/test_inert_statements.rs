//! An expression statement that calls nothing is a line the author believed was a declaration.
//!
//! `transfer_proof eth_receipt` written on its own line after a bridge — the *step's* option, meant to
//! be part of the step — parses as two identifier statements, lowers to nothing, and `x3c check` said
//! `ok` (TICKET-037). The same shape swallows the residue of any half-written clause: before the refund
//! arm was fixed, `refund ethereum.USDC to sender` left `to` and `sender` behind as statements that did
//! nothing. A statement is a call, or it is refused by name — the rule a declaration nothing reads
//! follows, one construct over.

fn errors(source: &str) -> Vec<String> {
    match x3_lang_compiler::check_source_diagnostics(source) {
        Ok((_, _, outcome)) => outcome.errors.iter().map(|error| error.to_string()).collect(),
        Err(error) => vec![format!("{error}")],
    }
}

fn program(body: &str) -> String {
    format!("fn f() {{\n{body}\n}}\n")
}

#[test]
fn a_stray_clause_option_is_refused_by_name() {
    let found = errors(&program("    transfer_proof eth_receipt"));
    assert!(
        found
            .iter()
            .any(|error| error.contains("transfer_proof") && error.contains("no effect")),
        "the option belongs on the step it modifies, and the line must say so: {found:?}"
    );
}

#[test]
fn the_residue_of_a_half_written_clause_is_refused() {
    let found = errors(&program("    to sender"));
    assert!(
        found
            .iter()
            .any(|error| error.contains("`to`") && error.contains("no effect")),
        "a clause's tail is not a statement: {found:?}"
    );
}

#[test]
fn a_bare_literal_is_refused() {
    let found = errors(&program("    400"));
    assert!(
        found
            .iter()
            .any(|error| error.contains("400") && error.contains("no effect")),
        "a value on its own line is dropped by every reader of the artifact: {found:?}"
    );
}

#[test]
fn a_call_statement_is_a_statement() {
    // The control: this is what an expression statement is for.
    let found = errors(&program("    mempool_scan(max_results=10)"));
    assert!(
        !found.iter().any(|error| error.contains("no effect")),
        "a call has an effect: {found:?}"
    );
}
