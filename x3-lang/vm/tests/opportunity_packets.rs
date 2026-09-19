//! PHASE 29 opportunity-packet tests: the path a packet takes from the solver
//! that signs it to an operator that admits it, over the bytes it travels as.

use std::collections::{BTreeMap, BTreeSet};

use ed25519_dalek::SigningKey;
use x3_lang_compiler::opportunity::Opportunity;
use x3_lang_vm::opportunity_packet::{
    sign_packet, validate_packet, verify_packet, OpportunityPacket, OpportunityPacketError, OpportunityPacketLedger,
    OPPORTUNITY_PACKET_VERSION,
};

const SOLVER: &str = "solver-1";

fn solver_key() -> SigningKey {
    SigningKey::from_bytes(&[7u8; 32])
}

fn stranger_key() -> SigningKey {
    SigningKey::from_bytes(&[11u8; 32])
}

fn trusted_solvers() -> BTreeMap<String, [u8; 32]> {
    BTreeMap::from([(SOLVER.to_string(), solver_key().verifying_key().to_bytes())])
}

fn route() -> Opportunity {
    Opportunity {
        venues: vec!["uniswap-v3".to_string(), "raydium".to_string()],
        assets: vec!["USDC".to_string(), "WETH".to_string(), "USDC".to_string()],
        fee_bps: 30,
        slippage_bps: 20,
        max_risk: 1,
        latency_ms: 400,
        finality_blocks: 12,
        min_liquidity: 1_000_000,
    }
}

/// The packet a solver would publish: 100k of capital needed, at most 500k
/// authorised, 520k expected back, a 2k fee ceiling and a 5k profit floor.
fn packet() -> OpportunityPacket {
    sign_packet(
        OpportunityPacket {
            version: OPPORTUNITY_PACKET_VERSION,
            strategy_id: "tri-arb".to_string(),
            artifact_hash: [3u8; 32],
            state_roots: BTreeMap::from([("ethereum".to_string(), [9u8; 32])]),
            route: route(),
            required_capital: 100_000,
            max_capital: 500_000,
            expected_output: 520_000,
            minimum_profit: 5_000,
            maximum_fee: 2_000,
            maximum_slippage_bps: 50,
            deadline_blocks: 500,
            proof_requirements: BTreeSet::from(["state".to_string()]),
            execution_commitment: [0u8; 32],
            packet_hash: [0u8; 32],
            signature: None,
        },
        SOLVER,
        &solver_key(),
    )
    .expect("the solver's packet signs")
}

#[test]
fn a_packet_verifies_after_the_bytes_it_travels_as() {
    let packet = packet();
    let bytes = packet.encode().expect("the packet encodes");
    let received = OpportunityPacket::decode(&bytes).expect("the packet decodes");
    assert_eq!(received, packet);
    assert_eq!(received.packet_hash, packet.packet_hash);
    assert_eq!(verify_packet(&received, &trusted_solvers(), 100), Ok(()));
}

#[test]
fn an_operator_admits_each_packet_once() {
    let mut ledger = OpportunityPacketLedger::new();
    let first = packet();
    assert_eq!(ledger.admit(&first, &trusted_solvers(), 100), Ok(()));
    assert_eq!(
        ledger.admit(&first, &trusted_solvers(), 101),
        Err(OpportunityPacketError::PacketAlreadySeen(first.packet_hash))
    );

    let mut second = first.clone();
    second.expected_output += 500;
    let second = sign_packet(second, SOLVER, &solver_key()).expect("the second packet signs");
    assert_eq!(ledger.admit(&second, &trusted_solvers(), 101), Ok(()));
    assert!(ledger.has_admitted(&second.packet_hash));
}

#[test]
fn an_operator_refuses_a_packet_whose_terms_were_edited_in_flight() {
    let mut received = packet();
    received.required_capital = 1;
    match verify_packet(&received, &trusted_solvers(), 100) {
        // `required_capital` is an execution term, so the execution commitment
        // is what catches it. An edit to a field outside the terms, such as
        // `strategy_id`, is caught by the packet hash instead.
        Err(OpportunityPacketError::ExecutionCommitmentMismatch { .. }) => {}
        other => panic!("expected the execution commitment to be refused, got {other:?}"),
    }
}

#[test]
fn an_operator_refuses_a_packet_from_a_solver_it_does_not_trust() {
    let packet = sign_packet(packet(), SOLVER, &stranger_key()).expect("the packet signs with the stranger's key");
    assert_eq!(validate_packet(&packet), Ok(()));
    assert_eq!(
        verify_packet(&packet, &trusted_solvers(), 100),
        Err(OpportunityPacketError::UntrustedSigner(SOLVER.to_string()))
    );
}
