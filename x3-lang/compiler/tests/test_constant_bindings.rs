use x3_lang_compiler::{compile_source, compile_to_ir, parser::parse_source, Operation};

fn source(body: &str) -> String {
    format!("fn main() {{ atomic {{ {body} release ethereum.USDC to sender; }} }}")
}

#[test]
fn immutable_amount_bindings_reach_real_bytecode() {
    let bound = source("let a = 5; let b = a + 2; lock ethereum.USDC amount b from sender;");
    let inline = source("lock ethereum.USDC amount 7 from sender;");
    let ir = compile_to_ir(&parse_source(&bound).unwrap()).expect("binding must lower");
    assert!(ir
        .operations
        .iter()
        .any(|op| matches!(op, Operation::Lock { amount: 7, .. })));
    assert_eq!(compile_source(&bound).unwrap(), compile_source(&inline).unwrap());
    let mut vm = x3_lang_vm::VM::new(
        compile_source(&bound).unwrap(),
        x3_lang_vm::VMConfig::default(),
        100_000,
    );
    vm.execute().expect("emitted binding amount must execute");
    assert!(vm
        .state
        .asset_ops
        .iter()
        .any(|op| matches!(op, x3_lang_common::AssetOpPayload::Lock { amount: 7, .. })));
}

#[test]
fn bindings_preserve_scope_and_shadowing() {
    let bound = source("let a = 5; if true { let a = a + 2; lock ethereum.USDC amount a from sender; } lock ethereum.USDC amount a from sender;");
    let ir = compile_to_ir(&parse_source(&bound).unwrap()).unwrap();
    let text = serde_json::to_string(&ir).unwrap();
    assert!(text.contains("\"amount\":7") && text.contains("\"amount\":5"));
    let mut vm = x3_lang_vm::VM::new(
        compile_source(&bound).unwrap(),
        x3_lang_vm::VMConfig::default(),
        100_000,
    );
    vm.execute().unwrap();
    let amounts: Vec<_> = vm
        .state
        .asset_ops
        .iter()
        .filter_map(|op| match op {
            x3_lang_common::AssetOpPayload::Lock { amount, .. } => Some(*amount),
            _ => None,
        })
        .collect();
    assert_eq!(amounts, vec![7, 5]);
}

#[test]
fn unsafe_or_unresolved_bindings_are_refused() {
    for body in [
        "let a = 1 / 0;",
        "let a = 0 - 1;",
        "let a = 340282366920938463463374607431768211455 + 1;",
        "let mut a = 1;",
        "let a = host_price();",
        "let a = 1; emit Event(a);",
    ] {
        assert!(compile_source(&source(body)).is_err(), "must refuse: {body}");
    }
}

#[test]
fn explicit_u128_bindings_keep_full_width_values() {
    let bound = source(&format!(
        "let amount: u128 = {}; lock ethereum.USDC amount amount from sender;",
        u128::MAX
    ));
    let inline = source(&format!("lock ethereum.USDC amount {} from sender;", u128::MAX));
    assert_eq!(compile_source(&bound).unwrap(), compile_source(&inline).unwrap());
    let mut vm = x3_lang_vm::VM::new(
        compile_source(&bound).unwrap(),
        x3_lang_vm::VMConfig::default(),
        100_000,
    );
    vm.execute().unwrap();
    assert!(vm
        .state
        .asset_ops
        .iter()
        .any(|op| matches!(op, x3_lang_common::AssetOpPayload::Lock { amount, .. } if *amount == u128::MAX)));
}

#[test]
fn compact_literal_arithmetic_matches_spaced_arithmetic() {
    for (compact, spaced) in [
        ("10-2", "10 - 2"),
        ("10u128-2u128", "10u128 - 2u128"),
        ("0xau128-2u128", "0xau128 - 2u128"),
        ("10--2", "10 - -2"),
    ] {
        let compact = parse_source(&source(&format!("let amount = {compact};"))).unwrap();
        let spaced = parse_source(&source(&format!("let amount = {spaced};"))).unwrap();
        assert_eq!(
            x3_lang_compiler::formatter::X3Formatter::new().format_program(&compact),
            x3_lang_compiler::formatter::X3Formatter::new().format_program(&spaced),
            "spacing must not change the parsed expression"
        );
    }
    let bound = source("let amount = 10-2; lock ethereum.USDC amount amount from sender;");
    let inline = source("lock ethereum.USDC amount 8 from sender;");
    assert_eq!(compile_source(&bound).unwrap(), compile_source(&inline).unwrap());
}

#[test]
fn bitwise_amounts_execute_with_full_width_checked_shifts() {
    for (expression, expected) in [
        ("13 & 7", 5u128),
        ("8 | 3", 11),
        ("13 ^ 7", 10),
        ("1 << 127", 1u128 << 127),
        ("340282366920938463463374607431768211455 << 0", u128::MAX),
        ("340282366920938463463374607431768211455 >> 127", 1),
        ("(0 << 127) + 1", 1),
        ("1 << 2 + 1", 8),
        ("1 | 2 ^ 3 & 1", 3),
        ("(1 | 2) & 1", 1),
        ("10 - (3 - 1)", 8),
        ("2 * (3 + 1)", 8),
    ] {
        let bound = source(&format!(
            "let amount = {expression}; lock ethereum.USDC amount amount from sender;"
        ));
        let inline = source(&format!("lock ethereum.USDC amount {expected} from sender;"));
        let bytecode = compile_source(&bound).unwrap_or_else(|e| panic!("{expression}: {e}"));
        assert_eq!(bytecode, compile_source(&inline).unwrap(), "{expression}");
        let formatted = x3_lang_compiler::formatter::X3Formatter::new().format_program(&parse_source(&bound).unwrap());
        assert_eq!(bytecode, compile_source(&formatted).unwrap(), "formatting {expression}");
        let mut vm = x3_lang_vm::VM::new(bytecode, x3_lang_vm::VMConfig::default(), 100_000);
        vm.execute().unwrap();
        assert!(vm
            .state
            .asset_ops
            .iter()
            .any(|op| matches!(op, x3_lang_common::AssetOpPayload::Lock { amount, .. } if *amount == expected)));
    }
    for expression in [
        "2 << 127",
        "1 << 128",
        "1 >> 128",
        "1 << 340282366920938463463374607431768211455",
        "1 < < 2",
        "1 << -1",
    ] {
        assert!(
            compile_source(&source(&format!("let amount = {expression};"))).is_err(),
            "must refuse {expression}"
        );
    }
}
