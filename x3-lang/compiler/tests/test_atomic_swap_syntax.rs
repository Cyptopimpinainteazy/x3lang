//! # X3-lang Atomic Swap Syntax Test
//!
//! Tests the `atomic swap` declaration syntax for the X3 cross-VM atomic swap
//! feature. The parser already supports the full syntax via
//! [`parse_atomic_swap_item_new`](x3-lang/compiler/src/parser.rs:620).
//!
//! ## Syntax
//!
//! ```x3
//! atomic swap eth.USDC -> sol.SOL {
//!     amount 500
//!     receiver sol.wallet.owner
//!     hashlock blake2b(secret)
//!     timeout source 40m
//!     timeout destination 20m
//!     require finality.eth >= 12
//!     require finality.sol == finalized
//!     require relayer_quorum >= 3
//! }
//! ```

use x3_lang_compiler::parser::parse_source;

#[test]
fn test_atomic_swap_syntax_basic() {
    let source = r#"
atomic swap eth.USDC -> sol.SOL {
    amount 500
    receiver sol.wallet.owner
    hashlock blake2b(secret)
    timeout source 2400
    timeout destination 1200
    require finality.eth >= 12
    require finality.sol == finalized
    require relayer_quorum >= 3
}
"#;

    let program = parse_source(source).expect("atomic swap syntax should parse");
    assert_eq!(program.items.len(), 1, "should have one declaration");
}

#[test]
fn test_atomic_swap_syntax_with_slippage() {
    let source = r#"
atomic swap btc.BTC -> eth.WBTC {
    amount 1
    receiver 0xRecipientAddress
    hashlock sha256(my_secret)
    timeout source 3600
    timeout destination 1800
    require finality.btc >= 6
    require finality.eth >= 12
    require slippage <= 50
}
"#;

    let program = parse_source(source).expect("atomic swap with slippage should parse");
    assert_eq!(program.items.len(), 1);
}

#[test]
fn test_atomic_swap_syntax_minimal() {
    let source = r#"
atomic swap x3.USDC -> sol.SOL {
    amount 1000
    receiver sol_receiver
    timeout source 2000
    timeout destination 1000
}
"#;

    let program = parse_source(source).expect("minimal atomic swap should parse");
    assert_eq!(program.items.len(), 1);
}

#[test]
fn test_atomic_swap_requires_valid_expression() {
    let source = r#"
atomic swap eth.DAI -> arb.USDC {
    amount 5000
    receiver arb_user
    hashlock sha256(secret_value)
    timeout source 3000
    timeout destination 1500
    require finality.eth >= 12
    require relayer_quorum >= 3
}
"#;

    let program = parse_source(source).expect("atomic swap with detailed requires should parse");
    assert_eq!(program.items.len(), 1);
}
