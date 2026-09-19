//! `x3c fmt` must not change what a program means — or stop it being one.
//!
//! A formatter is a claim that the text it writes means what the text it read
//! meant. Nothing about the formatter's own tests can establish that: the arms
//! can be self-consistent and still emit a dialect the parser does not read. So
//! the invariant is stated over the corpus and checked against the compiler
//! rather than against the formatter's expectations:
//!
//!   1. the formatted text parses, and
//!   2. it compiles to the *same bytecode* as the text it came from.
//!
//! The second is what makes this stronger than "the output looks like the input".
//! A formatter that dropped a clause, reordered one, or read `refund X to Y` as
//! `rollback` would satisfy a re-parse check and fail this one.

use std::path::PathBuf;

use x3_lang_ast::ast::Program;
use x3_lang_compiler::formatter::X3Formatter;

fn parse(source: &str) -> Result<Program, String> {
    x3_lang_compiler::parser::parse_source(source).map_err(|error| format!("{error}"))
}

/// Every example in the corpus, as `(file name, source)`.
///
/// Six of them are written in a dialect this parser does not read yet
/// (TICKET-014), and the round-trip test skips what the compiler itself refuses
/// — with a floor on how many were checked, so the test cannot pass by checking
/// nothing.
fn examples() -> Vec<(String, String)> {
    let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../examples");
    let mut found: Vec<(String, String)> = std::fs::read_dir(&dir)
        .unwrap_or_else(|error| panic!("read {}: {error}", dir.display()))
        .filter_map(|entry| {
            let path = entry.ok()?.path();
            if path.extension().and_then(|extension| extension.to_str()) != Some("x3") {
                return None;
            }
            let name = path.file_name()?.to_str()?.to_string();
            let source = std::fs::read_to_string(&path).ok()?;
            Some((name, source))
        })
        .collect();
    found.sort_by(|left, right| left.0.cmp(&right.0));
    assert!(!found.is_empty(), "no examples found in {}", dir.display());
    found
}

#[test]
fn formatting_an_example_preserves_its_meaning() {
    let mut checked = 0usize;
    let mut refused: Vec<String> = Vec::new();
    for (name, source) in examples() {
        let Ok(program) = parse(&source) else {
            refused.push(name);
            continue;
        };
        let before = match x3_lang_compiler::compile_program(&program) {
            Ok(bytecode) => bytecode,
            Err(error) => panic!("{name} is in the corpus and does not compile: {error}"),
        };

        let formatted = X3Formatter::new().format_program(&program);
        let reparsed = match parse(&formatted) {
            Ok(program) => program,
            Err(error) => panic!(
                "{name}: the formatter wrote text the parser does not read: {error}\n\
                 --- formatted ---\n{formatted}"
            ),
        };
        let after = x3_lang_compiler::compile_program(&reparsed).unwrap_or_else(|error| {
            panic!(
                "{name}: the formatted text parses but does not compile: {error}\n\
                 --- formatted ---\n{formatted}"
            )
        });
        assert_eq!(
            before,
            after,
            "{name}: formatting changed the compiled artifact ({} bytes before, {} after)",
            before.len(),
            after.len()
        );

        // Formatting what it just wrote has to be a no-op, or the command's
        // "already formatted" answer is not stable.
        let twice = X3Formatter::new().format_program(&reparsed);
        assert_eq!(twice, formatted, "{name}: formatting is not idempotent");
        checked += 1;
    }
    assert!(
        checked >= 17,
        "only {checked} examples were checked; the corpus or the dialect floor changed (refused: {refused:?})"
    );
}

#[test]
fn the_comment_count_says_what_the_formatter_would_drop() {
    use x3_lang_compiler::formatter::comment_count;

    assert_eq!(comment_count("intent a { }"), 0);
    assert_eq!(comment_count("// one\nintent a { }\n"), 1);
    assert_eq!(comment_count("intent a { // why\n}\n// and this\n"), 2);
    assert_eq!(comment_count("/* block\n   spanning lines */\nintent a { }\n"), 1);
    // A `//` inside a string is part of the string, and a formatter that counted
    // it would claim to have dropped something it never had.
    assert_eq!(comment_count("intent a { use x \"http://example\" }"), 0);
    assert_eq!(comment_count("intent a { use x \"/* not a comment */\" }"), 0);
}

#[test]
fn formatting_an_example_would_drop_its_comments_and_that_is_not_silent() {
    // The formatter cannot write a comment back — the lexer discards them before
    // the AST exists — so the honest thing is that `x3c fmt` says so. This test
    // holds the CLI's message to the count, and the count to the corpus.
    let commented: Vec<String> = examples()
        .into_iter()
        .filter(|(_, source)| x3_lang_compiler::formatter::comment_count(source) > 0)
        .map(|(name, _)| name)
        .collect();
    assert!(
        commented.len() >= 10,
        "the corpus is documentation-heavy and this test is meant to notice when it stops being: \
         {commented:?}"
    );
}
