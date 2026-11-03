//! Clauses whose words are **lexer keywords**, and the arms that read them.
//!
//! The parser matches tokens, and a word the lexer reserves never arrives as an identifier. Three
//! arms were written against the identifier form anyway, so the clauses they read were refused
//! while the messages and comments around them said the clauses were supported (TICKET-046):
//!
//! | clause | what happened |
//! |---|---|
//! | `use <target> <config>` in an intent body | `unexpected clause in intent body: KwUse`, from a
//!   match that lists `use` among the clauses it accepts |
//! | `finality_policy X { ethereum require finalized }` | `expected '}' after finality_policy body` |
//! | `rpc_quorum { source require 2_of_3 }` | `rpc_quorum source chain: expected identifier` |
//!
//! Each test below fails against the identifier arm: the source is a program no build could read.

use x3_lang_compiler::{compile_to_ir, parser::parse_source, Operation};

fn ir(source: &str) -> Vec<Operation> {
    let program = parse_source(source).unwrap_or_else(|error| panic!("must parse:\n{source}\n{error}"));
    compile_to_ir(&program)
        .unwrap_or_else(|error| panic!("must lower:\n{source}\n{error:?}"))
        .operations
}

#[test]
fn the_terse_finality_policy_form_parses() {
    // `<chain> require <mode>` — the form the parser's own comment documents, next to the long
    // `chain` / `requirement` / `blocks` form.
    let operations = ir("finality_policy strict {\n    ethereum require finalized\n    blocks 12\n}\n");
    assert!(
        operations.iter().any(|operation| matches!(
            operation,
            Operation::Require {
                condition: x3_lang_compiler::ir::Condition::FinalityPolicy { requirement, blocks, .. },
                ..
            } if requirement == "finalized" && *blocks == Some(12)
        )),
        "the terse form must state the requirement and the depth: {operations:?}"
    );
}

#[test]
fn the_long_finality_policy_form_still_parses() {
    let operations = ir("finality_policy strict {\n    chain ethereum\n    requirement finalized\n    blocks 12\n}\n");
    assert!(
        operations.iter().any(|operation| matches!(
            operation,
            Operation::Require {
                condition: x3_lang_compiler::ir::Condition::FinalityPolicy { requirement, blocks, .. },
                ..
            } if requirement == "finalized" && *blocks == Some(12)
        )),
        "the long form is unchanged: {operations:?}"
    );
}

#[test]
fn the_inline_rpc_quorum_form_parses() {
    // `source require N_of_M` on one line, which the parser's own comment calls the inline form.
    let operations = ir("rpc_quorum {\n    source require 2_of_3\n    relayers a b c\n}\n");
    assert!(
        operations
            .iter()
            .any(|operation| matches!(operation, Operation::RpcConsensus { require: (2, 3), .. })),
        "the inline form must state the quorum it names: {operations:?}"
    );
}

#[test]
fn the_use_clause_lowers_to_a_named_host_call() {
    // `use <target> <config>` is a host call: the language has no instruction for "use this
    // venue", so it asks the host by name — the same shape `on <event> <action>` lowers to.
    let operations = ir("intent probe {\n    from ethereum.USDC amount 1 receiver \
         0x1111111111111111111111111111111111111111\n    to solana.SOL receiver \
         4Nd1mzi8Y1QYxJt9wZWBYZpG7S4pYkZs6YzD3Vt9aBcD\n    use uniswap 1\n    on_fail \
         rollback\n}\n");
    assert!(
        operations.iter().any(|operation| matches!(
            operation,
            Operation::Call { function, args } if function == "use" && args == &["uniswap".to_string(), "1".to_string()]
        )),
        "`use uniswap 1` must reach the IR as a named call: {operations:?}"
    );
}

/// No arm may match an identifier for a word the lexer reserves.
///
/// This is the class the three clauses above came from, and it is worth a test rather than a note
/// because the failure is invisible: an arm for a word the lexer sends as a keyword token compiles,
/// reads correctly, and can never run. The three clauses it cost were refused while the comments
/// beside them said they were supported, which is how they survived a corpus, a formatter and a
/// round-trip test.
///
/// The two lists are read out of the sources that define them — the lexer's keyword table and
/// `keyword_to_tok` — so a word becoming a keyword later fails this test in the file that has to be
/// updated, rather than silently killing its arms.
#[test]
fn no_arm_matches_an_identifier_for_a_lexer_keyword() {
    use std::collections::BTreeSet;
    use std::path::PathBuf;

    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("..");
    let parser = std::fs::read_to_string(root.join("compiler").join("src").join("parser.rs"))
        .expect("the parser source must be readable");
    let lexer = std::fs::read_to_string(root.join("crates").join("x3-lexer").join("src").join("token.rs"))
        .expect("the lexer source must be readable");

    // `"<word>" => Some(Keyword::<Name>)` in the lexer, and `Keyword::<Name> => Tok::Kw<…>` in the
    // parser. A word whose lexer keyword has a `Tok::Kw*` arm arrives as a keyword token; a word
    // the lexer reserves with *no* such arm — `timeout` is one — falls back to `Tok::Ident`, which
    // is why the two lists have to be read together rather than inferred from the lexer alone.
    let keywords: Vec<(&str, &str)> = lexer
        .match_indices("\" => Some(Keyword::")
        .filter_map(|(at, _)| {
            let start = lexer[..at].rfind('"')?;
            let rest = &lexer[at + "\" => Some(Keyword::".len()..];
            let end = rest.find(|c: char| !c.is_alphanumeric() && c != '_')?;
            Some((&lexer[start + 1..at], &rest[..end]))
        })
        .collect();
    assert!(
        keywords.len() > 20,
        "the scan must read the lexer's keyword table: {keywords:?}"
    );
    let mapped: BTreeSet<&str> = parser
        .match_indices("Keyword::")
        .filter_map(|(at, _)| {
            let rest = &parser[at + "Keyword::".len()..];
            let end = rest.find(' ')?;
            let name = &rest[..end];
            let tail = &rest[end..];
            tail.trim_start().starts_with("=> Tok::Kw").then_some(name)
        })
        .collect();
    assert!(mapped.len() > 20, "the scan must read `keyword_to_tok`: {mapped:?}");

    let reserved: BTreeSet<&str> = keywords
        .iter()
        .filter(|(_, keyword)| mapped.contains(keyword))
        .map(|(word, _)| *word)
        .collect();
    assert!(
        reserved.len() > 5,
        "the scan must find the words that do arrive as keyword tokens: {reserved:?}"
    );

    // Comments in this file and in the parser quote the shape being searched for, so they are cut
    // before the scan: a line without its `//` tail cannot be mistaken for an arm.
    let mut offending: Vec<String> = Vec::new();
    for (number, line) in parser.lines().enumerate() {
        let code = line.split("//").next().unwrap_or("");
        let Some(at) = code.find("Tok::Ident(ref ") else {
            continue;
        };
        let rest = &code[at + "Tok::Ident(ref ".len()..];
        let Some(quote) = rest.find("\"") else { continue };
        let end = match rest[quote + 1..].find('"') {
            Some(end) => quote + 1 + end,
            None => continue,
        };
        let word = &rest[quote + 1..end];
        if reserved.contains(word) {
            offending.push(format!("parser.rs:{} matches Tok::Ident for `{word}`", number + 1));
        }
    }
    assert!(
        offending.is_empty(),
        "these words reach the parser as keyword tokens, so the arms can never run:\n  {}",
        offending.join("\n  ")
    );

    // The same rule for the guard's lookahead list: a word that arrives as a keyword stops a
    // valueless guard on its own, so listing it is a second statement of the grammar that says
    // something the parser does not need. `on_fail` was one — the list is read here, not restated,
    // so an entry cannot be forgotten about in one place and kept in another.
    let list_at = parser
        .find("const CLAUSE_WORDS")
        .expect("the parser must state its clause words");
    let list = &parser[list_at..];
    let list_end = list.find("];").expect("the list must end");
    let redundant: Vec<&str> = list[..list_end]
        .split('"')
        .skip(1)
        .step_by(2)
        .filter(|word| reserved.contains(word))
        .collect();
    assert!(
        redundant.is_empty(),
        "these words are in CLAUSE_WORDS and reach the parser as keyword tokens, which cannot \
         begin an expression and stop a guard without an entry: {redundant:?}"
    );
}
