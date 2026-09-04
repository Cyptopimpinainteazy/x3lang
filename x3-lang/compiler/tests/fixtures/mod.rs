#![allow(dead_code)]

// E2E test fixtures (target H — "Finish Examples").
//
// These cover the documented production-shape intent surface that the
// Rust parser accepts today. Each fixture is paired with a test in
// tests/test_e2e_fixtures.rs that asserts:
//   1. the source parses without error,
//   2. the source lowers to a known IR shape,
//   3. the IR compiles to 4-byte aligned bytecode,
//   4. the bytecode verifies and runs on the dry-run VM,
//   5. (for invalid fixtures) the semantic verifier rejects the program.

/// Transfer: a single chain internal asset move. Wrapped in an atomic
/// block (the parser always wraps `route {}` in `Statement::Atomic`).
pub const TRANSFER: &str = r#"intent internal_transfer {
    from Ethereum.USDC amount 100 receiver 0x1111111111111111111111111111111111111111
    to Solana.USDC receiver 4Nd1mzi8Y1QYxJt9wZWBYZpG7S4pYkZs6YzD3Vt9aBcD
    route {
        swap uniswap ethereum.USDC -> ethereum.ETH amount 1000 min_output 777
    }
    timeout 30s refund ethereum.USDC to sender
    on_fail rollback
}
"#;

/// Atomic swap: bridge + swap on the destination chain. Exercises the
/// cross-VM path under a single atomic block with a refund policy.
pub const ATOMIC_SWAP: &str = r#"intent swap_demo {
    from Ethereum.USDC amount 100 receiver 0x1111111111111111111111111111111111111111
    to Solana.USDC receiver 4Nd1mzi8Y1QYxJt9wZWBYZpG7S4pYkZs6YzD3Vt9aBcD
    route {
        bridge X3 ethereum.USDC -> solana.USDC receiver 4Nd1mzi8Y1QYxJt9wZWBYZpG7S4pYkZs6YzD3Vt9aBcD
        swap uniswap solana.USDC -> solana.ETH amount 1000 min_output 500
    }
    require nonce unused swap_demo_2026_06_06
    timeout 30s refund ethereum.USDC to sender
    on_fail rollback
}
"#;

/// EVM call: route through a pure EVM chain pair. Used to exercise the
/// Ethereum adapter path.
pub const EVM_CALL: &str = r#"intent evm_call {
    from Ethereum.USDC amount 100 receiver 0x1111111111111111111111111111111111111111
    to Polygon.USDC receiver 0x2222222222222222222222222222222222222222
    route {
        bridge X3 ethereum.USDC -> polygon.USDC receiver 0x2222222222222222222222222222222222222222
    }
    require nonce unused evm_call_2026_06_06
    timeout 30s refund ethereum.USDC to sender
    on_fail rollback
}
"#;

/// X3VM internal call: the destination is the X3 chain itself.
pub const X3_CALL: &str = r#"intent x3_call {
    from Ethereum.USDC amount 100 receiver 0x1111111111111111111111111111111111111111
    to X3.USDC receiver x3-receiver-1234
    route {
        bridge X3 ethereum.USDC -> x3.USDC receiver x3-receiver-1234
    }
    require nonce unused x3_call_2026_06_06
    timeout 30s refund ethereum.USDC to sender
    on_fail rollback
}
"#;

/// BTC/UTXO route. The X3 side of the route uses a real bridge; the
/// adapter is feature-gated and fails closed in production builds.
pub const BTC_ROUTE: &str = r#"intent btc_route {
    from Bitcoin.BTC amount 1 receiver bc1qexampleexampleexampleexampleexampleexampleexample
    to Ethereum.WBTC receiver 0x1111111111111111111111111111111111111111
    route {
        bridge X3 bitcoin.BTC -> ethereum.WBTC receiver 0x1111111111111111111111111111111111111111
    }
    require nonce unused btc_route_2026_06_06
    timeout 30s refund bitcoin.BTC to sender
    on_fail rollback
}
"#;

/// Invalid route: the source and destination chains are identical on
/// a bridge. The semantic verifier rejects this because a cross-VM
/// bridge must move value between two distinct chains.
pub const INVALID_ROUTE: &str = r#"intent invalid_route {
    from Ethereum.USDC amount 100 receiver 0x1111111111111111111111111111111111111111
    to Ethereum.USDC receiver 0x2222222222222222222222222222222222222222
    route {
        bridge X3 ethereum.USDC -> ethereum.USDC receiver 0x2222222222222222222222222222222222222222
    }
}
"#;

/// Unknown chain: exercises the adapter allow-list rejection. The
/// semantic verifier must refuse to route through an unknown chain.
pub const UNKNOWN_CHAIN: &str = r#"intent unknown_chain {
    from MyChain.USDC amount 100 receiver 0x1111111111111111111111111111111111111111
    to Solana.USDC receiver 4Nd1mzi8Y1QYxJt9wZWBYZpG7S4pYkZs6YzD3Vt9aBcD
    route {
        swap uniswap mychain.USDC -> ethereum.ETH amount 1000 min_output 777
    }
}
"#;

/// Malformed input: an obviously broken intent that the parser must
/// reject with a useful diagnostic.
pub const MALFORMED: &str = r#"this is not a valid program @#$%"#;

/// Refund swap: a simple intent with timeout and refund path.
#[allow(dead_code)]
pub const REFUND_SWAP_SOURCE: &str = include_str!("refund_swap.x3");

/// Mainnet safe swap: full production intent with vm/solver/relayer/rpc config.
#[allow(dead_code)]
pub const MAINNET_SAFE_SWAP_SOURCE: &str = include_str!("mainnet_safe_swap.x3");

/// Intent with proofs: intent that requires multiple proof types.
#[allow(dead_code)]
pub const INTENT_WITH_PROOFS_SOURCE: &str = include_str!("intent_with_proofs.x3");
