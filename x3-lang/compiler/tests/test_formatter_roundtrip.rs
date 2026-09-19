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

/// Files the parser reads but the compiler refuses.
///
/// Listed rather than skipped silently: a corpus file that parses and does not
/// compile is a real finding, and the round-trip property cannot be checked on
/// it (there is no bytecode to compare). A new one fails this test instead of
/// joining a growing pile of skipped files.
const REFUSED_BY_THE_COMPILER: &[&str] = &[
    // `swap min_output 0.09` / `10.25` are not integers the checks can compare,
    // and the bridge is written from an asset no `from` clause funds, so the
    // amounts lower to zero. Parsing it is TICKET-045's fix; making it *compile*
    // is a question about the example, not about the formatter.
    "arb_solana_eth.x3",
];

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
            Err(error) => {
                assert!(
                    REFUSED_BY_THE_COMPILER.contains(&name.as_str()),
                    "{name} parses but does not compile, and is not on the known-refused list \
                     (add it there with the reason, or fix it): {error}"
                );
                refused.push(name);
                continue;
            }
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
        "only {checked} examples were checked; the corpus or the dialect floor changed (not \
         compiled: {refused:?})"
    );
}

#[test]
fn the_lexers_comment_reader_agrees_with_what_it_drops() {
    use x3_lang_compiler::parser::source_comments;

    assert_eq!(source_comments("intent a { }").len(), 0);
    assert_eq!(source_comments("// one\nintent a { }\n").len(), 1);
    assert_eq!(source_comments("intent a { // why\n}\n// and this\n").len(), 2);
    assert_eq!(
        source_comments("/* block\n   spanning lines */\nintent a { }\n").len(),
        1
    );
    // A `//` inside a string is part of the string, and a formatter that counted
    // it would claim to have dropped something it never had.
    assert_eq!(source_comments("intent a { use x \"http://example\" }").len(), 0);
    assert_eq!(source_comments("intent a { use x \"/* not a comment */\" }").len(), 0);
}

#[test]
fn the_corpus_is_documentation_heavy() {
    // The corpus's comments are its documentation, so the count matters: it is what
    // `x3c fmt` reports and what the formatter now places rather than deletes.
    let commented: Vec<String> = examples()
        .into_iter()
        .filter(|(_, source)| !x3_lang_compiler::parser::source_comments(source).is_empty())
        .map(|(name, _)| name)
        .collect();
    assert!(
        commented.len() >= 10,
        "the corpus is documentation-heavy and this test is meant to notice when it stops being: \
         {commented:?}"
    );
}

#[test]
fn formatting_keeps_the_comments_it_can_place() {
    use x3_lang_compiler::formatter::X3Formatter;

    let source = "// A header comment about the program.\n//\n// Two lines of it.\n\nintent probe {\n    from ethereum.USDC amount 1\n    to solana.SOL\n    require slippage <= 50\n    on_fail refund ethereum.USDC to sender\n}\n\n// A trailing note.\n";
    let program = parse(source).expect("the fixture must parse");
    let comments = x3_lang_compiler::parser::source_comments(source);
    assert_eq!(
        comments.len(),
        4,
        "three header lines, one of them bare, and a trailing note: {comments:?}"
    );

    let formatted = X3Formatter::new().format_program_with_comments(&program, &comments);
    assert!(
        formatted.contains("// A header comment about the program."),
        "the header stays:\n{formatted}"
    );
    assert!(
        formatted.contains("// Two lines of it."),
        "including its second line:\n{formatted}"
    );
    assert!(
        formatted.contains("// A trailing note."),
        "and a note after the last item:\n{formatted}"
    );
    assert!(
        formatted.find("// A header comment").unwrap() < formatted.find("intent probe").unwrap(),
        "the header is above the declaration it was written above"
    );
    // And the result is still the same program.
    let reparsed = parse(&formatted).expect("the formatted text must parse");
    assert_eq!(
        x3_lang_compiler::compile_program(&program).expect("compiles"),
        x3_lang_compiler::compile_program(&reparsed).expect("compiles"),
        "comments do not change what the program does"
    );
    assert_eq!(
        X3Formatter::new().format_program_with_comments(&reparsed, &comments),
        formatted,
        "and formatting it again changes nothing"
    );
}
