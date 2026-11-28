use x3_lang_ast::ast::Item;
use x3_lang_common::liquidation::LiquidationPolicy;
use x3_lang_common::{decode_capability_payload, encode_capability_payload, CapabilityPayload};
use x3_lang_compiler::emitter::{emit_x3ir, instructions};
use x3_lang_compiler::ir::{Condition, Operation, X3IR};
use x3_lang_compiler::spec::opcodes::{BYTECODE_VERSION_3, HALT, NATIVE_LIQUIDATION};
use x3_lang_compiler::{compile_source, parser::parse_source};

fn policy() -> LiquidationPolicy {
    let program = parse_source(&source()).unwrap();
    let Item::NativeLiquidation(decl) = &program.items[0].node else {
        panic!("native declaration")
    };
    decl.policy.clone()
}

fn source() -> String {
    format!(
        r#"native_liquidation BaseKeeper {{
    chain_id: 8453;
    executor: "0x{}";
    executor_code_hash: "0x{}";
    keeper: "0x{}";
    owner: "0x{}";
    morpho: "0x{}";
    loan_token: "0x{}";
    markets: ["0x{}"];
    routers: ["0x{}"];
    min_repay: 1_000_000_000;
    max_repay: 10_000_000_000;
    min_profit: 1_000_000;
    max_gas: 1_000_000;
    max_fee_per_gas: 2_000_000_000;
    max_priority_fee_per_gas: 1_000_000;
    max_total_fee_wei: 3_000_000_000_000_000;
    max_age_blocks: 2;
    expires_at_block: 70_000_000;
    max_candidates: 64;
}}
"#,
        "11".repeat(20),
        "22".repeat(32),
        "33".repeat(20),
        "44".repeat(20),
        "55".repeat(20),
        "66".repeat(20),
        "77".repeat(32),
        "88".repeat(20),
    )
}

#[test]
fn compiles_a_standalone_native_liquidation_policy() {
    let bytes = compile_source(&source()).expect("a complete native policy must compile");
    assert_eq!(bytes[0], BYTECODE_VERSION_3);
    let records = instructions(&bytes).unwrap();
    assert_eq!(
        records.iter().map(|record| record.opcode).collect::<Vec<_>>(),
        [NATIVE_LIQUIDATION, HALT]
    );
    let decoded = decode_capability_payload(records[0].opcode, records[0].payload).unwrap();
    assert_eq!(decoded, CapabilityPayload::NativeLiquidation { policy: policy() });
    let mut downgraded = bytes.clone();
    downgraded[0] = 2;
    assert!(
        instructions(&downgraded).is_err(),
        "native liquidation cannot hide in a version-2 artifact"
    );
}

#[test]
fn source_formatter_preserves_the_complete_policy() {
    let parsed = parse_source(&source()).unwrap();
    let formatted = x3_lang_compiler::formatter::X3Formatter::new().format_program(&parsed);
    assert_eq!(compile_source(&source()).unwrap(), compile_source(&formatted).unwrap());
}

#[test]
fn native_effect_cannot_be_mixed_with_other_source_or_ir_operations() {
    for mixed in [
        format!("{}\nfn after() {{}}", source()),
        format!("fn before() {{}}\n{}", source()),
        format!("{}\n{}", source(), source()),
        format!("mod hidden {{ {} }}", source()),
    ] {
        assert!(
            compile_source(&mixed).is_err(),
            "a terminal effect cannot be composed: {mixed}"
        );
    }
    for operations in [
        vec![Operation::NativeLiquidation { policy: policy() }, Operation::Nop],
        vec![Operation::If {
            condition: Condition::True,
            then_ops: vec![Operation::NativeLiquidation { policy: policy() }],
            else_ops: None,
        }],
    ] {
        let mut ir = X3IR::new();
        ir.operations = operations;
        assert!(
            x3_lang_compiler::verify::verify_ir(&ir).is_err(),
            "structural IR verification must reject composed native effects"
        );
        assert!(
            emit_x3ir(&ir).is_err(),
            "the public emitter must also refuse composed native effects"
        );
    }
}

#[test]
fn every_policy_field_is_mandatory_and_unique() {
    let complete = source();
    for field_line in complete.lines().filter(|line| line.contains(':')) {
        let missing = complete.replace(field_line, "");
        assert!(
            compile_source(&missing).is_err(),
            "missing field accepted: {field_line}"
        );
        let duplicate = complete.replace(field_line, &format!("{field_line}\n{field_line}"));
        assert!(
            compile_source(&duplicate).is_err(),
            "duplicate field accepted: {field_line}"
        );
    }
}

#[test]
fn parser_refuses_unknown_fields_wrong_types_and_integer_overflow() {
    for (from, to) in [
        ("max_candidates: 64", "max_candidates: 4294967296"),
        ("max_gas: 1_000_000", "max_gas: 18446744073709551616"),
        ("chain_id: 8453", "chain_id: -1"),
        ("chain_id: 8453", "chain_id: \"8453\""),
        ("chain_id: 8453", "chain_id: 1.5"),
        (
            "max_candidates: 64",
            "max_candidates: 340282366920938463463374607431768211456",
        ),
        ("max_candidates: 64", "max_candidates: candidate_limit"),
        ("max_candidates: 64", "max_candidates: 64; unsupported: 1"),
        ("markets: [", "markets: [1, "),
        ("chain_id: 8453;", "chain_id: 8453"),
    ] {
        let invalid = source().replace(from, to);
        assert!(compile_source(&invalid).is_err(), "unsupported source accepted: {to}");
    }
}

#[test]
fn shared_validation_rejects_invalid_bounds_and_identities() {
    let cases: Vec<(&str, fn(&mut LiquidationPolicy))> = vec![
        ("zero chain", |p| p.chain_id = 0),
        ("zero repay floor", |p| p.min_repay = 0),
        ("zero repay ceiling", |p| p.max_repay = 0),
        ("inverted repay", |p| p.max_repay = p.min_repay - 1),
        ("zero profit", |p| p.min_profit = 0),
        ("zero gas", |p| p.max_gas = 0),
        ("zero max fee", |p| p.max_fee_per_gas = 0),
        ("priority exceeds max", |p| {
            p.max_priority_fee_per_gas = p.max_fee_per_gas + 1
        }),
        ("zero total fee", |p| p.max_total_fee_wei = 0),
        ("zero expiry", |p| p.expires_at_block = 0),
        ("old evidence", |p| p.max_age_blocks = 65),
        ("zero candidates", |p| p.max_candidates = 0),
        ("too many candidates", |p| p.max_candidates = 65),
        ("empty markets", |p| p.markets.clear()),
        ("empty routers", |p| p.routers.clear()),
        ("duplicate market", |p| p.markets.push(p.markets[0].clone())),
        ("duplicate router", |p| p.routers.push(p.routers[0].clone())),
        ("too many markets", |p| {
            p.markets = (1..=33).map(|n| format!("0x{n:064x}")).collect()
        }),
        ("too many routers", |p| {
            p.routers = (1..=33).map(|n| format!("0x{n:040x}")).collect()
        }),
        ("zero executor", |p| p.executor = format!("0x{}", "00".repeat(20))),
        ("short keeper", |p| p.keeper.truncate(41)),
        ("uppercase owner", |p| p.owner = format!("0x{}", "AB".repeat(20))),
        ("missing morpho prefix", |p| p.morpho = "55".repeat(20)),
        ("bad loan hex", |p| p.loan_token = format!("0x{}", "zz".repeat(20))),
        ("zero code hash", |p| {
            p.executor_code_hash = format!("0x{}", "00".repeat(32))
        }),
        ("short market", |p| p.markets[0] = "0x01".into()),
        ("zero router", |p| p.routers[0] = format!("0x{}", "00".repeat(20))),
    ];
    for (name, mutate) in cases {
        let mut invalid = policy();
        mutate(&mut invalid);
        assert!(invalid.validate().is_err(), "validator accepted {name}");
        assert!(
            encode_capability_payload(&CapabilityPayload::NativeLiquidation { policy: invalid }).is_err(),
            "encoder accepted {name}"
        );
    }
    let mut same_block_no_tip = policy();
    same_block_no_tip.max_age_blocks = 0;
    same_block_no_tip.max_priority_fee_per_gas = 0;
    same_block_no_tip
        .validate()
        .expect("same-block evidence and zero priority fee are valid");
}

#[test]
fn binary_codec_preserves_wide_units_and_refuses_malformed_records() {
    let wide_source = source().replace("min_profit: 1_000_000", &format!("min_profit: {}", u128::MAX));
    let compiled = compile_source(&wide_source).unwrap();
    let records = instructions(&compiled).unwrap();
    let decoded = decode_capability_payload(NATIVE_LIQUIDATION, records[0].payload).unwrap();
    let CapabilityPayload::NativeLiquidation { policy: wide } = decoded else {
        panic!("typed policy")
    };
    assert_eq!(wide.min_profit, u128::MAX);
    assert_eq!(wide.min_repay, 1_000_000_000);
    let bytes = encode_capability_payload(&CapabilityPayload::NativeLiquidation { policy: wide.clone() }).unwrap();
    assert_eq!(
        decode_capability_payload(NATIVE_LIQUIDATION, &bytes).unwrap(),
        CapabilityPayload::NativeLiquidation { policy: wide }
    );
    for end in 0..bytes.len() {
        assert!(
            decode_capability_payload(NATIVE_LIQUIDATION, &bytes[..end]).is_err(),
            "truncated payload accepted at {end}"
        );
    }
    let mut trailing = bytes.clone();
    trailing.push(0);
    assert!(decode_capability_payload(NATIVE_LIQUIDATION, &trailing).is_err());
    let mut zero_chain = bytes.clone();
    zero_chain[..8].fill(0);
    assert!(decode_capability_payload(NATIVE_LIQUIDATION, &zero_chain).is_err());
    let mut bad_utf8 = bytes.clone();
    bad_utf8[10] = 0xff;
    assert!(decode_capability_payload(NATIVE_LIQUIDATION, &bad_utf8).is_err());
    let mut zero_candidates = bytes.clone();
    let len = zero_candidates.len();
    zero_candidates[len - 4..].fill(0);
    assert!(decode_capability_payload(NATIVE_LIQUIDATION, &zero_candidates).is_err());
    let mut oversized_array = bytes;
    // u64 chain, five address strings (2-byte length +42), one code hash (2+66).
    let markets_count_offset = 8 + 5 * 44 + 68;
    oversized_array[markets_count_offset..markets_count_offset + 2].copy_from_slice(&33u16.to_le_bytes());
    assert!(decode_capability_payload(NATIVE_LIQUIDATION, &oversized_array).is_err());
}
