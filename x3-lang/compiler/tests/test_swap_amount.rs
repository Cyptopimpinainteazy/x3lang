//! `Statement::Swap`'s amount is the step's own (TICKET-067).
//!
//! The field was named `route` while holding `amount <expr>`. Three readers treated
//! it as the amount (the lowering, the formatter, the profitability check) and the
//! name said otherwise; a body-level swap, meanwhile, takes its amount from the
//! matching endpoint. These tests pin which quantity comes from where, so the two
//! sources cannot be confused again.

fn ir_of(source: &str) -> x3_lang_compiler::X3IR {
    let program = x3_lang_compiler::parser::parse_source(source).expect("the fixture must parse");
    x3_lang_compiler::compile_to_ir(&program).expect("the fixture must lower")
}

const SOURCE: &str = "intent amount_probe {\n    from ethereum.USDC amount 500\n    to ethereum.USDC\n    \
                      route {\n        swap costly_lending ethereum.USDC -> ethereum.USDC amount 1_000 \
                      min_output 2_000\n    }\n    require slippage <= 50\n    on_fail refund \
                      ethereum.USDC to sender\n}\n";

#[test]
fn the_steps_amount_reaches_the_swap_instruction() {
    let ir = ir_of(SOURCE);
    let (input_amount, min_output) = ir
        .operations
        .iter()
        .find_map(|op| match op {
            x3_lang_compiler::Operation::Swap {
                input_amount,
                min_output,
                ..
            } => Some((*input_amount, *min_output)),
            _ => None,
        })
        .expect("the route has a swap");
    assert_eq!(
        (input_amount, min_output),
        (1_000, 2_000),
        "both figures are the step's own, not the endpoint's"
    );
}

#[test]
fn the_endpoints_amount_is_a_different_quantity() {
    // The lock carries what the intent's `from` endpoint states (500), not what the
    // route step spends (1_000): the two are separate declarations, and reading one
    // as the other is what the old field name invited.
    let ir = ir_of(SOURCE);
    let locked = ir
        .operations
        .iter()
        .find_map(|op| match op {
            x3_lang_compiler::Operation::Lock { amount, .. } => Some(*amount),
            _ => None,
        })
        .expect("the intent locks its source");
    assert_eq!(locked, 500);
}

#[test]
fn the_formatter_writes_the_amount_back_where_the_parser_reads_it() {
    let program = x3_lang_compiler::parser::parse_source(SOURCE).expect("parses");
    let formatted = x3_lang_compiler::formatter::X3Formatter::new().format_program(&program);
    assert!(
        formatted.contains("amount 1000") && formatted.contains("min_output 2000"),
        "the step's clauses must survive formatting: {formatted}"
    );
    // Bytecode equality, not an AST comparison: the formatter's claim is that the
    // text it writes means what the text it read meant, and the artifact is where
    // that is decided.
    assert_eq!(
        x3_lang_compiler::compile_source(SOURCE).expect("the original compiles"),
        x3_lang_compiler::compile_source(&formatted).expect("the formatted text compiles"),
        "formatting must not change what the program does"
    );
}

#[test]
fn an_ast_naming_the_old_field_still_loads() {
    // `route` held the amount until this rename, and an AST serialized then has to
    // keep loading: the new field carries the old name as a serde alias. The
    // document is produced from the real AST and then renamed, so this test cannot
    // pass against a shape the parser never writes.
    let program = x3_lang_compiler::parser::parse_source(SOURCE).expect("parses");
    let x3_lang_ast::ast::Item::IntentDecl(intent) = &program.items[0].node else {
        panic!("the fixture is an intent");
    };
    // The route body is an `Atomic` block, so the swap is one level down.
    fn swap_in(statements: &[x3_lang_ast::ast::Statement]) -> Option<&x3_lang_ast::ast::Statement> {
        for statement in statements {
            match statement {
                x3_lang_ast::ast::Statement::Swap { .. } => return Some(statement),
                x3_lang_ast::ast::Statement::Atomic(block) => {
                    if let Some(found) = swap_in(&block.body.stmts) {
                        return Some(found);
                    }
                }
                _ => {}
            }
        }
        None
    }
    let statement = swap_in(&intent.body.stmts).expect("the body has a swap step");
    let json = serde_json::to_string(statement).expect("a statement serializes");
    let renamed = json.replace("\"amount\"", "\"route\"");
    assert_ne!(json, renamed, "the field is written as `amount` now: {json}");

    let loaded: x3_lang_ast::ast::Statement =
        serde_json::from_str(&renamed).expect("an AST naming the old field must still load");
    let x3_lang_ast::ast::Statement::Swap { amount, .. } = loaded else {
        panic!("expected a swap");
    };
    assert!(
        amount.is_some(),
        "and the old name must arrive as the amount it always was"
    );
}

/// A route swap that states no `amount`, whose `from` is the intent's source endpoint,
/// takes the endpoint's amount — the rule `Statement::Bridge` follows and the rule the
/// parser's comment on a body-level swap claims in as many words.
///
/// It did not. `fill_route_bridge_amounts` handled only `Statement::Bridge`, so a swap in
/// this shape lowered to an amount of zero and was refused two passes later with "swap
/// input_amount must be greater than zero": a report about a zero that names neither the
/// step nor the reason, three passes after the clause that was missing (TICKET-088).
#[test]
fn a_route_swap_with_no_amount_takes_the_source_endpoints() {
    const NO_AMOUNT: &str = "intent amount_probe {\n    from ethereum.USDC amount 500\n    to ethereum.ETH\n    \
                             route {\n        swap uniswap ethereum.USDC -> ethereum.ETH min_output 1\n    }\n    \
                             require slippage <= 50\n    on_fail refund ethereum.USDC to sender\n}\n";
    let ir = ir_of(NO_AMOUNT);
    let input_amount = ir
        .operations
        .iter()
        .find_map(|op| match op {
            x3_lang_compiler::ir::Operation::Swap { input_amount, .. } => Some(*input_amount),
            _ => None,
        })
        .expect("the fixture lowers to a swap");
    assert_eq!(
        input_amount, 500,
        "a step that names no amount takes the intent's source amount"
    );
}

/// The fill stops at steps that match the source. A later leg's input is what the
/// previous leg returns, which is a market outcome rather than a constant, so the
/// compiler must leave it alone — filling it from the source endpoint would be a claim
/// about a price the compiler does not have.
///
/// This is the boundary of the fix above, and it is why the second leg is still refused
/// for stating no amount rather than being quietly given the first leg's size.
#[test]
fn a_later_step_with_no_amount_is_not_filled_from_the_source() {
    const CHAINED: &str = "intent amount_probe {\n    from ethereum.USDC amount 500\n    to ethereum.USDC\n    \
                           route {\n        swap uniswap ethereum.USDC -> ethereum.ETH amount 500 min_output 1\n        \
                           swap uniswap ethereum.ETH -> ethereum.USDC min_output 500\n    }\n    \
                           require slippage <= 50\n    on_fail refund ethereum.USDC to sender\n}\n";
    let program = x3_lang_compiler::parser::parse_source(CHAINED).expect("the fixture must parse");
    // Lowering refuses the zero the second leg would otherwise carry, and it refuses it
    // rather than filling it in.
    let error = x3_lang_compiler::compile_to_ir(&program)
        .expect_err("the unfillable second leg must not lower to a zero the verifier has to catch");
    let message = format!("{error}");
    assert!(
        message.contains("cannot infer one") && message.contains("amount"),
        "the refusal must be made where the zero would have been written, and say why: {message}"
    );
    // Specifically not the two-passes-later report this used to produce.
    assert!(
        !message.contains("greater than zero"),
        "a missing clause must not be reported as a zero amount: {message}"
    );
}
