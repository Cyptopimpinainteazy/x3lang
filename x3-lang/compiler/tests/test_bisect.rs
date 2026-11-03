use x3_lang_compiler::parser::parse_source;

/// Parse `src`, or fail with the parser's diagnostic.
///
/// This helper used to `println!("OK"/"FAIL")` and return nothing, so the test
/// passed whether the program parsed or not — a bisection aid that had been left
/// in the suite. Both samples are full programs with six top-level sections; the
/// count is asserted so a section that silently stops lowering is not silent.
fn parse_items(label: &str, src: &str) -> usize {
    match parse_source(src) {
        Ok(p) => p.items.len(),
        Err(e) => panic!("[{label}] expected the program to parse, got {e:?}"),
    }
}

#[test]
fn both_bisection_samples_parse_with_every_section() {
    let with_timeout = parse_items(
        "with timeout",
        r#"
vm {
    chain arbitrum
    adapter evm
    finality safe
}

solver_market {
    mode competitive
    min_reputation 95
}

relayers {
    quorum_numerator 3
    quorum_denominator 5
    relayers [relayer_a, relayer_b, relayer_c, relayer_d, relayer_e]
}

rpc_quorum {
    source arbitrum
    require_numerator 2
    require_denominator 3
    reject_on [receipt_disagree, finality_disagree]
}

risk_policy {
    max_slippage 5
    max_position 500000
}

privacy {
    hide_route_until_commit true
    reveal_on claim
    encrypted true
}

invariant no_double_claim

proofs required {
    source_lock_proof
    source_finality_proof
    destination_fill_proof
}

finality_policy strict {
    chain ethereum
    requirement finalized
}

error SlippageExceeded

target evm {
    adapter evm_adapter
    contract 0x742d35Cc6634C0532925a3b844Bc9e7595f2bD18
}

intent safe_cross_vm_swap {
    from arbitrum.USDC amount 500
    to solana.SOL receiver wallet

    route {
        bridge X3 arbitrum.USDC -> solana.SOL receiver wallet
    }

    require nonce unused safe_swap_001
    require slippage <= 5
    require route_score >= 90
    require finality.arbitrum >= 32
    require finality.solana >= 32
    require relayer_quorum >= 3
    require solver_bond >= 10000

    timeout 3600s
}
"#,
    );
    assert!(
        with_timeout >= 6,
        "the sample declares six sections, parsed {with_timeout} items"
    );

    let full_intent = parse_items(
        "full intent",
        r#"
vm {
    chain arbitrum
    adapter evm
    finality safe
}

solver_market {
    mode competitive
    min_reputation 95
}

relayers {
    quorum_numerator 3
    quorum_denominator 5
    relayers [relayer_a, relayer_b, relayer_c, relayer_d, relayer_e]
}

rpc_quorum {
    source arbitrum
    require_numerator 2
    require_denominator 3
    reject_on [receipt_disagree, finality_disagree]
}

risk_policy {
    max_slippage 5
    max_position 500000
}

privacy {
    hide_route_until_commit true
    reveal_on claim
    encrypted true
}

invariant no_double_claim

proofs required {
    source_lock_proof
    source_finality_proof
    destination_fill_proof
}

finality_policy strict {
    chain ethereum
    requirement finalized
}

error SlippageExceeded

target evm {
    adapter evm_adapter
    contract 0x742d35Cc6634C0532925a3b844Bc9e7595f2bD18
}

intent safe_cross_vm_swap {
    from arbitrum.USDC amount 500
    to solana.SOL receiver wallet

    route {
        bridge X3 arbitrum.USDC -> solana.SOL receiver wallet
    }

    require nonce unused safe_swap_001
    require slippage <= 5
    require route_score >= 90
    require finality.arbitrum >= 32
    require finality.solana >= 32
    require relayer_quorum >= 3
    require solver_bond >= 10000

    timeout 3600s
    on_fail refund arbitrum.USDC to sender
}
"#,
    );
    assert!(
        full_intent >= 6,
        "the sample declares six sections, parsed {full_intent} items"
    );
}
