//! Trading Core v1 IR lowering and bytecode round-trip tests.

use x3_lang_ast::Item;
use x3_lang_compiler::emitter::{decode_trading_operation, disassemble, encode_trading_operation, trading_opcode};
use x3_lang_compiler::ir::{Operation, TradingOperation, ValueRef};
use x3_lang_compiler::parser::parse_source;
use x3_lang_compiler::semantic::CompilationMode;
use x3_lang_compiler::{analyze_trading, compile_program, compile_to_ir, lower_atomic_trade, TradingSymbols};

const SOURCE: &str = include_str!("fixtures/trading_core_v1.x3");

fn lowered_operations() -> Vec<Operation> {
    let program = parse_source(SOURCE).expect("trading fixture must parse");
    let symbols: TradingSymbols =
        analyze_trading(&program, CompilationMode::Dev).expect("trading fixture must type-check");
    let trade = program
        .items
        .iter()
        .find_map(|item| match &item.node {
            Item::AtomicTrade(trade) => Some(trade),
            _ => None,
        })
        .expect("fixture must contain one atomic trade");
    lower_atomic_trade(trade, &symbols).expect("valid trading fixture must lower")
}

#[test]
fn canonical_example_lowers_to_exact_operation_order() {
    let ops = lowered_operations();
    assert_eq!(
        ops.len(),
        9,
        "expected begin, open, swap, swap, close, guards, receipt, commit"
    );

    assert!(matches!(
        &ops[0],
        Operation::Trading(TradingOperation::BeginAtomicTrade { trade_id, policy_id })
            if trade_id == "CrossDexArb" && policy_id == "MainnetArb"
    ));
    assert!(matches!(
        &ops[1],
        Operation::Trading(TradingOperation::OpenDebt { debt_id, provider, principal, .. })
            if debt_id == "debt" && provider == "aave_v3" && *principal == 1_000_000_000_000
    ));
    assert!(matches!(
        &ops[2],
        Operation::Trading(TradingOperation::ExecuteSwap {
            binding,
            venue,
            input: ValueRef::Binding(input),
            min_output,
            ..
        }) if binding == "weth" && venue == "uniswap_v3" && input == "debt.amount"
            && *min_output == 410_000_000_000_000_000_000
    ));
    assert!(matches!(
        &ops[3],
        Operation::Trading(TradingOperation::ExecuteSwap {
            binding,
            venue,
            input: ValueRef::Binding(input),
            min_output,
            ..
        }) if binding == "returned" && venue == "sushiswap" && input == "weth"
            && *min_output == 1_002_000_000_000
    ));
    assert!(matches!(
        &ops[4],
        Operation::Trading(TradingOperation::CloseDebt { debt_id }) if debt_id == "debt"
    ));
    assert!(matches!(
        &ops[5],
        Operation::Trading(TradingOperation::AssertMinNetProfit {
            settlement_asset,
            minimum,
        }) if settlement_asset.symbol == "USDC" && *minimum == 1_000_000_000
    ));
    assert!(matches!(
        &ops[6],
        Operation::Trading(TradingOperation::AssertAllDebtsClosed)
    ));
    assert!(matches!(
        &ops[7],
        Operation::Trading(TradingOperation::EmitTradeReceipt)
    ));
    assert!(matches!(
        &ops[8],
        Operation::Trading(TradingOperation::CommitAtomicTrade)
    ));
}

#[test]
fn every_trading_operation_encodes_and_decodes_stably() {
    for op in lowered_operations() {
        let Operation::Trading(trading) = op else {
            panic!("lowered operation must be a trading operation");
        };
        let opcode = trading_opcode(&trading);
        let payload = encode_trading_operation(&trading).expect("trading op must encode");
        let decoded = decode_trading_operation(opcode, &payload).expect("trading op must decode");
        assert_eq!(decoded, trading);
    }
}

#[test]
fn compile_to_ir_contains_trading_ops_and_emits_bytecode() {
    let program = parse_source(SOURCE).expect("fixture must parse");
    let ir = compile_to_ir(&program).expect("trading program must lower to IR");
    let trading_count = ir
        .operations
        .iter()
        .filter(|op| matches!(op, Operation::Trading(_)))
        .count();
    assert_eq!(trading_count, 9);
    x3_lang_compiler::verify::verify_ir(&ir).expect("trading IR must pass structural verification");
    let bytecode = compile_program(&program).expect("trading program must emit valid bytecode");
    assert!(!bytecode.is_empty());
    assert_eq!(bytecode.len() % 4, 0);
    let trace = disassemble(&bytecode).expect("trading bytecode must disassemble");
    assert!(trace.contains("TRADING_BEGIN"));
    assert!(trace.contains("TRADING_EXECUTE_SWAP"));
    assert!(trace.contains("TRADING_COMMIT"));
}
