//! PHASE 29 — opportunity packets: a portable, signed, replay-protected
//! description of an opportunity a solver wants executed.
//!
//! The phase asks for a package carrying a strategy commitment, the state it
//! was observed against, the route it claims, its capital window, its expected
//! output, its profit floor, its fee and slippage ceilings, a deadline, its
//! proof requirements and an execution commitment — signed, versioned,
//! hashable, deterministic and replay-protected.
//!
//! This module decides the properties that are decidable *from the packet*:
//!
//! - the packet's own economics are coherent (a capital window that is not
//!   inverted, a profit floor its own numbers can reach *after* its own
//!   worst-case fee, ceilings that actually bound something);
//! - the route it claims can carry what it claims (the venue with the least
//!   declared liquidity must absorb `max_capital`, and the route's declared fee
//!   and slippage must fit inside the packet's ceilings);
//! - `execution_commitment` really covers the packet's execution terms, and
//!   `packet_hash` really covers the packet, so neither is decoration;
//! - the signature is present, from a signer the admission policy trusts, and
//!   valid over the packet hash;
//! - the packet has not expired at the block it is admitted, and has not been
//!   admitted before.
//!
//! What it deliberately does not decide is whether the opportunity is *real* —
//! that the state roots are current, that the venues will trade at the claimed
//! prices, or that the strategy behind `strategy_id` is the one that produced
//! the route. Those need host evidence this crate does not have, and a packet
//! that pretended to verify them would be a no-op check this repository
//! forbids. They are `TICKET-071`.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt;

use ed25519_dalek::{Signature, Signer, SigningKey, Verifier, VerifyingKey};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use x3_lang_compiler::opportunity::Opportunity;

/// Version of the opportunity packet schema this module reads and writes.
///
/// Carried in the packet rather than assumed, so a packet written by a later
/// compiler is refused with its version named instead of being decoded as this
/// one and silently meaning something else.
pub const OPPORTUNITY_PACKET_VERSION: u16 = 1;

/// Largest slippage ceiling a packet may declare, in basis points.
///
/// A ceiling is a bound on a cost, and costs are a fraction of the trade, so
/// 100% is the largest number that still bounds anything. A packet declaring
/// more is not declaring a loose ceiling, it is not declaring one at all, in
/// the same way `max_slippage_bps = 65535` would be.
pub const MAX_SLIPPAGE_BPS: u16 = 10_000;

/// Basis points are hundredths of a percent, so a bps figure is a fraction of
/// this denominator.
const BASIS_POINTS_DENOMINATOR: u128 = 10_000;

/// Domain separator for the packet hash, so a packet hash can never equal a
/// hash computed over the same bytes for another purpose.
const PACKET_DOMAIN: &[u8] = b"X3:OPPORTUNITY_PACKET:V1";

/// Domain separator for the execution commitment.
///
/// The version lives in the domain, which is why `version` itself is not part
/// of the committed terms: terms committed under a future schema produce a
/// different commitment by construction, so the two can never be confused for
/// one another.
const EXECUTION_DOMAIN: &[u8] = b"X3:OPPORTUNITY_PACKET_EXECUTION:V1";

/// The solver's signature over a packet hash.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct OpportunitySignature {
    /// Stable identifier the admission policy looks the key up by.
    pub key_id: String,
    /// The key that actually signed, carried so a verifier can check the
    /// signature without a second lookup — and so a mismatch between the key
    /// the policy trusts for `key_id` and the key presented here is visible.
    pub public_key: [u8; 32],
    /// The raw 64-byte ed25519 signature, carried as bytes for the same reason
    /// `ReceiptAttestation` carries it that way: serde implements neither
    /// `Serialize` nor `Deserialize` for arrays longer than 32, and a signature
    /// that cannot survive the wire is not portable. Its length is checked
    /// before it is used.
    pub signature: Vec<u8>,
}

/// A solver's signed claim that an opportunity exists and how it wants it run.
///
/// Field-by-field this follows the phase's example structure, with two units
/// made explicit rather than left to the reader: `deadline_blocks` is a block
/// height because "deadline" with no unit is exactly the ambiguity `TICKET-033`
/// refuses for timeouts, and `maximum_fee` is an absolute amount in the route's
/// input asset because a fee ceiling expressed in basis points could not bound
/// a route whose declared fees are summed in basis points but whose cost grows
/// with size.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct OpportunityPacket {
    pub version: u16,
    /// Commitment to the strategy that produced the route. The packet reveals
    /// the route and hides the strategy, which is the whole point of the
    /// execution-auction reading of the phase.
    pub strategy_id: String,
    /// Hash of the compiled artifact the execution must follow.
    pub artifact_hash: [u8; 32],
    /// Root the opportunity was observed against, per domain (chain or VM
    /// family). A packet that names no root was observed against nothing.
    pub state_roots: BTreeMap<String, [u8; 32]>,
    /// The route this opportunity claims, as decided by the opportunity graph.
    pub route: Opportunity,
    /// Capital the route needs before it can run.
    pub required_capital: u128,
    /// Largest capital the solver authorises for this packet.
    pub max_capital: u128,
    /// Output the solver expects, in the route's output asset.
    pub expected_output: u128,
    /// Profit floor the solver insists on, in the route's output asset.
    pub minimum_profit: u128,
    /// Largest fee the solver will pay, in the route's input asset.
    pub maximum_fee: u128,
    /// Largest slippage the solver will accept, in basis points.
    pub maximum_slippage_bps: u16,
    /// Block height at and after which the packet must not be admitted.
    pub deadline_blocks: u64,
    /// Evidence the solver promises to attach. The vocabulary belongs to the
    /// host and the venue adapters, so this module checks only that a
    /// requirement is named rather than inventing a closed set of proof kinds
    /// that nothing reads yet.
    pub proof_requirements: BTreeSet<String>,
    /// Commitment over the packet's execution terms.
    pub execution_commitment: [u8; 32],
    /// Hash over the packet with this field and the signature cleared.
    pub packet_hash: [u8; 32],
    pub signature: Option<OpportunitySignature>,
}

/// Failures raised while validating, admitting, or committing a packet.
///
/// Every variant names the figures it refused, because "invalid packet" tells a
/// solver nothing about which of its claims was rejected.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum OpportunityPacketError {
    UnsupportedVersion {
        version: u16,
    },
    CanonicalEncoding,
    EmptyStrategyId,
    ZeroArtifactHash,
    NoStateRoots,
    UnnamedStateRootDomain,
    ZeroStateRoot {
        domain: String,
    },
    EmptyRoute,
    RouteAssetCountMismatch {
        venues: usize,
        assets: usize,
    },
    UnnamedVenue {
        index: usize,
    },
    ZeroRequiredCapital,
    CapitalWindowInverted {
        required_capital: u128,
        max_capital: u128,
    },
    ZeroMinimumProfit,
    ProfitFloorNotReachable {
        expected_output: u128,
        required_capital: u128,
        maximum_fee: u128,
        minimum_profit: u128,
    },
    RouteLiquidityBelowCapital {
        route_liquidity: u128,
        max_capital: u128,
    },
    RouteFeeAboveCeiling {
        route_fee: u128,
        maximum_fee: u128,
    },
    RouteFeeOverflow {
        fee_bps: u32,
    },
    RouteSlippageAboveCeiling {
        route_slippage_bps: u32,
        maximum_slippage_bps: u16,
    },
    SlippageCeilingAboveMaximum {
        declared_bps: u16,
        maximum_bps: u16,
    },
    UnnamedProofRequirement,
    ExecutionCommitmentMismatch {
        declared: [u8; 32],
        actual: [u8; 32],
    },
    HashMismatch {
        expected: [u8; 32],
        actual: [u8; 32],
    },
    Expired {
        deadline_blocks: u64,
        at_block: u64,
    },
    MissingSignature,
    UntrustedSigner(String),
    InvalidSignature,
    /// This exact packet (by `packet_hash`) has already been admitted once.
    PacketAlreadySeen([u8; 32]),
}

impl fmt::Display for OpportunityPacketError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::UnsupportedVersion { version } => {
                write!(f, "unsupported opportunity packet version {version}, expected {OPPORTUNITY_PACKET_VERSION}")
            }
            Self::CanonicalEncoding => write!(f, "canonical opportunity packet encoding failed"),
            Self::EmptyStrategyId => write!(f, "opportunity packet names no strategy"),
            Self::ZeroArtifactHash => write!(f, "opportunity packet names no artifact hash"),
            Self::NoStateRoots => write!(f, "opportunity packet carries no state root"),
            Self::UnnamedStateRootDomain => write!(f, "opportunity packet has a state root with no domain name"),
            Self::ZeroStateRoot { domain } => {
                write!(f, "opportunity packet carries a zero state root for domain '{domain}'")
            }
            Self::EmptyRoute => write!(f, "opportunity packet claims a route with no venues"),
            Self::RouteAssetCountMismatch { venues, assets } => {
                write!(f, "opportunity packet route has {venues} venues but {assets} assets, expected {}", venues + 1)
            }
            Self::UnnamedVenue { index } => {
                write!(f, "opportunity packet route venue at index {index} is unnamed")
            }
            Self::ZeroRequiredCapital => write!(f, "opportunity packet requires zero capital"),
            Self::CapitalWindowInverted {
                required_capital,
                max_capital,
            } => write!(
                f,
                "opportunity packet capital window is inverted: requires {required_capital} but authorises at most {max_capital}"
            ),
            Self::ZeroMinimumProfit => write!(f, "opportunity packet declares no minimum profit"),
            Self::ProfitFloorNotReachable {
                expected_output,
                required_capital,
                maximum_fee,
                minimum_profit,
            } => write!(
                f,
                "opportunity packet cannot reach its own floor: expected output {expected_output} less capital \
                 {required_capital} and worst-case fee {maximum_fee} does not cover minimum profit {minimum_profit}"
            ),
            Self::RouteLiquidityBelowCapital {
                route_liquidity,
                max_capital,
            } => write!(
                f,
                "opportunity packet authorises {max_capital} but the thinnest venue on its route declares liquidity {route_liquidity}"
            ),
            Self::RouteFeeAboveCeiling { route_fee, maximum_fee } => write!(
                f,
                "opportunity packet route declares fees of {route_fee} at its maximum capital, above the ceiling {maximum_fee}"
            ),
            Self::RouteFeeOverflow { fee_bps } => {
                write!(f, "opportunity packet route fee of {fee_bps} bps overflows at the packet's maximum capital")
            }
            Self::RouteSlippageAboveCeiling {
                route_slippage_bps,
                maximum_slippage_bps,
            } => write!(
                f,
                "opportunity packet route declares up to {route_slippage_bps} bps of slippage, above the ceiling {maximum_slippage_bps}"
            ),
            Self::SlippageCeilingAboveMaximum {
                declared_bps,
                maximum_bps,
            } => write!(f, "opportunity packet slippage ceiling {declared_bps} bps exceeds the largest ceiling that bounds anything, {maximum_bps}"),
            Self::UnnamedProofRequirement => write!(f, "opportunity packet has a proof requirement with no name"),
            Self::ExecutionCommitmentMismatch { declared, actual } => write!(
                f,
                "opportunity packet execution commitment {declared:?} does not cover its terms, which commit to {actual:?}"
            ),
            Self::HashMismatch { expected, actual } => {
                write!(f, "opportunity packet hash mismatch: expected {expected:?}, actual {actual:?}")
            }
            Self::Expired {
                deadline_blocks,
                at_block,
            } => write!(f, "opportunity packet expired at block {deadline_blocks} and was admitted at block {at_block}"),
            Self::MissingSignature => write!(f, "opportunity packet is unsigned"),
            Self::UntrustedSigner(key_id) => write!(f, "opportunity packet signer '{key_id}' is not trusted"),
            Self::InvalidSignature => write!(f, "opportunity packet signature is invalid"),
            Self::PacketAlreadySeen(hash) => write!(f, "opportunity packet {hash:?} has already been admitted"),
        }
    }
}

impl std::error::Error for OpportunityPacketError {}

/// The packet terms an `execution_commitment` covers.
///
/// `strategy_id` is absent on purpose: the packet sells its execution and hides
/// the strategy that produced it, so a commitment over the execution is one a
/// builder can act on without learning what generated it. `version` is absent
/// because `EXECUTION_DOMAIN` already pins the schema.
#[derive(Serialize)]
struct ExecutionTerms<'a> {
    artifact_hash: &'a [u8; 32],
    state_roots: &'a BTreeMap<String, [u8; 32]>,
    route: &'a Opportunity,
    required_capital: u128,
    max_capital: u128,
    expected_output: u128,
    minimum_profit: u128,
    maximum_fee: u128,
    maximum_slippage_bps: u16,
    deadline_blocks: u64,
    proof_requirements: &'a BTreeSet<String>,
}

impl OpportunityPacket {
    fn execution_terms(&self) -> ExecutionTerms<'_> {
        ExecutionTerms {
            artifact_hash: &self.artifact_hash,
            state_roots: &self.state_roots,
            route: &self.route,
            required_capital: self.required_capital,
            max_capital: self.max_capital,
            expected_output: self.expected_output,
            minimum_profit: self.minimum_profit,
            maximum_fee: self.maximum_fee,
            maximum_slippage_bps: self.maximum_slippage_bps,
            deadline_blocks: self.deadline_blocks,
            proof_requirements: &self.proof_requirements,
        }
    }

    /// Canonical bytes of the execution terms, domain-separated.
    pub fn execution_commitment(&self) -> Result<[u8; 32], OpportunityPacketError> {
        let bytes =
            bincode::serialize(&self.execution_terms()).map_err(|_| OpportunityPacketError::CanonicalEncoding)?;
        let mut hasher = Sha256::new();
        hasher.update(EXECUTION_DOMAIN);
        hasher.update(bytes);
        Ok(hasher.finalize().into())
    }

    /// The packet as it is hashed: `packet_hash` cleared and `signature`
    /// removed, so a hash never depends on itself and a signature never covers
    /// itself.
    fn canonical_form(&self) -> Self {
        let mut canonical = self.clone();
        canonical.packet_hash = [0u8; 32];
        canonical.signature = None;
        canonical
    }

    /// Deterministic bytes a packet hash is computed over.
    pub fn canonical_bytes(&self) -> Result<Vec<u8>, OpportunityPacketError> {
        bincode::serialize(&self.canonical_form()).map_err(|_| OpportunityPacketError::CanonicalEncoding)
    }

    /// Hash over the packet with `packet_hash` and `signature` cleared.
    pub fn compute_hash(&self) -> Result<[u8; 32], OpportunityPacketError> {
        let mut hasher = Sha256::new();
        hasher.update(PACKET_DOMAIN);
        hasher.update(self.canonical_bytes()?);
        Ok(hasher.finalize().into())
    }

    /// Portable bytes for the whole packet, including its hash and signature.
    pub fn encode(&self) -> Result<Vec<u8>, OpportunityPacketError> {
        bincode::serialize(self).map_err(|_| OpportunityPacketError::CanonicalEncoding)
    }

    pub fn decode(bytes: &[u8]) -> Result<Self, OpportunityPacketError> {
        bincode::deserialize(bytes).map_err(|_| OpportunityPacketError::CanonicalEncoding)
    }

    /// Set the execution commitment from the packet's terms and the packet hash
    /// from the packet. Any signature the packet had is invalidated by both and
    /// is therefore cleared.
    pub fn finalize(mut self) -> Result<Self, OpportunityPacketError> {
        self.signature = None;
        self.execution_commitment = self.execution_commitment()?;
        self.packet_hash = self.compute_hash()?;
        Ok(self)
    }

    /// Whether the packet requires the named proof.
    pub fn requires_proof(&self, requirement: &str) -> bool {
        self.proof_requirements.contains(requirement)
    }
}

/// Sign a packet's hash with `signing_key`, after re-deriving its commitments.
pub fn sign_packet(
    packet: OpportunityPacket,
    key_id: &str,
    signing_key: &SigningKey,
) -> Result<OpportunityPacket, OpportunityPacketError> {
    let mut packet = packet.finalize()?;
    let signature = signing_key.sign(&packet.packet_hash);
    packet.signature = Some(OpportunitySignature {
        key_id: key_id.to_string(),
        public_key: signing_key.verifying_key().to_bytes(),
        signature: signature.to_bytes().to_vec(),
    });
    Ok(packet)
}

/// Check everything about a packet that does not depend on when or by whom it
/// is admitted: its schema version, its structure, its economics, and both of
/// its commitments.
pub fn validate_packet(packet: &OpportunityPacket) -> Result<(), OpportunityPacketError> {
    if packet.version != OPPORTUNITY_PACKET_VERSION {
        return Err(OpportunityPacketError::UnsupportedVersion {
            version: packet.version,
        });
    }
    if packet.strategy_id.trim().is_empty() {
        return Err(OpportunityPacketError::EmptyStrategyId);
    }
    if packet.artifact_hash == [0u8; 32] {
        return Err(OpportunityPacketError::ZeroArtifactHash);
    }
    validate_state_roots(&packet.state_roots)?;
    validate_route(&packet.route)?;
    validate_economics(packet)?;
    validate_commitments(packet)
}

fn validate_state_roots(roots: &BTreeMap<String, [u8; 32]>) -> Result<(), OpportunityPacketError> {
    if roots.is_empty() {
        return Err(OpportunityPacketError::NoStateRoots);
    }
    for (domain, root) in roots {
        if domain.trim().is_empty() {
            return Err(OpportunityPacketError::UnnamedStateRootDomain);
        }
        if root == &[0u8; 32] {
            return Err(OpportunityPacketError::ZeroStateRoot { domain: domain.clone() });
        }
    }
    Ok(())
}

fn validate_route(route: &Opportunity) -> Result<(), OpportunityPacketError> {
    if route.venues.is_empty() {
        return Err(OpportunityPacketError::EmptyRoute);
    }
    if route.assets.len() != route.venues.len() + 1 {
        return Err(OpportunityPacketError::RouteAssetCountMismatch {
            venues: route.venues.len(),
            assets: route.assets.len(),
        });
    }
    for (index, venue) in route.venues.iter().enumerate() {
        if venue.trim().is_empty() {
            return Err(OpportunityPacketError::UnnamedVenue { index });
        }
    }
    Ok(())
}

fn validate_economics(packet: &OpportunityPacket) -> Result<(), OpportunityPacketError> {
    if packet.required_capital == 0 {
        return Err(OpportunityPacketError::ZeroRequiredCapital);
    }
    if packet.max_capital < packet.required_capital {
        return Err(OpportunityPacketError::CapitalWindowInverted {
            required_capital: packet.required_capital,
            max_capital: packet.max_capital,
        });
    }
    if packet.minimum_profit == 0 {
        return Err(OpportunityPacketError::ZeroMinimumProfit);
    }

    // The floor is checked against the worst case the packet itself declares:
    // the output it expects, less the capital it needs, less the *largest* fee
    // it authorises. A packet that only clears its floor when fees are smaller
    // than the ceiling it wrote down is a packet whose floor is not a floor.
    let net = packet
        .expected_output
        .checked_sub(packet.required_capital)
        .and_then(|gross| gross.checked_sub(packet.maximum_fee));
    match net {
        Some(net) if net >= packet.minimum_profit => {}
        _ => {
            return Err(OpportunityPacketError::ProfitFloorNotReachable {
                expected_output: packet.expected_output,
                required_capital: packet.required_capital,
                maximum_fee: packet.maximum_fee,
                minimum_profit: packet.minimum_profit,
            })
        }
    }

    if packet.maximum_slippage_bps > MAX_SLIPPAGE_BPS {
        return Err(OpportunityPacketError::SlippageCeilingAboveMaximum {
            declared_bps: packet.maximum_slippage_bps,
            maximum_bps: MAX_SLIPPAGE_BPS,
        });
    }
    if packet.route.slippage_bps > u32::from(packet.maximum_slippage_bps) {
        return Err(OpportunityPacketError::RouteSlippageAboveCeiling {
            route_slippage_bps: packet.route.slippage_bps,
            maximum_slippage_bps: packet.maximum_slippage_bps,
        });
    }

    // The thinnest venue decides whether the route can carry the size: a venue
    // that cannot absorb the capital is not a venue for it, which is the
    // opportunity graph's own reading of `min_liquidity`.
    if packet.max_capital > packet.route.min_liquidity {
        return Err(OpportunityPacketError::RouteLiquidityBelowCapital {
            route_liquidity: packet.route.min_liquidity,
            max_capital: packet.max_capital,
        });
    }

    // The route sums its venues' fees in basis points, so at the largest size
    // the packet authorises that sum is this many input-asset units.
    let route_fee = packet.max_capital.checked_mul(u128::from(packet.route.fee_bps)).ok_or(
        OpportunityPacketError::RouteFeeOverflow {
            fee_bps: packet.route.fee_bps,
        },
    )? / BASIS_POINTS_DENOMINATOR;
    if route_fee > packet.maximum_fee {
        return Err(OpportunityPacketError::RouteFeeAboveCeiling {
            route_fee,
            maximum_fee: packet.maximum_fee,
        });
    }

    if packet
        .proof_requirements
        .iter()
        .any(|requirement| requirement.trim().is_empty())
    {
        return Err(OpportunityPacketError::UnnamedProofRequirement);
    }

    Ok(())
}

fn validate_commitments(packet: &OpportunityPacket) -> Result<(), OpportunityPacketError> {
    // The execution commitment is checked first because it is the narrower
    // claim: a packet whose *terms* were edited fails here naming the terms,
    // and one whose non-term fields (`strategy_id`) were edited falls through
    // to the packet hash. Checking the packet hash first would report both as
    // the same defect and say nothing about which fields moved.
    let commitment = packet.execution_commitment()?;
    if commitment != packet.execution_commitment {
        return Err(OpportunityPacketError::ExecutionCommitmentMismatch {
            declared: packet.execution_commitment,
            actual: commitment,
        });
    }
    let hash = packet.compute_hash()?;
    if hash != packet.packet_hash {
        return Err(OpportunityPacketError::HashMismatch {
            expected: packet.packet_hash,
            actual: hash,
        });
    }
    Ok(())
}

/// Verify a packet's signature against the keys the admission policy trusts.
pub fn verify_packet_signature(
    packet: &OpportunityPacket,
    trusted_keys: &BTreeMap<String, [u8; 32]>,
) -> Result<(), OpportunityPacketError> {
    let signature = packet
        .signature
        .as_ref()
        .ok_or(OpportunityPacketError::MissingSignature)?;
    let trusted = trusted_keys
        .get(&signature.key_id)
        .ok_or_else(|| OpportunityPacketError::UntrustedSigner(signature.key_id.clone()))?;
    if trusted != &signature.public_key {
        return Err(OpportunityPacketError::UntrustedSigner(signature.key_id.clone()));
    }
    let verifying_key = VerifyingKey::from_bytes(trusted).map_err(|_| OpportunityPacketError::InvalidSignature)?;
    let signature_bytes: [u8; 64] = signature
        .signature
        .as_slice()
        .try_into()
        .map_err(|_| OpportunityPacketError::InvalidSignature)?;
    let signature = Signature::from_bytes(&signature_bytes);
    verifying_key
        .verify(&packet.packet_hash, &signature)
        .map_err(|_| OpportunityPacketError::InvalidSignature)
}

/// Full admission check: structure and commitments, then expiry, then
/// signature. Replay is the ledger's job, because it needs state this function
/// must not carry.
pub fn verify_packet(
    packet: &OpportunityPacket,
    trusted_keys: &BTreeMap<String, [u8; 32]>,
    at_block: u64,
) -> Result<(), OpportunityPacketError> {
    validate_packet(packet)?;
    if at_block >= packet.deadline_blocks {
        return Err(OpportunityPacketError::Expired {
            deadline_blocks: packet.deadline_blocks,
            at_block,
        });
    }
    verify_packet_signature(packet, trusted_keys)
}

/// In-process record of every packet hash already admitted.
///
/// `verify_packet` has no memory: the identical packet verifies successfully
/// every time it is presented, which is exactly what a solver submitting the
/// same opportunity twice wants. This ledger closes that, mirroring the
/// trading receipt ledger — and, like it, it is deliberately not persisted or
/// distributed by this crate: a real admission service is expected to back the
/// check with whatever durable storage its deployment has.
#[derive(Debug, Clone, Default)]
pub struct OpportunityPacketLedger {
    seen: BTreeSet<[u8; 32]>,
}

impl OpportunityPacketLedger {
    pub fn new() -> Self {
        Self::default()
    }

    /// Verify `packet` exactly as `verify_packet` does, and additionally refuse
    /// it if this ledger has already admitted the same packet hash. On success
    /// the hash is recorded.
    pub fn admit(
        &mut self,
        packet: &OpportunityPacket,
        trusted_keys: &BTreeMap<String, [u8; 32]>,
        at_block: u64,
    ) -> Result<(), OpportunityPacketError> {
        verify_packet(packet, trusted_keys, at_block)?;
        if !self.seen.insert(packet.packet_hash) {
            return Err(OpportunityPacketError::PacketAlreadySeen(packet.packet_hash));
        }
        Ok(())
    }

    /// Whether this ledger has already admitted `packet_hash`.
    pub fn has_admitted(&self, packet_hash: &[u8; 32]) -> bool {
        self.seen.contains(packet_hash)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const KEY_ID: &str = "solver-1";

    fn signing_key() -> SigningKey {
        SigningKey::from_bytes(&[7u8; 32])
    }

    fn other_key() -> SigningKey {
        SigningKey::from_bytes(&[9u8; 32])
    }

    fn trusted() -> BTreeMap<String, [u8; 32]> {
        BTreeMap::from([(KEY_ID.to_string(), signing_key().verifying_key().to_bytes())])
    }

    fn route() -> Opportunity {
        Opportunity {
            venues: vec!["dex-a".to_string(), "dex-b".to_string()],
            assets: vec!["USDC".to_string(), "WETH".to_string(), "USDC".to_string()],
            fee_bps: 30,
            slippage_bps: 20,
            max_risk: 1,
            latency_ms: 400,
            finality_blocks: 12,
            min_liquidity: 1_000_000,
        }
    }

    /// The fixture the whole suite varies one field at a time: a two-venue
    /// USDC round trip needing 100k, expecting 520k back, authorising a 2k fee
    /// (the route's own 30 bps at 500k is 1.5k) and insisting on 5k of profit.
    fn unsigned() -> OpportunityPacket {
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
        }
    }

    fn packet() -> OpportunityPacket {
        sign_packet(unsigned(), KEY_ID, &signing_key()).expect("the fixture signs")
    }

    /// The fixture with one field changed, re-committed and re-signed, so a
    /// test proves the field it names is what was refused.
    fn signed_after(mutate: impl FnOnce(&mut OpportunityPacket)) -> OpportunityPacket {
        let mut packet = unsigned();
        mutate(&mut packet);
        sign_packet(packet, KEY_ID, &signing_key()).expect("the fixture signs")
    }

    #[test]
    fn a_signed_packet_verifies_for_its_signer() {
        let packet = packet();
        assert_ne!(packet.packet_hash, [0u8; 32]);
        assert_ne!(packet.execution_commitment, [0u8; 32]);
        assert!(verify_packet(&packet, &trusted(), 100).is_ok());
        assert!(verify_packet(&packet, &trusted(), 499).is_ok());
        assert!(packet.requires_proof("state"));
        assert!(!packet.requires_proof("execution"));
    }

    #[test]
    fn the_packet_hash_does_not_depend_on_map_insertion_order() {
        let one = signed_after(|packet| {
            packet.state_roots = BTreeMap::new();
            packet.state_roots.insert("ethereum".to_string(), [9u8; 32]);
            packet.state_roots.insert("solana".to_string(), [4u8; 32]);
            packet.proof_requirements = BTreeSet::new();
            packet.proof_requirements.insert("state".to_string());
            packet.proof_requirements.insert("execution".to_string());
        });
        let two = signed_after(|packet| {
            packet.state_roots = BTreeMap::new();
            packet.state_roots.insert("solana".to_string(), [4u8; 32]);
            packet.state_roots.insert("ethereum".to_string(), [9u8; 32]);
            packet.proof_requirements = BTreeSet::new();
            packet.proof_requirements.insert("execution".to_string());
            packet.proof_requirements.insert("state".to_string());
        });
        assert_eq!(one.packet_hash, two.packet_hash);
        assert_eq!(one.execution_commitment, two.execution_commitment);
        assert_eq!(one.encode().expect("encodes"), two.encode().expect("encodes"));
    }

    #[test]
    fn editing_the_strategy_id_after_signing_breaks_the_packet_hash() {
        let mut packet = packet();
        packet.strategy_id = "tri-arb-2".to_string();
        match validate_packet(&packet) {
            Err(OpportunityPacketError::HashMismatch { .. }) => {}
            other => panic!("expected a hash mismatch, got {other:?}"),
        }
    }

    #[test]
    fn an_execution_commitment_that_does_not_cover_the_terms_is_refused() {
        let mut packet = packet();
        packet.maximum_slippage_bps = 60;
        packet.packet_hash = packet.compute_hash().expect("the packet hashes");
        match validate_packet(&packet) {
            Err(OpportunityPacketError::ExecutionCommitmentMismatch { declared, .. }) => {
                assert_eq!(declared, packet.execution_commitment);
            }
            other => panic!("expected an execution commitment mismatch, got {other:?}"),
        }
    }

    #[test]
    fn a_profit_floor_the_numbers_do_not_reach_is_refused() {
        let mut packet = packet();
        packet.minimum_profit = 418_001;
        packet.execution_commitment = packet.execution_commitment().expect("commits");
        packet.packet_hash = packet.compute_hash().expect("hashes");
        match validate_packet(&packet) {
            Err(OpportunityPacketError::ProfitFloorNotReachable {
                expected_output,
                required_capital,
                maximum_fee,
                minimum_profit,
            }) => {
                assert_eq!(expected_output, 520_000);
                assert_eq!(required_capital, 100_000);
                assert_eq!(maximum_fee, 2_000);
                assert_eq!(minimum_profit, 418_001);
            }
            other => panic!("expected the floor to be refused, got {other:?}"),
        }
    }

    #[test]
    fn a_capital_window_that_is_inverted_is_refused() {
        let packet = signed_after(|packet| {
            packet.max_capital = 99_999;
        });
        match validate_packet(&packet) {
            Err(OpportunityPacketError::CapitalWindowInverted {
                required_capital,
                max_capital,
            }) => {
                assert_eq!(required_capital, 100_000);
                assert_eq!(max_capital, 99_999);
            }
            other => panic!("expected an inverted window, got {other:?}"),
        }
    }

    #[test]
    fn zero_required_capital_is_refused() {
        let packet = signed_after(|packet| {
            packet.required_capital = 0;
        });
        assert_eq!(
            validate_packet(&packet),
            Err(OpportunityPacketError::ZeroRequiredCapital)
        );
    }

    #[test]
    fn a_route_too_thin_for_the_authorised_capital_is_refused() {
        let packet = signed_after(|packet| {
            packet.route.min_liquidity = 499_999;
        });
        match validate_packet(&packet) {
            Err(OpportunityPacketError::RouteLiquidityBelowCapital {
                route_liquidity,
                max_capital,
            }) => {
                assert_eq!(route_liquidity, 499_999);
                assert_eq!(max_capital, 500_000);
            }
            other => panic!("expected the thin route to be refused, got {other:?}"),
        }
    }

    #[test]
    fn a_route_fee_above_the_packets_ceiling_is_refused() {
        let packet = signed_after(|packet| {
            packet.maximum_fee = 1_200;
        });
        assert!(packet.expected_output - packet.required_capital - packet.maximum_fee > packet.minimum_profit);
        match validate_packet(&packet) {
            Err(OpportunityPacketError::RouteFeeAboveCeiling { route_fee, maximum_fee }) => {
                assert_eq!(route_fee, 1_500);
                assert_eq!(maximum_fee, 1_200);
            }
            other => panic!("expected the fee ceiling to bind, got {other:?}"),
        }
    }

    #[test]
    fn a_route_fee_that_overflows_at_the_authorised_capital_is_refused() {
        let packet = signed_after(|packet| {
            packet.max_capital = u128::MAX;
            packet.expected_output = u128::MAX;
            packet.route.min_liquidity = u128::MAX;
        });
        match validate_packet(&packet) {
            Err(OpportunityPacketError::RouteFeeOverflow { fee_bps }) => assert_eq!(fee_bps, 30),
            other => panic!("expected the fee product to overflow, got {other:?}"),
        }
    }

    #[test]
    fn a_slippage_ceiling_below_the_routes_own_slippage_is_refused() {
        let packet = signed_after(|packet| {
            packet.maximum_slippage_bps = 19;
        });
        match validate_packet(&packet) {
            Err(OpportunityPacketError::RouteSlippageAboveCeiling {
                route_slippage_bps,
                maximum_slippage_bps,
            }) => {
                assert_eq!(route_slippage_bps, 20);
                assert_eq!(maximum_slippage_bps, 19);
            }
            other => panic!("expected the slippage ceiling to bind, got {other:?}"),
        }
    }

    #[test]
    fn a_slippage_ceiling_above_one_hundred_percent_is_refused() {
        let packet = signed_after(|packet| {
            packet.maximum_slippage_bps = MAX_SLIPPAGE_BPS + 1;
        });
        match validate_packet(&packet) {
            Err(OpportunityPacketError::SlippageCeilingAboveMaximum {
                declared_bps,
                maximum_bps,
            }) => {
                assert_eq!(declared_bps, 10_001);
                assert_eq!(maximum_bps, MAX_SLIPPAGE_BPS);
            }
            other => panic!("expected the ceiling to be refused, got {other:?}"),
        }
    }

    #[test]
    fn a_route_with_no_venues_is_refused() {
        let packet = signed_after(|packet| {
            packet.route.venues = Vec::new();
            packet.route.assets = Vec::new();
        });
        assert_eq!(validate_packet(&packet), Err(OpportunityPacketError::EmptyRoute));
    }

    #[test]
    fn a_route_whose_assets_do_not_match_its_venues_is_refused() {
        let packet = signed_after(|packet| {
            packet.route.assets = vec!["USDC".to_string(), "USDC".to_string()];
        });
        match validate_packet(&packet) {
            Err(OpportunityPacketError::RouteAssetCountMismatch { venues, assets }) => {
                assert_eq!(venues, 2);
                assert_eq!(assets, 2);
            }
            other => panic!("expected the asset count to be refused, got {other:?}"),
        }
    }

    #[test]
    fn an_unnamed_venue_is_refused() {
        let packet = signed_after(|packet| {
            packet.route.venues = vec!["dex-a".to_string(), "  ".to_string()];
        });
        assert_eq!(
            validate_packet(&packet),
            Err(OpportunityPacketError::UnnamedVenue { index: 1 })
        );
    }

    #[test]
    fn a_packet_with_no_state_root_is_refused() {
        let packet = signed_after(|packet| {
            packet.state_roots = BTreeMap::new();
        });
        assert_eq!(validate_packet(&packet), Err(OpportunityPacketError::NoStateRoots));
    }

    #[test]
    fn a_zero_state_root_is_refused() {
        let packet = signed_after(|packet| {
            packet.state_roots = BTreeMap::from([("solana".to_string(), [0u8; 32])]);
        });
        match validate_packet(&packet) {
            Err(OpportunityPacketError::ZeroStateRoot { domain }) => assert_eq!(domain, "solana"),
            other => panic!("expected the zero root to be refused, got {other:?}"),
        }
    }

    #[test]
    fn a_packet_that_names_no_artifact_is_refused() {
        let packet = signed_after(|packet| {
            packet.artifact_hash = [0u8; 32];
        });
        assert_eq!(validate_packet(&packet), Err(OpportunityPacketError::ZeroArtifactHash));
    }

    #[test]
    fn a_packet_that_names_no_strategy_is_refused() {
        let packet = signed_after(|packet| {
            packet.strategy_id = "   ".to_string();
        });
        assert_eq!(validate_packet(&packet), Err(OpportunityPacketError::EmptyStrategyId));
    }

    #[test]
    fn an_unnamed_proof_requirement_is_refused() {
        let packet = signed_after(|packet| {
            packet.proof_requirements = BTreeSet::from(["".to_string()]);
        });
        assert_eq!(
            validate_packet(&packet),
            Err(OpportunityPacketError::UnnamedProofRequirement)
        );
    }

    #[test]
    fn an_unsupported_version_is_refused_before_anything_else() {
        let packet = signed_after(|packet| {
            packet.version = OPPORTUNITY_PACKET_VERSION + 1;
        });
        assert_eq!(
            validate_packet(&packet),
            Err(OpportunityPacketError::UnsupportedVersion {
                version: OPPORTUNITY_PACKET_VERSION + 1
            })
        );
    }

    #[test]
    fn an_unsigned_packet_is_refused() {
        let packet = unsigned().finalize().expect("the packet commits");
        assert_eq!(validate_packet(&packet), Ok(()));
        assert_eq!(
            verify_packet(&packet, &trusted(), 100),
            Err(OpportunityPacketError::MissingSignature)
        );
    }

    #[test]
    fn a_packet_from_a_signer_the_policy_does_not_trust_is_refused() {
        let packet = sign_packet(unsigned(), "solver-2", &signing_key()).expect("the packet signs");
        assert_eq!(
            verify_packet(&packet, &trusted(), 100),
            Err(OpportunityPacketError::UntrustedSigner("solver-2".to_string()))
        );
    }

    #[test]
    fn a_signature_presenting_a_key_the_policy_does_not_hold_for_that_signer_is_refused() {
        let packet = sign_packet(unsigned(), KEY_ID, &other_key()).expect("the packet signs");
        assert_eq!(
            verify_packet(&packet, &trusted(), 100),
            Err(OpportunityPacketError::UntrustedSigner(KEY_ID.to_string()))
        );
    }

    #[test]
    fn a_packet_whose_signature_was_replaced_is_refused() {
        let mut packet = packet();
        packet.signature = Some(OpportunitySignature {
            key_id: KEY_ID.to_string(),
            public_key: signing_key().verifying_key().to_bytes(),
            signature: vec![0u8; 64],
        });
        assert_eq!(
            verify_packet(&packet, &trusted(), 100),
            Err(OpportunityPacketError::InvalidSignature)
        );
    }

    #[test]
    fn a_packet_signed_over_a_different_hash_is_refused() {
        let mut packet = packet();
        packet.packet_hash = [1u8; 32];
        assert_eq!(
            verify_packet(&packet, &trusted(), 100),
            Err(OpportunityPacketError::HashMismatch {
                expected: [1u8; 32],
                actual: packet.compute_hash().expect("the packet hashes"),
            })
        );
    }

    #[test]
    fn a_packet_at_or_past_its_deadline_is_refused() {
        let packet = packet();
        assert_eq!(
            verify_packet(&packet, &trusted(), packet.deadline_blocks),
            Err(OpportunityPacketError::Expired {
                deadline_blocks: 500,
                at_block: 500
            })
        );
        assert_eq!(
            verify_packet(&packet, &trusted(), 501),
            Err(OpportunityPacketError::Expired {
                deadline_blocks: 500,
                at_block: 501
            })
        );
    }

    #[test]
    fn the_same_packet_is_admitted_only_once() {
        let packet = packet();
        let mut ledger = OpportunityPacketLedger::new();
        assert!(!ledger.has_admitted(&packet.packet_hash));
        assert_eq!(ledger.admit(&packet, &trusted(), 100), Ok(()));
        assert!(ledger.has_admitted(&packet.packet_hash));
        assert_eq!(
            ledger.admit(&packet, &trusted(), 100),
            Err(OpportunityPacketError::PacketAlreadySeen(packet.packet_hash))
        );
    }

    #[test]
    fn a_distinct_packet_from_the_same_solver_is_admitted() {
        let first = packet();
        let second = signed_after(|packet| {
            packet.expected_output += 1_000;
        });
        assert_ne!(first.packet_hash, second.packet_hash);
        let mut ledger = OpportunityPacketLedger::new();
        assert_eq!(ledger.admit(&first, &trusted(), 100), Ok(()));
        assert_eq!(ledger.admit(&second, &trusted(), 100), Ok(()));
    }

    #[test]
    fn expiry_is_reported_ahead_of_replay_when_both_apply() {
        let packet = packet();
        let mut ledger = OpportunityPacketLedger::new();
        assert_eq!(ledger.admit(&packet, &trusted(), 100), Ok(()));
        // Admission checks expiry before replay, so a packet presented at its
        // deadline reports the deadline rather than the earlier admission, and
        // the ledger only reports a replay when the packet is otherwise
        // admissible.
        assert_eq!(
            ledger.admit(&packet, &trusted(), 500),
            Err(OpportunityPacketError::Expired {
                deadline_blocks: 500,
                at_block: 500
            })
        );
        assert_eq!(
            ledger.admit(&packet, &trusted(), 100),
            Err(OpportunityPacketError::PacketAlreadySeen(packet.packet_hash))
        );
    }

    #[test]
    fn a_packet_survives_a_portable_round_trip() {
        let packet = packet();
        let bytes = packet.encode().expect("the packet encodes");
        let decoded = OpportunityPacket::decode(&bytes).expect("the packet decodes");
        assert_eq!(decoded, packet);
        assert_eq!(verify_packet(&decoded, &trusted(), 100), Ok(()));
    }
}
