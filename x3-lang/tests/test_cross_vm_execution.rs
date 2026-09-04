use x3_lang_compiler::{compile_program, compile_to_ir, FailureAction, Operation, RequireKind};
use x3_lang_ast::ast::{
    AssetRef, Block, BridgeDecl, ChainRef, Expression, FailureAction as AstFailureAction,
    Item, ItemNode, LiteralExpr, Program, RequireGuard, RequireKind as AstRequireKind, Statement,
};
use x3_lang_common::{Span, Spanned, Symbol};

fn spanned_item(node: Item) -> ItemNode {
    Spanned::new(node, Span::DUMMY)
}

fn lit_int(v: u128) -> Expression {
    Expression::Literal(LiteralExpr::Int {
        value: v,
        suffix: None,
    })
}

#[test]
fn test_cross_vm_bridge_lowers_semantics() {
    let bridge = BridgeDecl {
        name: Symbol::new("sol_to_eth"),
        from_asset: AssetRef::new(ChainRef::new(Symbol::new("solana")), Symbol::new("USDC")),
        to_asset: AssetRef::new(ChainRef::new(Symbol::new("ethereum")), Symbol::new("USDC")),
        body: vec![Statement::Lock {
            chain: Symbol::new("solana"),
            asset: AssetRef::new(ChainRef::new(Symbol::new("solana")), Symbol::new("USDC")),
            amount: lit_int(250),
            from: Expression::Literal(LiteralExpr::Address(Symbol::new("sender"))),
        }],
        requires: vec![RequireGuard {
            kind: AstRequireKind::Finality,
            subject: Some(Symbol::new("ethereum")),
            value: lit_int(64),
        }],
        on_fail: Some(AstFailureAction::Refund(Expression::Literal(LiteralExpr::String(
            Symbol::new("solana.USDC:sender"),
        )))),
        timeout: Some(lit_int(30)),
    };

    let program = Program::new(vec![spanned_item(Item::Bridge(bridge))]);
    let ir = compile_to_ir(&program).expect("bridge should lower to IR");

    assert!(ir.operations.iter().any(|op| matches!(op, Operation::AtomicBegin)));
    assert!(ir.operations.iter().any(|op| {
        matches!(
            op,
            Operation::Require {
                kind: RequireKind::Finality,
                ..
            }
        )
    }));
    assert!(ir.operations.iter().any(|op| {
        matches!(
            op,
            Operation::OnFail {
                action: FailureAction::Refund { chain, asset, to }
            } if chain == "solana" && asset == "USDC" && to == "sender"
        )
    }));
    assert!(ir.operations.iter().any(|op| {
        matches!(
            op,
            Operation::OnTimeout {
                duration_blocks: 30,
                ..
            }
        )
    }));

    let bytecode = compile_program(&program).expect("bridge program should compile to bytecode");
    assert_eq!(bytecode[0], 0x01);
    assert_eq!(bytecode.len() % 4, 0);
}

#[test]
fn test_cross_vm_invalid_multisig_fails_with_semantic_error() {
    use x3_lang_ast::ast::{Annotation, FunctionDecl};

    let func = FunctionDecl {
        name: Symbol::new("guarded"),
        visibility: None,
        constness: false,
        asyncness: false,
        generic_params: vec![],
        params: vec![],
        ret_type: None,
        body: Block { stmts: vec![] },
        annotations: vec![Annotation::Multisig(3, 2)],
    };

    let program = Program::new(vec![spanned_item(Item::Function(func))]);
    let err = compile_to_ir(&program).expect_err("invalid multisig must fail");
    let msg = err.to_string();
    assert!(msg.contains("multisig") || msg.contains("Semantic error"));
}

#[test]
fn test_cross_vm_source_compiles_and_executes() {
    let src = include_str!("../examples/mainnet_safe_swap.x3");
    let bytecode = x3_lang_compiler::compile_source(src).expect("mainnet_safe_swap should compile");
    assert_eq!(bytecode[0], 0x01, "bytecode version");
    assert_eq!(bytecode.len() % 4, 0, "4-byte aligned");
    let vm = x3_lang_vm::VM::new(
        x3_lang_vm::InstructionStream::new(bytecode),
        x3_lang_vm::VMConfig::default(),
        1_000_000,
    );
    let result = vm.execute();
    assert!(result.is_ok(), "VM execution should succeed");
}

#[test]
fn test_cross_vm_flagship_parses_and_lowers() {
    let src = include_str!("../examples/flagship_b52.x3");
    let result = x3_lang_compiler::check_source(src);
    assert!(result.is_ok(), "flagship_b52 should parse and lower to IR");
    let (_program, ir, _warnings) = result.unwrap();
    assert!(!ir.operations.is_empty(), "IR should contain operations");
    assert!(ir.operations.iter().any(|op| matches!(op, Operation::Lock { .. })), "should contain at least one Lock");
}
