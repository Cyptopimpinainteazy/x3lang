#![cfg(test)]

use x3_lang_ast::ast::*;
use x3_lang_common::{BytePos, Span, Spanned, Symbol};

#[test]
fn build_and_serialize_sample_agent() {
    let span = Span::new(BytePos(0), BytePos(0), 0);
    let id = Some(NodeId(1));

    let agent = Agent {
        name: Symbol::new("ArbBot"),
        id,
        context: Some(ContextBlock {
            entries: vec![(
                Symbol::new("chains"),
                Expression::Literal(LiteralExpr::String(Symbol::new("[ethereum,solana]"))),
            )],
        }),
        state: vec![StructField {
            name: Symbol::new("profit_total"),
            ty: TypeExpr::Primitive(Symbol::new("U256")),
            visibility: Visibility::Priv,
        }],
        methods: vec![],
        strategies: vec![Spanned::new(
            StrategyDecl {
                name: Symbol::new("find_opps"),
                id: None,
                params: vec![],
                body: Block::new(vec![]),
                is_async: false,
            },
            span,
        )],
        visibility: Visibility::Pub,
        annotations: vec![],
    };

    let program = Program::new(vec![Spanned::new(Item::Agent(agent), span)]);
    // Basic walk
    let mut calls = 0usize;
    struct Visitor {
        calls: usize,
    }
    impl x3_lang_ast::visitor::AstVisitor for Visitor {
        fn visit_agent(&mut self, _a: &Agent) {
            self.calls += 1;
        }
    }

    let mut v = Visitor { calls };
    program.walk(&mut v);
    assert_eq!(v.calls, 1);
}
