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

#[test]
fn an_invariant_body_is_source_text_and_survives_formatting() {
    // The body reaches the IR as a string, and the parser used to store
    // `format!("{:?}", expr)` — a Rust value tree. The formatter could not write it
    // back (its own printer would have emitted `profit Ge 5`, since the binary arm
    // used `{:?}` for the operator too, which the parser refuses). Both are source
    // now, and this asserts the pair: the stored text is what the program wrote, and
    // formatting the declaration produces a program.
    use x3_lang_compiler::formatter::X3Formatter;
    let source = "invariant profit_floor { assert profit >= 5 }\n\nintent probe {\n    from ethereum.USDC amount 1\n    to solana.SOL\n    require slippage <= 50\n    on_fail refund ethereum.USDC to sender\n}\n";
    let program = parse(source).expect("the fixture must parse");

    let stored = program
        .items
        .iter()
        .find_map(|item| match &item.node {
            x3_lang_ast::ast::Item::InvariantDecl(invariant) => Some(invariant.assert_expr.as_str().to_string()),
            _ => None,
        })
        .expect("the fixture declares an invariant");
    assert_eq!(stored, "profit >= 5", "the body is the source the program wrote");

    let formatted = X3Formatter::new().format_program(&program);
    assert!(formatted.contains("assert profit >= 5"), "{formatted}");
    let reparsed = parse(&formatted).expect("what the formatter writes must parse");
    assert_eq!(
        x3_lang_compiler::compile_program(&program).expect("compiles"),
        x3_lang_compiler::compile_program(&reparsed).expect("compiles"),
        "and mean the same thing"
    );
}

/// Every `@annotation` the parser accepts survives formatting.
///
/// The corpus check above only catches what the corpus contains, and no file in it carried an
/// annotation — so the formatter could drop every one of them, and did: `format_function` wrote
/// `async fn …` with nothing above it. `@subscribe(TransferDone)` and `@sponsor` are host calls
/// (`CALL_HOST` records in the artifact), so formatting those programs silently deleted two
/// instructions from them.
///
/// The spellings are listed here rather than read from `annotations::spelling`, because a list
/// derived from the thing under test cannot notice a variant the table forgot; the policy test
/// beside this one already checks that the table and the parser agree.
#[test]
fn formatting_preserves_every_annotation_the_parser_accepts() {
    let spellings = [
        "@no_heap",
        "@no_recursion(4)",
        "@hot",
        "@audit",
        "@role(admin)",
        "@multisig(2, 3)",
        "@version(1)",
        "@upgrade_from(0)",
        "@on_chain",
        "@off_chain",
        "@sandbox",
        "@whitelist(a, b)",
        "@concurrent",
        "@scheduled(30)",
        "@extern",
        "@payable",
        "@simd",
        "@subscribe(TransferDone)",
        "@sponsor",
        "@gas_adaptive",
    ];

    let annotations = |program: &Program| -> String {
        let item = program.items.first().expect("the fixture declares one item");
        match &item.node {
            x3_lang_ast::ast::Item::Function(function) => format!("{:?}", function.annotations),
            other => panic!("the fixture is a function, found {other:?}"),
        }
    };

    for spelling in spellings {
        let source = format!("{spelling}\nfn probe() {{ }}\n");
        let program = parse(&source).unwrap_or_else(|error| panic!("{spelling} must parse: {error}"));
        let formatted = X3Formatter::new().format_program(&program);
        let reparsed = parse(&formatted).unwrap_or_else(|error| {
            panic!("{spelling}: the formatter wrote text the parser does not read: {error}\n{formatted}")
        });
        assert_eq!(
            annotations(&program),
            annotations(&reparsed),
            "{spelling}: the formatted text must carry the same annotation:\n{formatted}"
        );
        assert!(
            formatted.contains(spelling),
            "{spelling}: the annotation must be written back, not only preserved in the AST:\n{formatted}"
        );
    }
}

/// A declaration's clauses are what `x3c fmt` is most likely to delete, twice over.
///
/// `format_bridge` wrote only the statement body: a bridge declaring a replay-protection nonce guard
/// and a refund path came back with neither, which is the defect `format_atomic_swap` had already
/// been fixed for one declaration over. And the *parser* had been throwing half of every refund
/// clause away — `refund <chain.ASSET> to <receiver>` left `to <receiver>` in the token stream, where
/// the enclosing body read it as two statements that lower to nothing, so the receiver never reached
/// the action and the formatter wrote `to;` and `sender;` back out as statements.
mod clauses_survive_formatting {
    use super::parse;
    use x3_lang_ast::ast::{FailureAction, Item, Statement};
    use x3_lang_compiler::formatter::X3Formatter;

    fn format(source: &str) -> String {
        let program = parse(source).unwrap_or_else(|error| panic!("must parse: {error}\n{source}"));
        X3Formatter::new().format_program(&program)
    }

    #[test]
    fn a_bridge_keeps_its_guards_and_its_failure_action() {
        let source = "bridge my_bridge ethereum.USDC to solana.USDC {\n    require nonce unused \
                      bridge_nonce_1\n    require slippage <= 50\n    on_fail rollback\n}\n";
        let formatted = format(source);
        for clause in [
            "require nonce unused bridge_nonce_1",
            "require slippage <= 50",
            "on_fail rollback",
        ] {
            assert!(
                formatted.contains(clause),
                "`{clause}` must be written back, not deleted: {formatted}"
            );
        }
        // And the bytes agree, which is the claim that matters: the clauses a program states are the
        // artifact it produces.
        let before = x3_lang_compiler::compile_source(source).expect("the original compiles");
        let after = x3_lang_compiler::compile_source(&formatted).expect("the formatted text compiles");
        assert_eq!(before, after, "formatting must not change the artifact: {formatted}");
    }

    #[test]
    fn a_refund_keeps_its_receiver_and_leaves_no_residue() {
        let source = "bridge my_bridge ethereum.USDC to solana.USDC {\n    on_fail refund \
                      ethereum.USDC to alice\n}\n";
        let formatted = format(source);
        assert!(
            !formatted.contains("to;") && !formatted.contains("alice;"),
            "the receiver is part of the clause, not a statement after it: {formatted}"
        );
        // The receiver survives: the re-parsed action names `alice`.
        let reparsed = parse(&formatted).expect("what the formatter writes must parse");
        let Item::Bridge(bridge) = &reparsed.items[0].node else {
            panic!("the fixture declares a bridge");
        };
        let Some(FailureAction::Refund(expression)) = &bridge.on_fail else {
            panic!("the bridge states a refund");
        };
        let text = format!("{expression:?}");
        assert!(
            text.contains("alice"),
            "the refund must still name the receiver it was written with: {text}"
        );
    }

    #[test]
    fn a_module_body_keeps_its_refund_receiver() {
        // The same clause in a statement position, which is where the corpus writes it.
        let source = "strategy S {\n    input ethereum.USDC amount 1_000\n    output ethereum.ETH\n    \
                      effects [swap]\n    domains [ethereum]\n    risk { max_slippage_bps 50 \
                      max_total_fee_bps 8 }\n    bounds { max_steps 10 max_gas 200_000 }\n    execute {\n\
                      \x20       swap uniswap ethereum.USDC -> ethereum.ETH amount 1000 min_output 1\n\
                      \x20       require slippage <= 50\n        on_fail refund ethereum.USDC to alice\n\
                      \x20   }\n}\n";
        let formatted = format(source);
        assert!(
            !formatted.contains("to;"),
            "no half-clause is left as a statement: {formatted}"
        );
        let reparsed = parse(&formatted).expect("what the formatter writes must parse");
        let Item::Strategy(module) = &reparsed.items[0].node else {
            panic!("the fixture declares a strategy");
        };
        let refund = module.body.iter().find_map(|statement| match statement {
            Statement::OnFail(FailureAction::Refund(expression)) => Some(expression),
            _ => None,
        });
        let text = format!("{:?}", refund.expect("the body states a refund"));
        assert!(text.contains("alice"), "the receiver must survive: {text}");
    }
}

/// An agent is three blocks, and the formatter used to write one.
///
/// The grammar reads a context block, then a state block, then the body — the first `{` after the
/// name is always the context. The writer put the methods into the first block, so `x3c fmt` turned
/// every agent into text the parser refuses (`context key: expected identifier`). Nothing in the
/// corpus declares an agent, which is why a round-trip test over the corpus never saw it.
mod an_agent_survives_formatting {
    use super::parse;
    use x3_lang_compiler::formatter::X3Formatter;

    #[test]
    fn the_formatted_text_parses_and_states_three_blocks() {
        let source = "agent Trader {\n    venue: \"uniswap\",\n    max_slippage: 50,\n}\n{\n    \
                      position: i64,\n}\n{\n    fn step() {\n        emit Step(1);\n    }\n}\n";
        let program = parse(source).expect("an agent parses");
        let formatted = X3Formatter::new().format_program(&program);
        assert_eq!(
            formatted.matches("\n{\n").count(),
            2,
            "a context, a state and a body are three blocks: {formatted}"
        );
        assert!(
            formatted.contains("venue: \"uniswap\"") && formatted.contains("position: i64"),
            "the blocks' content is the declaration: {formatted}"
        );
        // Formatting is compared rather than compilation: this fixture's context and state are
        // *refused* when compiled (`verify_declarations_have_a_reader` — nothing reads them), and a
        // program the compiler refuses is still one the formatter must not corrupt. Idempotence is the
        // property: what the first pass writes, the second pass leaves alone.
        let reparsed = parse(&formatted)
            .unwrap_or_else(|error| panic!("the formatter wrote text the parser refuses: {error}\n{formatted}"));
        let twice = X3Formatter::new().format_program(&reparsed);
        assert_eq!(formatted, twice, "formatting must be idempotent");
    }

    #[test]
    fn an_agent_with_empty_blocks_round_trips() {
        // An empty context is the grammar's braces rather than a claim — the parser reads the first
        // block as the context, so an agent that declares neither must still write both.
        let source = "agent Bare {\n}\n{\n}\n{\n    fn step() {\n        emit Step(1);\n    }\n}\n";
        let program = parse(source).expect("an agent with empty blocks parses");
        let formatted = X3Formatter::new().format_program(&program);
        parse(&formatted).unwrap_or_else(|error| panic!("must re-parse: {error}\n{formatted}"));
    }
}
