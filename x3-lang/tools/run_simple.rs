use x3_lang_ast::ast::*;
use x3_lang_common::Symbol;
use x3_lang_compiler::emitter;
use x3_lang_compiler::lowering::LowerCtx;
use x3_lang_compiler::lowering;
use x3_lang_vm::executor::execute;
use x3_lang_vm::{VMConfig, VM};

fn main() {
    // Build a trivial program AST
    let span = x3_lang_common::Span::DUMMY;
    let program = Program::new(vec![x3_lang_common::Spanned::new(
        Item::Function(Function {
            name: Symbol::new("main"),
            id: None,
            params: vec![],
            ret: None,
            generics: vec![],
            body: Block::new(vec![Statement::Expr(Expression::Literal(
                LiteralExpr::Int {
                    value: 42,
                    base: x3_lang_common::IntBase::Decimal,
                    suffix: None,
                },
            ))]),
            visibility: Visibility::Pub,
            is_async: false,
            annotations: vec![],
        }),
        span,
    )]);

    let lowered = lowering::lower_program(&program, LowerCtx::new()).expect("lower failed");
    let code = emitter::emit_x3ir(&lowered).expect("emit failed");

    let mut vm = VM::new(code, VMConfig::default(), 1000u128);
    let result = vm.execute();
    println!("vm exec result: {:?}", result);
}
