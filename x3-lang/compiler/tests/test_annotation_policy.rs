//! A modifier with no artifact form is stated as policy or reported, and never silently nothing
//! (TICKET-111).
//!
//! Seven of the twenty-one annotations this language can spell lower to nothing. Six are properties
//! of the code the artifact need not state; `@gas_adaptive` claims the program has two gas paths,
//! which the artifact *would* have to state, and it cannot — so it is reported, through the warning
//! path TICKET-104 built. This file holds the three facts that make the table trustworthy: every
//! annotation the parser can build has a row, every row names a spelling the parser accepts, and the
//! lowerer's output is non-empty exactly for the rows the table calls carried.

use x3_lang_compiler::annotations::{self, Disposition};
use x3_lang_compiler::lowering::{lower_program, LowerCtx};
use x3_lang_compiler::parser::parse_source;
use x3_lang_compiler::semantic::CompilationMode;

/// A source whose single annotation is `text`, with a body that lowers to nothing of its own.
fn program_with(text: &str) -> String {
    format!("{text}\nfn probe() {{ }}\n")
}

/// The annotation spellings the parser can build, read out of the parser itself.
///
/// A source scan for the same reason the version gate is one: a new `"name" => Ok(Annotation::…)` arm
/// is legal Rust, and its failure is an annotation that lowers to nothing with nobody having decided
/// that it should.
fn parser_spellings() -> Vec<String> {
    const PARSER: &str = include_str!("../src/parser.rs");
    // The arms live in one function and some of them are blocks (`"whitelist" => { … }`), so the scan
    // is over that function's body rather than over single-line arms — the first version of this scan
    // saw eighteen of the twenty-one spellings for exactly that reason.
    let body = PARSER
        .split_once("fn annotation_from_name_args(")
        .expect("the name map must exist")
        .1
        .split("\n}\n")
        .next()
        .expect("the map has a body");
    body.lines()
        .filter_map(|line| {
            let (name, _) = line.trim().split_once(" => ")?;
            let name = name.trim().strip_prefix('"')?.strip_suffix('"')?;
            // `"amount="` and friends are string literals *inside* arms, and an identifier-looking
            // name is what distinguishes an arm from those.
            name.chars()
                .all(|ch| ch.is_ascii_lowercase() || ch == '_')
                .then(|| name.to_string())
        })
        .collect()
}

/// The argument text each annotation needs to parse, or `None` when it takes none.
fn arguments(spelling: &str) -> &'static str {
    match spelling {
        "no_recursion" => "(4)",
        "role" => "(\"keeper\")",
        "multisig" => "(2, 3)",
        "version" => "(\"1.0\")",
        "upgrade_from" => "(\"0.9\")",
        // Each argument becomes a whitelisted name (`expr_to_string`), so the list is the arguments
        // rather than an array literal.
        "whitelist" => "(\"a\", \"b\")",
        "scheduled" => "(period=10)",
        "subscribe" => "(\"filled\")",
        _ => "",
    }
}

#[test]
fn every_annotation_the_parser_builds_has_a_row_in_the_table() {
    let spellings = parser_spellings();
    assert!(
        spellings.len() >= 20,
        "the scan must actually read the parser: it saw {spellings:?}"
    );
    let unclassified: Vec<&String> = spellings
        .iter()
        .filter(|spelling| annotations::disposition(spelling).is_none())
        .collect();
    assert!(
        unclassified.is_empty(),
        "these annotations have no row in `annotations::DISPOSITIONS`, so nothing has decided whether \
         they reach the artifact: {unclassified:?}"
    );
    // And the other direction: a row for a spelling the language cannot write is a claim about a
    // modifier no program can carry.
    let stale: Vec<&str> = annotations::DISPOSITIONS
        .iter()
        .map(|(name, _)| *name)
        .filter(|name| !spellings.iter().any(|spelling| spelling == name))
        .collect();
    assert!(
        stale.is_empty(),
        "these rows name annotations the parser cannot build: {stale:?}"
    );
}

#[test]
fn every_row_names_a_spelling_the_parser_accepts() {
    for (spelling, _) in annotations::DISPOSITIONS {
        let source = program_with(&format!("@{spelling}{}", arguments(spelling)));
        let parsed = parse_source(&source).unwrap_or_else(|error| panic!("`@{spelling}` must parse: {error}"));
        let found: Vec<&str> = parsed
            .items
            .iter()
            .flat_map(|item| match &item.node {
                x3_lang_ast::ast::Item::Function(function) => function
                    .annotations
                    .iter()
                    .map(annotations::spelling)
                    .collect::<Vec<_>>(),
                _ => Vec::new(),
            })
            .collect();
        assert_eq!(
            found,
            vec![*spelling],
            "the row's spelling must parse back to the same annotation, or a diagnostic would name a \
             modifier the language does not spell that way"
        );
    }
}

#[test]
fn the_lowerer_carries_exactly_the_annotations_the_table_says_it_does() {
    // The gate that would have caught `@gas_adaptive`'s placeholder: an annotation whose row says it
    // is invisible or reported must leave the IR *empty*, and one whose row says carried must leave
    // something behind. A record with placeholder bodies passes the second half and fails the first,
    // which is exactly what it did.
    for (spelling, disposition) in annotations::DISPOSITIONS {
        let source = program_with(&format!("@{spelling}{}", arguments(spelling)));
        let program = parse_source(&source).unwrap_or_else(|error| panic!("`@{spelling}`: {error}"));
        let ir = lower_program(&program, LowerCtx::new())
            .unwrap_or_else(|error| panic!("`@{spelling}` must lower: {error}"));
        let carried = !ir.operations.is_empty();
        match disposition {
            Disposition::Carried => assert!(
                carried,
                "`@{spelling}` is carried into the artifact by this table and left the IR empty"
            ),
            Disposition::Invisible(reason) | Disposition::Reported(reason) => assert!(
                !carried,
                "`@{spelling}` lowers to nothing ({reason}) and left {} operation(s) in the IR: {:?}",
                ir.operations.len(),
                ir.operations
            ),
        }
    }
}

#[test]
fn a_word_the_lexer_reserves_cannot_name_an_annotation() {
    // Measured: twenty of the twenty-one spellings parse and `subscription` does not, because the
    // lexer reserves that word for the `subscription <name>: … { … }` item. The name map used to carry
    // an arm for it, which no program could reach; the error now says where the construct lives.
    let error = parse_source("@subscription(amount=100, period=30)\nfn probe() { }\n")
        .expect_err("a keyword cannot name an annotation");
    let message = error.to_string();
    assert!(
        message.contains("subscription") && message.contains("begins a") && !message.contains("unknown annotation"),
        "the refusal must say what the word is for: {message}"
    );
}

#[test]
fn a_version_pair_carries_the_version_it_upgrades_from() {
    // `@upgrade_from` on its own is dropped (its row says so), and with `@version` beside it the
    // artifact states both — which is the pairing a program has to write, so it is asserted here
    // rather than left to the row's reason.
    let source = "@version(\"2.0\")\n@upgrade_from(\"1.9\")\nfn probe() { }\n".to_string();
    let program = parse_source(&source).expect("the pair must parse");
    let ir = lower_program(&program, LowerCtx::new()).expect("the pair must lower");
    let (version, upgrade_from) = ir
        .operations
        .iter()
        .find_map(|operation| match operation {
            x3_lang_compiler::ir::Operation::VersionMeta { version, upgrade_from } => {
                Some((version.clone(), upgrade_from.clone()))
            }
            _ => None,
        })
        .unwrap_or_else(|| panic!("the pair must reach the IR: {:?}", ir.operations));
    assert_eq!(version, "2.0");
    assert_eq!(upgrade_from.as_deref(), Some("1.9"));
}

#[test]
fn a_modifier_whose_claim_cannot_be_carried_is_reported_as_a_warning() {
    let source = program_with("@gas_adaptive");
    let (_, _ir, outcome) = x3_lang_compiler::check_source_diagnostics_with_mode(&source, CompilationMode::Dev)
        .expect("a program carrying `@gas_adaptive` is still a program");
    assert!(
        outcome.errors.is_empty(),
        "the finding must not reject the program: {:?}",
        outcome.errors
    );
    assert_eq!(outcome.warnings.len(), 1, "{:?}", outcome.warnings);
    let warning = outcome.warnings[0].to_string();
    assert!(
        warning.contains("X3E4026") && warning.contains("`@gas_adaptive`") && warning.contains("two gas paths"),
        "the warning must name the code, the modifier and what is lost: {warning}"
    );
}

#[test]
fn a_property_of_the_code_is_not_reported() {
    // The pair that stops "report every modifier" from passing the test above. `@payable` lowers to
    // nothing by policy, and a program that carries it is clean — with `--deny-warnings` semantics too,
    // which is what the example sweep's "warning-free" gate asserts.
    for spelling in ["payable", "no_heap", "on_chain", "concurrent"] {
        let source = program_with(&format!("@{spelling}"));
        let (_, _ir, outcome) = x3_lang_compiler::check_source_diagnostics_with_mode(&source, CompilationMode::Dev)
            .unwrap_or_else(|error| panic!("`@{spelling}` must check: {error}"));
        assert!(
            outcome.warnings.is_empty(),
            "`@{spelling}` is stated policy and must not warn: {:?}",
            outcome.warnings
        );
    }
}
