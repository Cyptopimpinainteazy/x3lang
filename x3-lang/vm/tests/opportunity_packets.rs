//! PHASE 29 opportunity-packet tests: the path a packet takes from the solver
//! that signs it to an operator that admits it, over the bytes it travels as.

use std::collections::{BTreeMap, BTreeSet};

use ed25519_dalek::{Signer, SigningKey};
use x3_lang_compiler::opportunity::Opportunity;
use x3_lang_vm::opportunity_packet::{
    requirements, sign_packet, validate_packet, verify_packet, verify_packet_with_evidence, OpportunityPacket,
    OpportunityPacketError, OpportunityPacketLedger, PacketEvidence, StrategyAttestation, VenueAttestation,
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
            proof_requirements: BTreeSet::from([requirements::STATE_ROOT_FRESHNESS.to_string()]),
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

// ===== TICKET-074: the packet's proof requirements, checked against what a host states =====

/// The packet the tests below verify, requiring everything this verifier can check.
fn packet_requiring_all_three() -> OpportunityPacket {
    let mut packet = packet();
    packet.proof_requirements = BTreeSet::from([
        requirements::STATE_ROOT_FRESHNESS.to_string(),
        requirements::VENUE_PRICE_ATTESTATION.to_string(),
        requirements::STRATEGY_COMMITMENT.to_string(),
    ]);
    sign_packet(packet, SOLVER, &solver_key()).expect("the packet signs after its requirements change")
}

/// Evidence that satisfies every requirement of `packet_requiring_all_three`.
fn complete_evidence(packet: &OpportunityPacket) -> PacketEvidence {
    PacketEvidence {
        state_root_blocks: BTreeMap::from([("ethereum".to_string(), 995)]),
        observed_block: 1_000,
        max_state_root_age_blocks: 10,
        venues: vec![
            VenueAttestation {
                venue: "uniswap-v3".to_string(),
                liquidity: 1_000_000,
                fee_bps: 20,
            },
            VenueAttestation {
                venue: "raydium".to_string(),
                liquidity: 2_000_000,
                fee_bps: 10,
            },
        ],
        strategies: vec![StrategyAttestation {
            commitment: packet.execution_commitment,
            key_id: SOLVER.to_string(),
            public_key: solver_key().verifying_key().to_bytes(),
            signature: solver_key().sign(&packet.execution_commitment).to_bytes().to_vec(),
        }],
    }
}

#[test]
fn every_requirement_a_packet_declares_is_checked_and_reported() {
    // The difference this module exists for: "verified" for a packet that declared three requirements
    // and had each of them checked is not the same as "verified" for one that declared none, so the
    // caller is told which held.
    let packet = packet_requiring_all_three();
    let evidence = complete_evidence(&packet);
    let verification = verify_packet_with_evidence(&packet, &trusted_solvers(), 100, &evidence)
        .expect("a packet whose evidence covers its requirements verifies");
    // The order is the packet's own `BTreeSet` order, so the property is *which* requirements were
    // checked and not the sequence they were visited in.
    let mut checked = verification.checked.clone();
    checked.sort_unstable();
    let mut expected = vec![
        requirements::STATE_ROOT_FRESHNESS,
        requirements::VENUE_PRICE_ATTESTATION,
        requirements::STRATEGY_COMMITMENT,
    ];
    expected.sort_unstable();
    assert_eq!(checked, expected, "all three, in the vocabulary's own spelling");
}

#[test]
fn a_stale_state_root_is_refused_with_both_block_numbers() {
    let packet = packet_requiring_all_three();
    let mut evidence = complete_evidence(&packet);
    // The root was read at 900 and the verifier is at 1,000, with a window of 10.
    evidence.state_root_blocks.insert("ethereum".to_string(), 900);
    match verify_packet_with_evidence(&packet, &trusted_solvers(), 100, &evidence) {
        Err(OpportunityPacketError::StateRootStale {
            ref domain,
            block,
            observed_block,
            max_age_blocks,
        }) => {
            assert_eq!(domain, "ethereum");
            assert_eq!((block, observed_block, max_age_blocks), (900, 1_000, 10));
            let rendered = OpportunityPacketError::StateRootStale {
                domain: domain.clone(),
                block,
                observed_block,
                max_age_blocks,
            }
            .to_string();
            assert!(
                rendered.contains("900") && rendered.contains("1000"),
                "the refusal must state both figures: {rendered}"
            );
        }
        other => panic!("expected a stale-root refusal, got {other:?}"),
    }
    // A root from a block the verifier has not reached is the other half, and its own refusal.
    evidence.state_root_blocks.insert("ethereum".to_string(), 1_001);
    match verify_packet_with_evidence(&packet, &trusted_solvers(), 100, &evidence) {
        Err(OpportunityPacketError::StateRootFromTheFuture {
            block, observed_block, ..
        }) => assert_eq!((block, observed_block), (1_001, 1_000)),
        other => panic!("expected a future-root refusal, got {other:?}"),
    }
    // And nothing stating the block at all is a refusal rather than a pass.
    evidence.state_root_blocks.clear();
    match verify_packet_with_evidence(&packet, &trusted_solvers(), 100, &evidence) {
        Err(OpportunityPacketError::StateRootBlockUnstated { domain }) => assert_eq!(domain, "ethereum"),
        other => panic!("expected an unstated-block refusal, got {other:?}"),
    }
}

#[test]
fn a_venue_that_will_not_fill_at_the_routes_terms_is_refused_with_both_figures() {
    let packet = packet_requiring_all_three();
    // The route claims a minimum liquidity of 1,000,000 and 30bps of fees.
    let mut evidence = complete_evidence(&packet);
    evidence.venues[1].liquidity = 999_999;
    match verify_packet_with_evidence(&packet, &trusted_solvers(), 100, &evidence) {
        Err(OpportunityPacketError::VenueLiquidityBelowRoute {
            venue,
            attested,
            route_min_liquidity,
        }) => {
            assert_eq!(venue, "raydium");
            assert_eq!((attested, route_min_liquidity), (999_999, 1_000_000));
        }
        other => panic!("expected a liquidity refusal, got {other:?}"),
    }
    // The fees the venues attest sum to the fee the route was scored from, or the route was scored
    // from prices that are not the prices that will fill.
    let mut evidence = complete_evidence(&packet);
    evidence.venues[1].fee_bps = 5;
    match verify_packet_with_evidence(&packet, &trusted_solvers(), 100, &evidence) {
        Err(OpportunityPacketError::VenueFeeDisagrees {
            attested_bps,
            route_bps,
        }) => assert_eq!((attested_bps, route_bps), (25, 30)),
        other => panic!("expected a fee refusal, got {other:?}"),
    }
    // And a venue the route names that attested nothing cannot be checked at all.
    let mut evidence = complete_evidence(&packet);
    evidence.venues.pop();
    match verify_packet_with_evidence(&packet, &trusted_solvers(), 100, &evidence) {
        Err(OpportunityPacketError::VenueUnattested { venue }) => assert_eq!(venue, "raydium"),
        other => panic!("expected an unattested-venue refusal, got {other:?}"),
    }
}

#[test]
fn a_strategy_commitment_is_checked_against_a_trusted_key_not_the_packets_own() {
    let packet = packet_requiring_all_three();
    // Nothing signed it.
    let mut evidence = complete_evidence(&packet);
    evidence.strategies.clear();
    match verify_packet_with_evidence(&packet, &trusted_solvers(), 100, &evidence) {
        Err(OpportunityPacketError::StrategyCommitmentUnattested { commitment }) => {
            assert_eq!(commitment, packet.execution_commitment)
        }
        other => panic!("expected an unattested-commitment refusal, got {other:?}"),
    }
    // A key that arrived beside the signature it checks is a restatement of the packet's own claim:
    // the stranger signs, and the verifier looks the key up in the trusted set rather than here.
    let mut evidence = complete_evidence(&packet);
    evidence.strategies = vec![StrategyAttestation {
        commitment: packet.execution_commitment,
        key_id: SOLVER.to_string(),
        public_key: stranger_key().verifying_key().to_bytes(),
        signature: stranger_key().sign(&packet.execution_commitment).to_bytes().to_vec(),
    }];
    match verify_packet_with_evidence(&packet, &trusted_solvers(), 100, &evidence) {
        Err(OpportunityPacketError::StrategyCommitmentUnproven { key_id }) => assert_eq!(key_id, SOLVER),
        other => panic!("expected an unproven-commitment refusal, got {other:?}"),
    }
}

#[test]
fn a_requirement_this_verifier_does_not_know_is_refused_by_name() {
    // The fixture this file carried before the vocabulary existed called it "state" — a name no
    // verifier had a check for, which is what made a packet look evidenced while nothing had read it.
    let mut packet = packet();
    packet.proof_requirements = BTreeSet::from(["state".to_string()]);
    let packet = sign_packet(packet, SOLVER, &solver_key()).expect("it still signs");
    match validate_packet(&packet) {
        Err(OpportunityPacketError::UnknownProofRequirement { requirement }) => assert_eq!(requirement, "state"),
        other => panic!("expected an unknown-requirement refusal, got {other:?}"),
    }
    // And the refusal says which requirements *are* checkable, so its reader can act on it.
    let rendered = OpportunityPacketError::UnknownProofRequirement {
        requirement: "state".to_string(),
    }
    .to_string();
    for known in requirements::ALL {
        assert!(rendered.contains(known), "the refusal must name {known}: {rendered}");
    }
}
