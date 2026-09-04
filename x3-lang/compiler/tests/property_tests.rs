use proptest::prelude::*;
use std::collections::HashMap;
use x3_lang_compiler::emitter::emit_x3ir;
use x3_lang_compiler::ir::{Operation, X3IR};

proptest! {
    #[test]
    fn prop_ir_operations_deterministic(
        strategy in "[a-z]{3,10}",
        score in 0u32..100,
    ) {
        let mut ir = X3IR::new();
        let mut weights = HashMap::new();
        weights.insert(strategy.clone(), score);
        ir.push(Operation::RouteScore { strategy, weights });

        let bc1 = emit_x3ir(&ir).expect("emit 1");
        let bc2 = emit_x3ir(&ir).expect("emit 2");
        assert_eq!(bc1, bc2, "IR emission must be deterministic");
    }
}

proptest! {
    #[test]
    fn prop_bytecode_alignment(
        lock_amount in 0u128..1000000,
    ) {
        let mut ir = X3IR::new();
        ir.push(Operation::Lock {
            chain: "ethereum".into(),
            asset: "USDC".into(),
            amount: lock_amount,
            from: "user".into(),
        });
        let bc = emit_x3ir(&ir).expect("emit");
        assert!(bc.len() % 4 == 0 || bc.len() % 4 == 1,
            "bytecode length {} should be 0 or 1 mod 4", bc.len());
        assert_eq!(bc[0], 0x01, "first byte must be version");
    }
}

fn arb_chain() -> impl Strategy<Value = String> {
    prop::sample::select(vec![
        "ethereum".to_string(),
        "solana".to_string(),
        "bitcoin".to_string(),
        "polygon".to_string(),
        "arbitrum".to_string(),
    ])
}

proptest! {
    #[test]
    fn prop_ir_never_panics_on_valid_input(
        chain in arb_chain(),
        name in "[A-Z]{2,8}",
        amount in 0u128..999999999,
    ) {
        let mut ir = X3IR::new();
        ir.push(Operation::Lock {
            chain: chain.clone(),
            asset: name.clone(),
            amount,
            from: "alice".into(),
        });
        ir.push(Operation::Release {
            chain: "solana".into(),
            asset: "SOL".into(),
            to: "bob".into(),
        });
        let result = emit_x3ir(&ir);
        prop_assert!(result.is_ok(), "Known chain '{}' should emit successfully", chain);
    }
}
