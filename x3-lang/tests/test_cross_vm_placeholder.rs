use x3_lang_ast::ast::{
    AssetRef, ChainRef, Expression, Item, LiteralExpr, Program, Statement,
};
use x3_lang_common::{Span, Spanned, Symbol};
use x3_lang_compiler::{compile_to_ir, Operation};

fn lit_int(v: u128) -> Expression {
    Expression::Literal(LiteralExpr::Int { value: v, suffix: None })
}

#[test]
fn test_cross_vm_atomic_sequence_lowers_without_placeholders() {
    let program = Program::new(vec![Spanned::new(
        Item::AtomicSwap(x3_lang_ast::ast::AtomicSwapDecl {
            name: Symbol::new("dual_chain_settlement"),
            from_asset: AssetRef::new(ChainRef::new(Symbol::new("ethereum")), Symbol::new("USDC")),
            to_asset: AssetRef::new(ChainRef::new(Symbol::new("solana")), Symbol::new("SOL")),
            source_vm: None,
            dest_vm: None,
            amount: None,
            receiver: None,
            hashlock: None,
            body: vec![
                Statement::Lock {
                    chain: ChainRef(Symbol::new("ethereum")),
                    asset: AssetRef::new(ChainRef::new(Symbol::new("ethereum")), Symbol::new("USDC")),
                    amount: lit_int(500),
                    from: Expression::Literal(LiteralExpr::Address(Symbol::new("0xsender"))),
                },
                Statement::Mint {
                    asset: AssetRef::new(ChainRef::new(Symbol::new("solana")), Symbol::new("USDC")),
                    amount: lit_int(500),
                    to: Expression::Literal(LiteralExpr::Address(Symbol::new("So1Receiver"))),
                },
            ],
            requires: vec![],
            on_fail: None,
            timeout_source: Some(lit_int(20)),
            timeout_destination: None,
        }),
        Span::DUMMY,
    )]);

    let ir = compile_to_ir(&program).expect("cross-vm atomic program should lower");

    let has_atomic = ir.operations.iter().any(|op| matches!(op, Operation::AtomicBegin))
        && ir.operations.iter().any(|op| matches!(op, Operation::AtomicEnd));
    assert!(has_atomic, "atomic begin/end must be emitted");

    assert!(
        ir.operations
            .iter()
            .any(|op| matches!(op, Operation::Lock { chain, asset, amount, .. } if chain == "ethereum" && asset == "USDC" && *amount == 500)),
        "ethereum lock semantics must be preserved"
    );

    assert!(
        ir.operations
            .iter()
            .any(|op| matches!(op, Operation::Mint { chain, asset, amount, .. } if chain == "solana" && asset == "USDC" && *amount == 500)),
        "solana mint semantics must be preserved"
    );
}

