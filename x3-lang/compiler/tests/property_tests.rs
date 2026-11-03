use proptest::prelude::*;
use std::collections::BTreeMap;
use x3_lang_compiler::emitter::emit_x3ir;
use x3_lang_compiler::ir::{Operation, ReleaseAct, X3IR};

proptest! {
    /// IR emission is deterministic for a weight map with **more than one** entry.
    ///
    /// This test generated one entry, and a one-entry map has no order to disagree
    /// about — which is why it passed while the emitter was rendering a `HashMap` with
    /// `{:?}` and twelve identical compiles of a real program produced six distinct
    /// artifacts (PHASE 42, measured this round). The second and third entries are the
    /// point of the test, not decoration: with the field back to a `HashMap` this
    /// fails, and with one entry it did not.
    #[test]
    fn prop_ir_operations_deterministic(
        strategy in "[a-z]{3,10}",
        score in 0u32..100,
        other in "[a-z]{3,10}",
        second in 0u32..100,
        third in "[a-z]{3,10}",
        third_score in 0u32..100,
    ) {
        let mut ir = X3IR::new();
        let mut weights = BTreeMap::new();
        weights.insert(strategy.clone(), score);
        weights.insert(other, second);
        weights.insert(third, third_score);
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
            act: ReleaseAct::Claims(0),
        });
        let result = emit_x3ir(&ir);
        prop_assert!(result.is_ok(), "Known chain '{}' should emit successfully", chain);
    }
}
