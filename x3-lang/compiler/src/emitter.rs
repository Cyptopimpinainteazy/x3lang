//! X3IR -> Bytecode/Runtime Dispatch emitter.
//!
//! This module converts X3IR operations into executable bytecode suitable
//! for the X3 runtime or specific chain emitters (EVM, SVM, etc.).

use crate::ir::{
    ChainMetricKind, ChoiceCriterion, ComparisonOp, CrdtKind, EmergencyKind, LifecycleKind, Operation, ProofKind,
    SerialFormat, StorageKind, TradingOperation, VectorOp, X3IR,
};
// Import shared opcode constants
use crate::spec::opcodes::*;
use std::io::Write;
use x3_lang_common::{
    encode_asset_op_payload, encode_bridge_payload, encode_capability_payload, AssetOpPayload, BridgePayload,
    CapabilityPayload, X3Error,
};
/// Pad `bytecode` so its length is a multiple of 4, which is what keeps every
/// *subsequent* instruction starting on a 4-byte boundary.
///
/// Note that the metadata header is written unpadded, on purpose: the executor's
/// `first_instruction_pc` and this crate's `disassemble` both walk it unpadded,
/// so the first instruction after a header sits at `3 + len + 1` rather than at
/// a rounded offset. An earlier version of this comment claimed the VM verifier
/// required every instruction — including that first one — to be aligned, and
/// `vm/src/verifier.rs` acted on that claim; the two disagreed for any nonce
/// whose record was 1 or 3 bytes short of a multiple of four.
fn pad_to_4(bytecode: &mut Vec<u8>) {
    let rem = bytecode.len() % 4;
    if rem != 0 {
        for _ in 0..(4 - rem) {
            bytecode.push(0);
        }
    }
}

/// Emit X3IR to bytecode suitable for the X3 runtime
pub fn emit_x3ir(ir: &X3IR) -> Result<Vec<u8>, X3Error> {
    let mut bytecode = Vec::new();

    // Header: version + metadata
    bytecode.write_all(&[BYTECODE_VERSION_1])?;

    // Encode metadata
    if let Some(nonce) = &ir.metadata.nonce {
        bytecode.write_all(&[META_NONCE])?;
        bytecode.write_all(&(nonce.len() as u16).to_le_bytes())?;
        bytecode.write_all(nonce.as_bytes())?;
    }

    if let Some(chain_id) = ir.metadata.chain_id {
        bytecode.write_all(&[META_CHAIN_ID])?;
        bytecode.write_all(&chain_id.to_le_bytes())?;
    }

    // The version binding (PHASE 45). Written before the operations so a loader can
    // reject an artifact before it reads an instruction, and fixed-width so its walk is
    // the same arithmetic everywhere.
    bytecode.write_all(&[META_VERSIONS])?;
    for version in [
        LANGUAGE_VERSION,
        COMPILER_FORMAT_VERSION,
        IR_VERSION,
        VM_VERSION,
        POLICY_VERSION,
    ] {
        bytecode.write_all(&version.to_le_bytes())?;
    }

    // Encode operations
    for op in &ir.operations {
        emit_operation(op, &mut bytecode)?;
    }

    // Pad to 4-byte alignment
    while bytecode.len() % 4 != 0 {
        bytecode.push(0);
    }

    Ok(bytecode)
}

/// Emit a single operation to bytecode
fn emit_operation(op: &Operation, bytecode: &mut Vec<u8>) -> Result<(), X3Error> {
    match op {
        Operation::Lock { .. } => emit_asset_op(LOCK, op, bytecode)?,
        Operation::Mint { .. } => emit_asset_op(MINT, op, bytecode)?,
        Operation::Burn { .. } => emit_asset_op(BURN, op, bytecode)?,
        Operation::Release { .. } => emit_asset_op(RELEASE, op, bytecode)?,
        Operation::Swap { .. } => emit_asset_op(SWAP, op, bytecode)?,
        Operation::Bridge { .. } => emit_bridge_op(op, bytecode)?,
        Operation::AtomicBegin => {
            bytecode.write_all(&[ATOMIC_BEGIN])?;
            bytecode.write_all(&0u16.to_le_bytes())?;
        }
        // `[ATOMIC_CHOICE][u16 len][criterion:paths:selected]`.
        //
        // The record is carried in the artifact rather than dropped after
        // lowering: a reader of the bytecode can see that this body is one of N
        // verified branches and which criterion picked it, which is the whole
        // claim the construct makes.
        //
        // It is a *payload* op rather than a fixed frame on purpose. A fixed
        // frame in this format is three bytes, and the reader advances a fixed
        // instruction with `align4(pc + 3)` regardless of what the writer
        // emitted, so a four-byte fixed frame is only readable when it does not
        // start at an offset congruent to 1 mod 4. `atomic_choice` is emitted
        // first in its program, which is exactly that offset, and the stream
        // desynced: `x3c run examples/atomic_choice.x3` failed with
        // `InvalidOpcode(3)` — the reader had advanced into the middle of the
        // record. A payload frame is consumed as `align4(pc + 3 + len)`, the
        // same expression the writer pads by, so it is correct at any offset.
        Operation::AtomicChoice {
            paths,
            criterion,
            selected,
        } => {
            let criterion_code = match criterion {
                ChoiceCriterion::HighestNetOutput => CHOICE_CRITERION_HIGHEST_NET_OUTPUT,
                ChoiceCriterion::FewestHops => CHOICE_CRITERION_FEWEST_HOPS,
                ChoiceCriterion::LowestDeclaredFee => CHOICE_CRITERION_LOWEST_DECLARED_FEE,
            };
            if *selected >= *paths {
                return Err(X3Error::CodegenError {
                    message: format!(
                        "atomic choice selects path {selected} of {paths}; the selected index must \
                         name a declared path"
                    ),
                    span: None,
                });
            }
            let payload = format!("{criterion_code}:{paths}:{selected}");
            bytecode.write_all(&[ATOMIC_CHOICE])?;
            bytecode.write_all(&(payload.len() as u16).to_le_bytes())?;
            bytecode.write_all(payload.as_bytes())?;
        }
        Operation::AtomicEnd => {
            bytecode.write_all(&[ATOMIC_END])?;
            bytecode.write_all(&0u16.to_le_bytes())?;
        }
        Operation::If {
            condition,
            then_ops,
            else_ops,
        } => {
            // Refused rather than written.
            //
            // This is refused here as well as in the IR verifier because
            // `emit_x3ir` is public: a caller that assembles an IR by hand gets a
            // refusal instead of a record no reader can follow.
            //
            // The record this used to write was `[IF][u16 cond_len][cond][u32
            // then_len][then][u32 else_len][else]` with each branch emitted into
            // its own vector — so the branch's inner instructions were padded to a
            // boundary of the *branch* rather than of the stream — while the VM's
            // `IF` reads a register and skips whole four-byte instructions. Neither
            // half can be repaired on its own: the stream frames instructions with
            // a width that varies and pads them absolutely, so a branch needs an
            // explicit target rather than a count of instructions, and the
            // condition needs codegen into a register, which the compiler does not
            // emit at all. Measured before this refusal: the artifact built,
            // `x3c explain` printed the condition text as opcodes, and `x3c run`
            // failed with `X3_VERIFY_FAILED: OutOfBounds(292)`. TICKET-058.
            let _ = (condition, then_ops, else_ops);
            return Err(X3Error::CodegenError {
                message: "cannot emit `if`: this VM branches on a register and skips whole four-byte \
                          instructions, while a compiler stream frames instructions with a width that \
                          varies and pads them to absolute four-byte boundaries, so the record would \
                          have no target a reader could follow or an executor could jump to"
                    .to_string(),
                span: None,
            });
        }
        Operation::Loop { max_iterations, body } => {
            // Refused for the same reason as `if`; see the comment there.
            let _ = (max_iterations, body);
            return Err(X3Error::CodegenError {
                message: "cannot emit `loop`: this VM branches on a register and skips whole \
                          four-byte instructions, while a compiler stream frames instructions with a \
                          width that varies and pads them to absolute four-byte boundaries, so the \
                          record would have no target a reader could follow or an executor could jump \
                          back to"
                    .to_string(),
                span: None,
            });
        }
        Operation::Require { .. } => {
            // `[REQUIRE][comparison][threshold u16]`. Mostly STATIC: a guard a program
            // writes is a constraint the compiler checks against the declarations, so
            // the instruction records it and has nothing to test. Two modes do test: the
            // nonce guard, whose quantity `NONCE_UNUSED` leaves in `r0`, and the economic
            // floors a *plan generator* emits, whose quantity is a measurement a host
            // reports in the reply to the trade (`CAPABILITY_REPLY_MEASURED_TAG`).
            //
            // Every guard in the language asserts something about the artifact's
            // configuration — `require relayer_quorum >= 3` against
            // `relayers { quorum 3_of_5 }`, `require finality.ethereum >= 32`
            // against the settlement path's finality policy — and the
            // compile-time verifier is what checks those. The instruction records
            // the guard in the artifact; it does not test run-time state.
            //
            // A comparison (`REQUIRE_COMPARE_GE`, evaluated by the executor) is
            // defined and tested, but nothing emits it yet: a guard would have to
            // find its quantity in `r0`, and no instruction puts it there. The
            // declarations sit at the top of a program and the guards at the
            // bottom, so `r0` at a guard holds whatever the last unrelated
            // instruction left — which is exactly the accident that used to make
            // guards fail. Emitting a comparison against that would be a new
            // wrong answer rather than a fix; the data flow has to arrive first
            // (TICKET-027).
            let guard_operator = match op {
                Operation::Require {
                    comparison: Some(comparison),
                    ..
                } => match comparison {
                    ComparisonOp::Less => GUARD_OP_LT,
                    ComparisonOp::LessOrEqual => GUARD_OP_LE,
                    ComparisonOp::Greater => GUARD_OP_GT,
                    ComparisonOp::GreaterOrEqual => GUARD_OP_GE,
                    ComparisonOp::Equal => GUARD_OP_EQ,
                    ComparisonOp::NotEqual => GUARD_OP_NE,
                },
                _ => GUARD_OP_NONE,
            };
            // Which comparison the VM makes. Every guard is STATIC — the
            // compiler checked it and there is no run-time quantity — *except*
            // the nonce guard, whose quantity is whether the nonce is new: the
            // `NONCE_UNUSED` instruction emitted immediately before it puts that
            // in `r0`, so this one compares (`r0 >= 1`) and a replay fails at the
            // guard rather than at a later, unrelated instruction.
            let (mode, threshold) = if matches!(
                op,
                Operation::Require {
                    kind: crate::ir::RequireKind::NonceUnused,
                    ..
                }
            ) {
                (REQUIRE_COMPARE_GE, 1u16)
            } else if let Operation::Require {
                condition: crate::ir::Condition::FinalityPolicy { blocks, .. },
                ..
            } = op
            {
                // A finality policy's depth travels in the operand, which is what
                // makes it re-checkable from the artifact: a replayer can compare
                // the guards it reads against the number the declaration states
                // (TICKET-059). Zero means the declaration states no depth — the
                // parser refuses `blocks 0`, so the two cannot be confused — and a
                // depth the operand cannot hold is refused rather than truncated.
                let blocks = blocks.unwrap_or(0);
                let threshold = u16::try_from(blocks).map_err(|_| X3Error::CodegenError {
                    message: format!(
                        "finality depth {blocks} does not fit the instruction's operand ({})",
                        u16::MAX
                    ),
                    span: None,
                })?;
                (REQUIRE_COMPARE_STATIC, threshold)
            } else if let Operation::Require {
                measured: true,
                kind: crate::ir::RequireKind::ProfitThreshold,
                condition: crate::ir::Condition::Expression { expr },
                ..
            } = op
            {
                // A floor the compiler itself emitted *after* the trade it bounds, so
                // it is a post-condition on what the trade realised. The mode says the
                // comparison is against a measurement and the executor refuses when no
                // host reported one — it never compares register residue, which is what
                // makes this an enforced constraint rather than a record (TICKET-027).
                //
                // The floor travels in the operand in basis points, because the operand
                // is two bytes: an absolute floor would not fit, and comparing a
                // basis-point floor against an absolute amount would be a units mismatch
                // dressed as enforcement.
                (REQUIRE_COMPARE_MEASURED_PROFIT, guard_bps(expr, "profit floor")?)
            } else if let Operation::Require {
                measured: true,
                kind: crate::ir::RequireKind::SlippageTolerance,
                condition: crate::ir::Condition::Expression { expr },
                ..
            } = op
            {
                (REQUIRE_COMPARE_MEASURED_SLIPPAGE, guard_bps(expr, "slippage ceiling")?)
            } else {
                (REQUIRE_COMPARE_STATIC, 0u16)
            };
            bytecode.write_all(&[REQUIRE, require_flags(mode, guard_operator)])?;
            bytecode.write_all(&threshold.to_le_bytes())?;
        }
        Operation::OnFail { .. } => {
            bytecode.write_all(&[ON_FAIL])?;
            bytecode.write_all(&0u16.to_le_bytes())?;
        }
        Operation::OnTimeout { .. } => {
            // Operand 0 = "no instruction budget". The instruction is the
            // program's declaration that a timeout policy exists; `duration_blocks`
            // stays in the IR, where the verifier checks it is non-zero and
            // within range, and where the timeout/refund engine reads it.
            //
            // It must not become an instruction budget: the two are different
            // units, and the VM has no block height to compare against. The
            // operand is explicit (rather than "read r0") because the deadline
            // used to be taken from a register nothing set for it, so the
            // outcome depended on the previous instruction's residue.
            bytecode.write_all(&[ON_TIMEOUT])?;
            bytecode.write_all(&0u16.to_le_bytes())?;
        }
        Operation::Emit { name, data } => {
            bytecode.write_all(&[EMIT])?;
            let payload = format!("{}:{:?}", name, data);
            bytecode.write_all(&(payload.len() as u16).to_le_bytes())?;
            bytecode.write_all(payload.as_bytes())?;
        }
        // `[ROUTE_FALLBACK][u16 len][venue,venue,...]`. The approved list is
        // the payload because a runtime can only restrict itself to the
        // compiler's approvals if the approvals travel with the artifact; a
        // bare count would be a claim the runtime could not act on.
        // `[PARALLEL_PLAN][u16 len][legs=<n>;waves=a,b|c;edges=a->c,b->c]`.
        //
        // The plan travels with the artifact because "these legs are
        // independent" is a claim about them, and a reader who cannot see the
        // waves cannot check it. Encoded as a payload frame like every other
        // record here: a payload is consumed as `align4(pc + 3 + len)`, the
        // expression the writer pads by, so it is correct at any offset.
        // `[FEATURE_ALLOW][0][u16 code]` — a three-byte frame, like every other
        // fixed instruction here. A four-byte one would desync the reader when it
        // lands at an offset congruent to 1 mod 4, which is where the first
        // instruction of a stream sits.
        Operation::FeatureAllow { feature, .. } => {
            // `[FEATURE_ALLOW][flags = 0][code]`. Three bytes of content, then
            // the per-instruction padding — the shape every fixed-frame
            // instruction here uses, and the only shape the reader can follow at
            // any offset.
            bytecode.write_all(&[FEATURE_ALLOW, 0, *feature])?;
        }
        // `[STRATEGY_LICENSE][u16 len][creator=..;royalty_bps=..;executions=..;expires_block=..;split=a:1,b:2]`
        Operation::StrategyLicense {
            creator,
            royalty_bps,
            executions,
            expires_block,
            split,
        } => {
            let split_text = split
                .iter()
                .map(|(recipient, bps)| format!("{recipient}:{bps}"))
                .collect::<Vec<_>>()
                .join(",");
            let payload = format!(
                "creator={creator};royalty_bps={royalty_bps};executions={};expires_block={};split={split_text}",
                executions
                    .map(|value| value.to_string())
                    .unwrap_or_else(|| "-".to_string()),
                expires_block
                    .map(|value| value.to_string())
                    .unwrap_or_else(|| "-".to_string())
            );
            if payload.len() > u16::MAX as usize {
                return Err(X3Error::CodegenError {
                    message: format!("strategy licence payload too large: {} bytes", payload.len()),
                    span: None,
                });
            }
            bytecode.write_all(&[STRATEGY_LICENSE])?;
            bytecode.write_all(&(payload.len() as u16).to_le_bytes())?;
            bytecode.write_all(payload.as_bytes())?;
        }
        Operation::ParallelPlan {
            waves,
            edges,
            domains,
            settlement,
        } => {
            let leg_count: usize = waves.iter().map(|wave| wave.len()).sum();
            let wave_text = waves.iter().map(|wave| wave.join(",")).collect::<Vec<_>>().join("|");
            let edge_text = edges
                .iter()
                .map(|(from, to)| format!("{from}->{to}"))
                .collect::<Vec<_>>()
                .join(",");
            // Domains are carried per leg because "is this plan multi-VM" is a
            // question about the legs, and a reader who cannot see which VM
            // each leg runs on cannot answer it.
            let domain_text = domains
                .iter()
                .map(|(leg, domains)| format!("{leg}:{}", domains.iter().cloned().collect::<Vec<_>>().join("+")))
                .collect::<Vec<_>>()
                .join(",");
            // The settlement section is what a coordinator acts on, so it is in
            // the artifact rather than left to be re-derived: `wave:domains:
            // proofs:recoverable`, with `-` for an empty set.
            let settlement_text = settlement
                .iter()
                .map(|wave| {
                    let domains = if wave.domains.is_empty() {
                        "-".to_string()
                    } else {
                        wave.domains.iter().cloned().collect::<Vec<_>>().join("+")
                    };
                    let proofs = if wave.outstanding_proofs.is_empty() {
                        "-".to_string()
                    } else {
                        wave.outstanding_proofs.iter().cloned().collect::<Vec<_>>().join("+")
                    };
                    format!(
                        "{}:{}:{}:{}",
                        wave.wave,
                        domains,
                        proofs,
                        if wave.locally_recoverable {
                            "local"
                        } else {
                            "coordinated"
                        }
                    )
                })
                .collect::<Vec<_>>()
                // `|` between records: `,` already separates the domains and
                // proofs within one, and a separator that appears inside the
                // thing it separates cannot be parsed back.
                .join("|");
            let payload = format!(
                "legs={leg_count};waves={wave_text};edges={edge_text};domains={domain_text};settle={settlement_text}"
            );
            if payload.len() > u16::MAX as usize {
                return Err(X3Error::CodegenError {
                    message: format!("parallel plan payload too large: {} bytes", payload.len()),
                    span: None,
                });
            }
            bytecode.write_all(&[PARALLEL_PLAN])?;
            bytecode.write_all(&(payload.len() as u16).to_le_bytes())?;
            bytecode.write_all(payload.as_bytes())?;
        }
        Operation::RouteFallback { approved } => {
            if approved.is_empty() {
                return Err(X3Error::CodegenError {
                    message: "route fallback with no approved venues must not be emitted".to_string(),
                    span: None,
                });
            }
            let payload = approved.join(",");
            if payload.len() > u16::MAX as usize {
                return Err(X3Error::CodegenError {
                    message: format!("route fallback payload too large: {} bytes", payload.len()),
                    span: None,
                });
            }
            bytecode.write_all(&[ROUTE_FALLBACK])?;
            bytecode.write_all(&(payload.len() as u16).to_le_bytes())?;
            bytecode.write_all(payload.as_bytes())?;
        }
        Operation::Call { function, args } => {
            bytecode.write_all(&[CALL_HOST])?;
            let payload = format!("{}:{:?}", function, args);
            bytecode.write_all(&(payload.len() as u16).to_le_bytes())?;
            bytecode.write_all(payload.as_bytes())?;
        }
        Operation::VenueOrder { .. } => emit_payload_op(VENUE_ORDER, op, bytecode)?,
        Operation::Rebalance { .. } => emit_payload_op(REBALANCE_TARGET, op, bytecode)?,
        Operation::GpuDispatch { .. } => emit_payload_op(GPU_DISPATCH, op, bytecode)?,
        Operation::Simulate { .. } => emit_payload_op(SIMULATE, op, bytecode)?,
        Operation::ScheduledDispatch { .. } => emit_payload_op(SCHEDULED_DISPATCH, op, bytecode)?,
        Operation::IntentResolve { .. } => emit_payload_op(INTENT_RESOLVE, op, bytecode)?,
        Operation::CrdtOp { .. } => emit_payload_op(CRDT_OP, op, bytecode)?,
        Operation::ProofVerify { .. } => emit_payload_op(PROOF_VERIFY, op, bytecode)?,
        Operation::StorageOp { .. } => emit_payload_op(STORAGE_OP, op, bytecode)?,
        Operation::Pathfind { .. } => emit_payload_op(PATHFIND, op, bytecode)?,
        Operation::MempoolScan { .. } => emit_payload_op(MEMPOOL_SCAN, op, bytecode)?,
        Operation::OracleRequest { .. } => emit_payload_op(ORACLE_REQUEST, op, bytecode)?,
        Operation::EmergencyControl { .. } => emit_payload_op(EMERGENCY_CONTROL, op, bytecode)?,
        Operation::Lifecycle { .. } => emit_payload_op(LIFECYCLE, op, bytecode)?,
        Operation::Serialize { .. } => emit_payload_op(SERIALIZE, op, bytecode)?,
        Operation::Deserialize { .. } => emit_payload_op(DESERIALIZE, op, bytecode)?,
        Operation::GasEstimate { .. } => emit_payload_op(GAS_ESTIMATE, op, bytecode)?,
        Operation::ChainMetric { .. } => emit_payload_op(CHAIN_METRIC, op, bytecode)?,
        Operation::EventProvenance { .. } => emit_payload_op(EVENT_PROVENANCE, op, bytecode)?,
        Operation::MultiHopSwap { .. } => emit_payload_op(MULTI_HOP_SWAP, op, bytecode)?,
        Operation::VectorMath { .. } => emit_payload_op(VECTOR_MATH, op, bytecode)?,
        Operation::RoleCheck { .. } => emit_payload_op(ROLE_CHECK, op, bytecode)?,
        Operation::MultisigCheck { .. } => emit_payload_op(MULTISIG_CHECK, op, bytecode)?,
        Operation::VersionMeta { .. } => emit_payload_op(VERSION_META, op, bytecode)?,
        Operation::StorageNamespace { .. } => emit_payload_op(STORAGE_NAMESPACE, op, bytecode)?,
        Operation::AbiExport { .. } => emit_payload_op(ABI_EXPORT, op, bytecode)?,
        Operation::DocEmbed { .. } => emit_payload_op(DOC_EMBED, op, bytecode)?,
        Operation::GasAdaptive { .. } => emit_payload_op(GAS_ADAPTIVE, op, bytecode)?,
        Operation::Bounty { .. } => emit_payload_op(BOUNTY, op, bytecode)?,
        // B-52 feature lock operations
        Operation::RouteScore { .. } => emit_payload_op(ROUTE_SCORE, op, bytecode)?,
        Operation::SolverBid { .. } => emit_payload_op(SOLVER_BID, op, bytecode)?,
        Operation::RelayerAttest { .. } => emit_payload_op(RELAYER_ATTEST, op, bytecode)?,
        Operation::RpcConsensus { .. } => emit_payload_op(RPC_CONSENSUS, op, bytecode)?,
        Operation::RiskScore { .. } => emit_payload_op(RISK_SCORE, op, bytecode)?,
        Operation::InvariantCheck { .. } => emit_payload_op(INVARIANT_CHECK, op, bytecode)?,
        Operation::PrivacyCommit { .. } => emit_payload_op(PRIVACY_COMMIT, op, bytecode)?,
        Operation::ProofRequired { .. } => emit_payload_op(PROOF_REQUIRED, op, bytecode)?,
        Operation::NonceUnused { .. } => emit_payload_op(NONCE_UNUSED, op, bytecode)?,
        Operation::VmAdapterCall { .. } => emit_payload_op(VM_ADAPTER_CALL, op, bytecode)?,
        Operation::ModeCheck { .. } => emit_payload_op(MODE_CHECK, op, bytecode)?,
        Operation::PackageImport { .. } => emit_payload_op(PACKAGE_IMPORT, op, bytecode)?,
        Operation::RefundPolicy { .. } => emit_payload_op(REFUND_POLICY, op, bytecode)?,
        Operation::Trading(trading) => emit_trading_op(trading, bytecode)?,
        Operation::Nop => {
            bytecode.write_all(&[NOP])?;
            bytecode.write_all(&0u16.to_le_bytes())?;
        }
    }
    pad_to_4(bytecode);
    Ok(())
}

fn emit_bridge_op(op: &Operation, bytecode: &mut Vec<u8>) -> Result<(), X3Error> {
    bytecode.write_all(&[BRIDGE])?;
    let payload = encode_bridge_payload(&operation_to_bridge_payload(op)?).map_err(|err| X3Error::CodegenError {
        message: format!("failed to encode bridge payload: {err}"),
        span: None,
    })?;
    if payload.len() > u16::MAX as usize {
        return Err(X3Error::CodegenError {
            message: "bridge operation payload too large".to_string(),
            span: None,
        });
    }
    bytecode.write_all(&(payload.len() as u16).to_le_bytes())?;
    bytecode.write_all(&payload)?;
    Ok(())
}

fn emit_asset_op(opcode: u8, op: &Operation, bytecode: &mut Vec<u8>) -> Result<(), X3Error> {
    bytecode.write_all(&[opcode])?;
    let payload = encode_asset_op_payload(&operation_to_asset_payload(op)?).map_err(|err| X3Error::CodegenError {
        message: format!("failed to encode asset payload: {err}"),
        span: None,
    })?;
    if payload.len() > u16::MAX as usize {
        return Err(X3Error::CodegenError {
            message: format!("asset operation payload too large for opcode 0x{opcode:02x}"),
            span: None,
        });
    }
    bytecode.write_all(&(payload.len() as u16).to_le_bytes())?;
    bytecode.write_all(&payload)?;
    Ok(())
}

fn emit_payload_op(opcode: u8, op: &Operation, bytecode: &mut Vec<u8>) -> Result<(), X3Error> {
    bytecode.write_all(&[opcode])?;
    let payload = encode_capability_payload(&operation_to_payload(op)?).map_err(|err| X3Error::CodegenError {
        message: format!("failed to encode capability payload: {err}"),
        span: None,
    })?;
    if payload.len() > u16::MAX as usize {
        return Err(X3Error::CodegenError {
            message: format!("operation payload too large for opcode 0x{opcode:02x}"),
            span: None,
        });
    }
    bytecode.write_all(&(payload.len() as u16).to_le_bytes())?;
    bytecode.write_all(&payload)?;
    Ok(())
}

/// The basis points a generated guard's condition states.
///
/// The condition is an expression, and a floor written as a literal is the only shape
/// this reads: a floor the emitter cannot evaluate is refused rather than defaulted to
/// zero, because a guard of zero passes everything and would be a guard in name only.
fn guard_bps(expr: &str, what: &str) -> Result<u16, X3Error> {
    expr.trim().parse::<u16>().map_err(|_| X3Error::CodegenError {
        message: format!(
            "the {what} '{expr}' is not a count of basis points this instruction can carry; a \
             measured guard's threshold is the operand and must be a basis-point figure"
        ),
        span: None,
    })
}

/// Return the stable opcode for a trading operation variant.
pub fn trading_opcode(op: &TradingOperation) -> u8 {
    match op {
        TradingOperation::BeginAtomicTrade { .. } => TRADING_BEGIN,
        TradingOperation::OpenDebt { .. } => TRADING_OPEN_DEBT,
        TradingOperation::ExecuteSwap { .. } => TRADING_EXECUTE_SWAP,
        TradingOperation::CloseDebt { .. } => TRADING_CLOSE_DEBT,
        TradingOperation::AssertMinNetProfit { .. } => TRADING_ASSERT_MIN_PROFIT,
        TradingOperation::AssertAllDebtsClosed => TRADING_ASSERT_ALL_DEBTS,
        TradingOperation::AssertInvariant { .. } => TRADING_ASSERT_INVARIANT,
        TradingOperation::Bridge { .. } => TRADING_BRIDGE,
        TradingOperation::EmitTradeReceipt => TRADING_EMIT_RECEIPT,
        TradingOperation::CommitAtomicTrade => TRADING_COMMIT,
        TradingOperation::AbortAtomicTrade => TRADING_ABORT,
    }
}

/// Encode a trading operation payload deterministically.
pub fn encode_trading_operation(op: &TradingOperation) -> Result<Vec<u8>, X3Error> {
    serde_json::to_vec(op).map_err(|err| X3Error::CodegenError {
        message: format!("failed to encode trading operation: {err}"),
        span: None,
    })
}

/// Decode and authenticate a trading operation payload against its opcode.
pub fn decode_trading_operation(opcode: u8, payload: &[u8]) -> Result<TradingOperation, X3Error> {
    let op: TradingOperation = serde_json::from_slice(payload).map_err(|err| X3Error::CodegenError {
        message: format!("failed to decode trading operation: {err}"),
        span: None,
    })?;
    if trading_opcode(&op) != opcode {
        return Err(X3Error::CodegenError {
            message: format!("trading payload does not match opcode 0x{opcode:02x}"),
            span: None,
        });
    }
    Ok(op)
}

/// Decode every Trading Core instruction from emitted X3 bytecode.
///
/// Non-trading opcodes are skipped using the standard length-prefixed framing.
/// This decoder is intentionally strict for trading payloads: malformed
/// lengths, unknown trading opcodes, or opcode/payload mismatches fail closed.
pub fn decode_trading_program(bytecode: &[u8]) -> Result<Vec<TradingOperation>, X3Error> {
    if bytecode.first().copied() != Some(BYTECODE_VERSION_1) {
        return Err(X3Error::CodegenError {
            message: "unsupported or missing bytecode version".to_string(),
            span: None,
        });
    }

    let mut pos = 1usize;
    let mut operations = Vec::new();

    while pos < bytecode.len() {
        if bytecode[pos] == 0 {
            pos += 1;
            continue;
        }

        let opcode = bytecode[pos];
        pos += 1;

        if matches!(opcode, META_NONCE | META_CHAIN_ID | META_VERSIONS) {
            // The tag has already been consumed, so the record is read from the byte
            // before it: one walker for the whole set, rather than a `match` here that a
            // new record can be left out of.
            let Some((len, _, _)) = crate::spec::opcodes::metadata_record(bytecode, pos - 1) else {
                return Err(X3Error::CodegenError {
                    message: format!("truncated metadata record for opcode 0x{opcode:02x}"),
                    span: None,
                });
            };
            pos = pos - 1 + len;
            continue;
        }

        if pos + 2 > bytecode.len() {
            return Err(X3Error::CodegenError {
                message: format!("truncated instruction header for opcode 0x{opcode:02x}"),
                span: None,
            });
        }
        let len = u16::from_le_bytes([bytecode[pos], bytecode[pos + 1]]) as usize;
        pos += 2;
        if pos + len > bytecode.len() {
            return Err(X3Error::CodegenError {
                message: format!("truncated instruction payload for opcode 0x{opcode:02x}"),
                span: None,
            });
        }
        let payload = &bytecode[pos..pos + len];
        pos += len;

        if matches!(
            opcode,
            TRADING_BEGIN
                | TRADING_OPEN_DEBT
                | TRADING_EXECUTE_SWAP
                | TRADING_CLOSE_DEBT
                | TRADING_ASSERT_MIN_PROFIT
                | TRADING_ASSERT_ALL_DEBTS
                | TRADING_EMIT_RECEIPT
                | TRADING_COMMIT
                | TRADING_ABORT
                | TRADING_ASSERT_INVARIANT
                | TRADING_BRIDGE
        ) {
            operations.push(decode_trading_operation(opcode, payload)?);
        }

        while pos % 4 != 0 && pos < bytecode.len() {
            pos += 1;
        }
    }

    Ok(operations)
}

fn emit_trading_op(op: &TradingOperation, bytecode: &mut Vec<u8>) -> Result<(), X3Error> {
    let opcode = trading_opcode(op);
    let payload = encode_trading_operation(op)?;
    if payload.len() > u16::MAX as usize {
        return Err(X3Error::CodegenError {
            message: "trading operation payload too large".to_string(),
            span: None,
        });
    }
    bytecode.write_all(&[opcode])?;
    bytecode.write_all(&(payload.len() as u16).to_le_bytes())?;
    bytecode.write_all(&payload)?;
    Ok(())
}

fn operation_to_bridge_payload(op: &Operation) -> Result<BridgePayload, X3Error> {
    if let Operation::Bridge {
        via,
        from_chain,
        from_asset,
        to_chain,
        to_asset,
        amount,
        receiver,
        source_finality_proof,
        transfer_proof,
    } = op
    {
        Ok(BridgePayload {
            via: via.clone(),
            from_chain: from_chain.clone(),
            from_asset: from_asset.clone(),
            to_chain: to_chain.clone(),
            to_asset: to_asset.clone(),
            amount: *amount,
            receiver: receiver.clone(),
            source_finality_proof: source_finality_proof.clone(),
            transfer_proof: transfer_proof.clone(),
        })
    } else {
        Err(X3Error::CodegenError {
            message: "operation is not a bridge payload".to_string(),
            span: None,
        })
    }
}

fn operation_to_asset_payload(op: &Operation) -> Result<AssetOpPayload, X3Error> {
    let payload = match op {
        Operation::Lock {
            chain,
            asset,
            amount,
            from,
        } => AssetOpPayload::Lock {
            chain: chain.clone(),
            asset: asset.clone(),
            amount: *amount,
            from: from.clone(),
        },
        Operation::Mint {
            chain,
            asset,
            amount,
            to,
        } => AssetOpPayload::Mint {
            chain: chain.clone(),
            asset: asset.clone(),
            amount: *amount,
            to: to.clone(),
        },
        Operation::Burn {
            chain,
            asset,
            amount,
            from,
        } => AssetOpPayload::Burn {
            chain: chain.clone(),
            asset: asset.clone(),
            amount: *amount,
            from: from.clone(),
        },
        Operation::Release { chain, asset, to } => AssetOpPayload::Release {
            chain: chain.clone(),
            asset: asset.clone(),
            to: to.clone(),
        },
        Operation::Swap {
            from_chain,
            from_asset,
            to_chain,
            to_asset,
            input_amount,
            min_output,
            dex,
        } => AssetOpPayload::Swap {
            from_chain: from_chain.clone(),
            from_asset: from_asset.clone(),
            to_chain: to_chain.clone(),
            to_asset: to_asset.clone(),
            input_amount: *input_amount,
            min_output: *min_output,
            dex: dex.clone(),
        },
        _ => {
            return Err(X3Error::CodegenError {
                message: "operation is not an asset opcode payload".to_string(),
                span: None,
            })
        }
    };
    Ok(payload)
}

fn operation_to_payload(op: &Operation) -> Result<CapabilityPayload, X3Error> {
    let payload = match op {
        Operation::Rebalance {
            name,
            weights,
            criterion,
        } => CapabilityPayload::RebalanceTarget {
            portfolio: name.clone(),
            weights: weights
                .iter()
                .map(|(asset, percent)| (asset.clone(), *percent))
                .collect(),
            criterion: criterion.clone(),
        },
        Operation::VenueOrder {
            action,
            subject,
            asset,
            quantity,
        } => CapabilityPayload::VenueOrder {
            action: action.clone(),
            subject: subject.clone(),
            asset: asset.clone(),
            quantity: *quantity,
        },
        Operation::GpuDispatch { kernel, args, is_simd } => CapabilityPayload::GpuDispatch {
            kernel: kernel.clone(),
            args: args.clone(),
            is_simd: *is_simd,
        },
        Operation::Simulate { body, receipt_slot } => CapabilityPayload::Simulate {
            body_ops: body.len() as u32,
            receipt_slot: receipt_slot.clone(),
        },
        Operation::ScheduledDispatch { period_blocks, entry } => CapabilityPayload::ScheduledDispatch {
            period_blocks: *period_blocks,
            entry_ops: entry.len() as u32,
        },
        Operation::IntentResolve { constraints, resolver } => CapabilityPayload::IntentResolve {
            constraints: constraints.clone(),
            resolver: resolver.clone(),
        },
        Operation::CrdtOp { kind, key, value } => CapabilityPayload::CrdtOp {
            kind: crdt_kind_id(kind),
            key: key.clone(),
            value: value.clone(),
        },
        Operation::ProofVerify {
            kind,
            proof,
            input,
            key_or_threshold,
        } => CapabilityPayload::ProofVerify {
            kind: proof_kind_id(kind),
            proof: proof.clone(),
            input: input.clone(),
            key_or_threshold: key_or_threshold.clone(),
        },
        Operation::StorageOp { kind, data } => CapabilityPayload::StorageOp {
            kind: storage_kind_id(kind),
            data: data.clone(),
        },
        Operation::Pathfind { from, to, max_depth } => CapabilityPayload::Pathfind {
            from: from.clone(),
            to: to.clone(),
            max_depth: *max_depth,
        },
        Operation::MempoolScan { max_results } => CapabilityPayload::MempoolScan {
            max_results: *max_results,
        },
        Operation::OracleRequest { token, reward } => CapabilityPayload::OracleRequest {
            token: token.clone(),
            reward: *reward,
        },
        Operation::EmergencyControl { kind } => CapabilityPayload::EmergencyControl {
            kind: emergency_kind_id(kind),
        },
        Operation::Lifecycle { kind, target } => CapabilityPayload::Lifecycle {
            kind: lifecycle_kind_id(kind),
            target: target.clone(),
        },
        Operation::Serialize { format, data } => CapabilityPayload::Serialize {
            format: serial_format_id(format),
            data: data.clone(),
        },
        Operation::Deserialize { format, data } => CapabilityPayload::Deserialize {
            format: serial_format_id(format),
            data: data.clone(),
        },
        Operation::GasEstimate { chain, route } => CapabilityPayload::GasEstimate {
            chain: chain.clone(),
            route: route.clone(),
        },
        Operation::ChainMetric { metric } => CapabilityPayload::ChainMetric {
            metric: chain_metric_id(metric),
        },
        Operation::EventProvenance { event_type, data } => CapabilityPayload::EventProvenance {
            event_type: event_type.clone(),
            data: data.clone(),
        },
        Operation::MultiHopSwap { path, amount } => CapabilityPayload::MultiHopSwap {
            path: path.clone(),
            amount: *amount,
        },
        Operation::VectorMath { op, a, b, size } => CapabilityPayload::VectorMath {
            op: vector_op_id(op),
            a: a.clone(),
            b: b.clone(),
            size: *size,
        },
        Operation::RoleCheck { role } => CapabilityPayload::RoleCheck { role: role.clone() },
        Operation::MultisigCheck { required, total } => CapabilityPayload::MultisigCheck {
            required: *required,
            total: *total,
        },
        Operation::VersionMeta { version, upgrade_from } => CapabilityPayload::VersionMeta {
            version: version.clone(),
            upgrade_from: upgrade_from.clone(),
        },
        Operation::StorageNamespace { package, key } => CapabilityPayload::StorageNamespace {
            package: package.clone(),
            key: key.clone(),
        },
        Operation::AbiExport { function, params, ret } => CapabilityPayload::AbiExport {
            function: function.clone(),
            params: params.clone(),
            ret: ret.clone(),
        },
        Operation::DocEmbed { content } => CapabilityPayload::DocEmbed {
            content: content.clone(),
        },
        Operation::GasAdaptive {
            high_gas_ops,
            low_gas_ops,
        } => CapabilityPayload::GasAdaptive {
            high_gas_ops: high_gas_ops.len() as u32,
            low_gas_ops: low_gas_ops.len() as u32,
        },
        Operation::Bounty { amount, condition } => CapabilityPayload::Bounty {
            amount: *amount,
            condition: condition.clone(),
        },
        // B-52 feature lock payloads
        Operation::RouteScore { strategy, weights } => CapabilityPayload::RouteScore {
            strategy: strategy.clone(),
            weights: weights.iter().map(|(k, v)| (k.clone(), *v)).collect(),
        },
        Operation::SolverBid {
            solver,
            receive_asset,
            deliver_asset,
            fee,
            bond,
        } => CapabilityPayload::SolverBid {
            solver: solver.clone(),
            receive_asset: receive_asset.clone(),
            deliver_asset: deliver_asset.clone(),
            fee: fee.clone(),
            bond: *bond,
        },
        Operation::RelayerAttest {
            relayers,
            quorum,
            signatures,
        } => CapabilityPayload::RelayerAttest {
            relayers: relayers.clone(),
            quorum_numerator: quorum.0,
            quorum_denominator: quorum.1,
            signatures: signatures.clone(),
        },
        Operation::RpcConsensus {
            chain,
            require,
            reject_on,
        } => CapabilityPayload::RpcConsensus {
            chain: chain.clone(),
            require_numerator: require.0,
            require_denominator: require.1,
            reject_on: reject_on.clone(),
        },
        Operation::NonceUnused { nonce } => CapabilityPayload::NonceUnused { nonce: nonce.clone() },
        Operation::RiskScore { score, category } => CapabilityPayload::RiskScore {
            score: *score,
            category: category.clone(),
        },
        Operation::InvariantCheck { name, assert_expr } => CapabilityPayload::InvariantCheck {
            name: name.clone(),
            assert_expr: assert_expr.clone(),
        },
        Operation::PrivacyCommit { reveal_on, encrypted } => CapabilityPayload::PrivacyCommit {
            reveal_on: reveal_on.clone(),
            encrypted: *encrypted,
        },
        Operation::ProofRequired { proof_type, source } => CapabilityPayload::ProofRequired {
            proof_type: proof_type.clone(),
            source: source.clone(),
        },
        Operation::VmAdapterCall { vm, adapter, calldata } => CapabilityPayload::VmAdapterCall {
            vm: vm.clone(),
            adapter: adapter.clone(),
            calldata: calldata.clone(),
        },
        Operation::ModeCheck { mode, restriction } => CapabilityPayload::ModeCheck {
            mode: mode.clone(),
            restriction: restriction.clone(),
        },
        Operation::PackageImport { path, alias } => CapabilityPayload::PackageImport {
            path: path.clone(),
            alias: alias.clone(),
        },
        Operation::RefundPolicy {
            action,
            target,
            after_blocks,
        } => CapabilityPayload::RefundPolicy {
            action: action.clone(),
            target: target.clone(),
            after_blocks: *after_blocks,
        },
        _ => {
            return Err(X3Error::CodegenError {
                message: "operation is not a capability payload".to_string(),
                span: None,
            })
        }
    };
    Ok(payload)
}

fn crdt_kind_id(kind: &CrdtKind) -> u8 {
    match kind {
        CrdtKind::Get => 0,
        CrdtKind::Set => 1,
        CrdtKind::Append => 2,
        CrdtKind::Merge => 3,
    }
}

fn proof_kind_id(kind: &ProofKind) -> u8 {
    match kind {
        ProofKind::Zk => 0,
        ProofKind::Mpc => 1,
    }
}

fn storage_kind_id(kind: &StorageKind) -> u8 {
    match kind {
        StorageKind::Store => 0,
        StorageKind::Load => 1,
    }
}

fn emergency_kind_id(kind: &EmergencyKind) -> u8 {
    match kind {
        EmergencyKind::Pause => 0,
        EmergencyKind::Resume => 1,
    }
}

fn lifecycle_kind_id(kind: &LifecycleKind) -> u8 {
    match kind {
        LifecycleKind::Destroy => 0,
        LifecycleKind::Migrate => 1,
    }
}

fn serial_format_id(format: &SerialFormat) -> u8 {
    match format {
        SerialFormat::Rlp => 0,
        SerialFormat::Cbor => 1,
        SerialFormat::Json => 2,
        SerialFormat::Ssz => 3,
    }
}

fn chain_metric_id(metric: &ChainMetricKind) -> u8 {
    match metric {
        ChainMetricKind::Snapshot => 0,
        ChainMetricKind::Congestion => 1,
        ChainMetricKind::BaseFee => 2,
        ChainMetricKind::FinalityLag => 3,
        ChainMetricKind::BlockTime => 4,
    }
}

fn vector_op_id(op: &VectorOp) -> u8 {
    match op {
        VectorOp::Add => 0,
        VectorOp::DotProduct => 1,
        VectorOp::Mul => 2,
        VectorOp::Sub => 3,
    }
}

/// One instruction a compiler stream contains, as a reader sees it.
///
/// The framing rules — which opcodes carry a payload, how wide a fixed frame is,
/// where the padding sits — come from `spec/opcodes.rs`. Two readers that walk the
/// same bytes have to agree, so `disassemble` and the cost estimate (PHASE 35) both
/// walk through here rather than each stepping the cursor itself.
pub struct StreamInstruction<'a> {
    pub pc: usize,
    pub opcode: u8,
    pub flags: u8,
    pub operand: u16,
    pub payload: &'a [u8],
    /// Where the next instruction starts.
    pub next_pc: usize,
}

/// Walk a compiler stream into its instructions.
pub fn instructions(bytecode: &[u8]) -> Result<Vec<StreamInstruction<'_>>, X3Error> {
    if bytecode.is_empty() {
        return Err(X3Error::CodegenError {
            message: "bytecode is empty".to_string(),
            span: None,
        });
    }
    let mut found = Vec::new();
    let mut pc = first_instruction_offset(bytecode);
    while pc + 4 <= bytecode.len() {
        if bytecode[pc..pc + 4].iter().all(|byte| *byte == 0) {
            pc += 4;
            continue;
        }
        let opcode = bytecode[pc];
        let payload_len = u16::from_le_bytes([bytecode[pc + 1], bytecode[pc + 2]]) as usize;
        let payload_end = align4(pc + 3 + payload_len);
        let safe_end = payload_end.min(bytecode.len());
        let payload = &bytecode[pc + 3..safe_end.min(pc + 3 + payload_len)];
        let next_pc = if is_payload_opcode(opcode, true) {
            payload_end
        } else {
            align4(pc + fixed_frame_content_len(opcode))
        };
        found.push(StreamInstruction {
            pc,
            opcode,
            flags: bytecode[pc + 1],
            operand: u16::from_le_bytes([bytecode[pc + 2], bytecode[pc + 3]]),
            payload,
            next_pc,
        });
        pc = next_pc;
    }
    Ok(found)
}

/// A metadata record at the head of a compiler stream.
pub struct MetadataRecord<'a> {
    pub label: &'static str,
    /// A rendered value: the nonce as a string, the chain id as digits.
    pub rendered: String,
    pub next_pc: usize,
    pub _marker: core::marker::PhantomData<&'a ()>,
}

/// The metadata records at the head of a compiler stream, in order.
///
/// One matcher for the block, because the writer leaves it unpadded and a second
/// reader that aligned it started one byte late (see the note on `disassemble`).
pub fn metadata_records(bytecode: &[u8]) -> Vec<MetadataRecord<'_>> {
    let mut records = Vec::new();
    let mut pc = 1usize;
    loop {
        if pc + 3 > bytecode.len() {
            return records;
        }
        match bytecode[pc] {
            META_NONCE => {
                // One walker for the whole set: a record added to `spec::opcodes` cannot be
                // read as instructions here.
                let Some((len, label, rendered)) = crate::spec::opcodes::metadata_record(bytecode, pc) else {
                    return records;
                };
                let next_pc = pc + len;
                records.push(MetadataRecord {
                    label,
                    rendered,
                    next_pc,
                    _marker: core::marker::PhantomData,
                });
                pc = next_pc;
            }
            META_CHAIN_ID => {
                if pc + 9 > bytecode.len() {
                    return records;
                }
                let id = u64::from_le_bytes([
                    bytecode[pc + 1],
                    bytecode[pc + 2],
                    bytecode[pc + 3],
                    bytecode[pc + 4],
                    bytecode[pc + 5],
                    bytecode[pc + 6],
                    bytecode[pc + 7],
                    bytecode[pc + 8],
                ]);
                let next_pc = pc + 9;
                records.push(MetadataRecord {
                    label: "meta.chain_id",
                    rendered: id.to_string(),
                    next_pc,
                    _marker: core::marker::PhantomData,
                });
                pc = next_pc;
            }
            META_VERSIONS => {
                // The version binding (PHASE 45). Fixed-width, so this walker needs only
                // its length — but it *is* a third place that names the metadata set, and
                // an unknown tag ends the walk, so a reader that did not learn it would
                // take the record's bytes for instructions.
                if pc + VERSIONS_RECORD_LEN > bytecode.len() {
                    return records;
                }
                let read = |offset: usize| -> u16 {
                    let at = pc + 1 + offset * 2;
                    u16::from_le_bytes([bytecode[at], bytecode[at + 1]])
                };
                let next_pc = pc + VERSIONS_RECORD_LEN;
                records.push(MetadataRecord {
                    label: "meta.versions",
                    rendered: format!(
                        "language {} compiler {} IR {} VM {} policy {}",
                        read(0),
                        read(1),
                        read(2),
                        read(3),
                        read(4)
                    ),
                    next_pc,
                    _marker: core::marker::PhantomData,
                });
                pc = next_pc;
            }
            _ => return records,
        }
    }
}

/// The offset of the instruction stream: past the version byte and the metadata
/// block, which the writer leaves unpadded.
pub fn first_instruction_offset(bytecode: &[u8]) -> usize {
    metadata_records(bytecode)
        .last()
        .map(|record| record.next_pc)
        .unwrap_or(1)
}

/// Disassemble X3 bytecode into a human-readable trace.
///
/// This is the "explain" subcommand of the x3c CLI: a reviewer can run
/// `x3c explain program.x3b` and see per-instruction pseudo-code without
/// reading raw bytes.
pub fn disassemble(bytecode: &[u8]) -> Result<String, X3Error> {
    let mut out = String::new();
    out.push_str(&format!("; x3-lang bytecode v0x{:02x}\n", bytecode[0]));
    let mut idx = 0u32;

    // The metadata block is written by `emit_x3ir` with no padding, so the reader
    // mirrors the writer: aligning the cursor here started the walk one byte late
    // and turned payload bytes into opcodes, which is how `x3c explain` printed
    // garbage for any program with a nonce. The VM's `first_instruction_pc` *does*
    // align, so the writer and the VM disagree about this block; that is a
    // separate, coordinated format fix (TICKET-023), not something to paper over
    // here. The matcher is `metadata_records`, shared with `first_instruction_offset`.
    for record in metadata_records(bytecode) {
        out.push_str(&format!("  {idx:04}  {} = {}\n", record.label, record.rendered));
        idx += 1;
    }

    // One walk, shared with the cost estimate: which opcodes carry a payload, how
    // wide a fixed frame is and where the padding sits all come from
    // `spec/opcodes.rs`.
    for instruction in instructions(bytecode)? {
        let entry = disassemble_op(
            instruction.opcode,
            instruction.payload,
            instruction.flags,
            instruction.operand,
        );
        out.push_str(&format!("  {idx:04}  0x{:02x}  {entry}\n", instruction.opcode));
        idx += 1;
    }
    Ok(out)
}

// The payload/fixed-frame classification is not defined here. It lives once, in
// `spec/opcodes.rs`, because this crate and the VM both walk the same bytes: the
// two copies had drifted, and this one listed `0x66` — which no emitter arm
// produces — while omitting `CALL_HOST` (`0x61`), which does carry a payload.

fn align4(value: usize) -> usize {
    (value + 3) & !3
}

fn disassemble_op(opcode: u8, payload: &[u8], flags: u8, operand: u16) -> String {
    // The name comes from the one table both crates include, and whether the
    // instruction carries a payload comes from the one predicate that says so.
    // This function used to hold its own name table *and* its own idea of which
    // opcodes carry payloads — including a `VECTOR` arm for `0x70..=0x7F`, a range
    // no instruction has ever been emitted in.
    let name = opcode_name(opcode);
    // `REQUIRE` is the one fixed frame whose operand is read: a finality policy
    // declaration carries the depth it requires there, so printing it is what
    // makes the number visible to whoever reads the artifact — a guard's own
    // threshold is not carried (`REQUIRE static 0`), and a run-time comparison
    // says which mode it is in (`REQUIRE ge 1`, the nonce guard).
    if opcode == REQUIRE {
        let mode = match flags & REQUIRE_COMPARE_MASK {
            REQUIRE_COMPARE_STATIC => "static",
            REQUIRE_COMPARE_GE => "ge",
            _ => "?",
        };
        return format!("{name} {mode} {operand}");
    }
    if !is_payload_opcode(opcode, true) {
        return name.to_string();
    }
    let payload_str = match decode_payload(opcode, payload) {
        Ok(s) => s,
        Err(_) => format!("<raw {} bytes>", payload.len()),
    };
    format!("{name:<24} {payload_str}")
}

fn decode_payload(opcode: u8, payload: &[u8]) -> Result<String, X3Error> {
    use x3_lang_common::{decode_asset_op_payload, decode_bridge_payload, decode_capability_payload};
    if (TRADING_BEGIN..=TRADING_BRIDGE).contains(&opcode) {
        let op = decode_trading_operation(opcode, payload)?;
        return Ok(format!("{op:?}"));
    }
    if matches!(opcode, 0x20..=0x24) {
        let p = decode_asset_op_payload(opcode, payload).map_err(|_| X3Error::CodegenError {
            message: "bad asset payload".into(),
            span: None,
        })?;
        return Ok(format!("{p:?}"));
    }
    if opcode == 0x25 {
        let p = decode_bridge_payload(payload).map_err(|_| X3Error::CodegenError {
            message: "bad bridge payload".into(),
            span: None,
        })?;
        return Ok(format!("{p:?}"));
    }
    if (0x80..=0x9C).contains(&opcode) || (0xA0..=0xAB).contains(&opcode) {
        let p = decode_capability_payload(opcode, payload).map_err(|_| X3Error::CodegenError {
            message: "bad capability payload".into(),
            span: None,
        })?;
        return Ok(format!("{p:?}"));
    }
    Ok(String::from_utf8_lossy(payload).to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ir::{
        ChainMetricKind, CrdtKind, EmergencyKind, LifecycleKind, ProofKind, SerialFormat, StorageKind, VectorOp,
    };
    use x3_lang_common::{decode_capability_payload, CapabilityPayload};

    /// The number of instructions a trace lists.
    ///
    /// The entry lines are `  {index}  0x{opcode}  {detail}` and the metadata
    /// lines are `  {index}  meta.{record} = …`, so the second column is what
    /// separates them — counting lines that contain `0x` anywhere counts the
    /// version banner too, which is how the first version of this assertion was
    /// off by one in the direction that looks like a passing test.
    fn instruction_lines(trace: &str) -> usize {
        trace
            .lines()
            .filter(|line| {
                line.split_whitespace()
                    .nth(1)
                    .is_some_and(|column| column.starts_with("0x"))
            })
            .count()
    }

    #[test]
    fn emits_all_capability_opcodes_0x80_through_0x9a() {
        let mut ir = X3IR::new();
        ir.operations = vec![
            Operation::GpuDispatch {
                kernel: "k".into(),
                args: vec!["a".into()],
                is_simd: true,
            },
            Operation::Simulate {
                body: vec![Operation::Nop],
                receipt_slot: "receipt".into(),
            },
            Operation::ScheduledDispatch {
                period_blocks: 1,
                entry: vec![Operation::Nop],
            },
            Operation::IntentResolve {
                constraints: vec!["c".into()],
                resolver: "r".into(),
            },
            Operation::CrdtOp {
                kind: CrdtKind::Set,
                key: "k".into(),
                value: Some("v".into()),
            },
            Operation::ProofVerify {
                kind: ProofKind::Zk,
                proof: "p".into(),
                input: "i".into(),
                key_or_threshold: "vk".into(),
            },
            Operation::StorageOp {
                kind: StorageKind::Store,
                data: "d".into(),
            },
            Operation::Pathfind {
                from: "a".into(),
                to: "b".into(),
                max_depth: 2,
            },
            Operation::MempoolScan { max_results: 3 },
            Operation::OracleRequest {
                token: "X3".into(),
                reward: 4,
            },
            Operation::EmergencyControl {
                kind: EmergencyKind::Pause,
            },
            Operation::Lifecycle {
                kind: LifecycleKind::Migrate,
                target: Some("next".into()),
            },
            Operation::Serialize {
                format: SerialFormat::Rlp,
                data: "d".into(),
            },
            Operation::Deserialize {
                format: SerialFormat::Cbor,
                data: "d".into(),
            },
            Operation::GasEstimate {
                chain: "evm".into(),
                route: "r".into(),
            },
            Operation::ChainMetric {
                metric: ChainMetricKind::BaseFee,
            },
            Operation::EventProvenance {
                event_type: "e".into(),
                data: "d".into(),
            },
            Operation::MultiHopSwap {
                path: vec!["a".into(), "b".into()],
                amount: 5,
            },
            Operation::VectorMath {
                op: VectorOp::DotProduct,
                a: "a".into(),
                b: "b".into(),
                size: 2,
            },
            Operation::RoleCheck { role: "admin".into() },
            Operation::MultisigCheck { required: 2, total: 3 },
            Operation::VersionMeta {
                version: "1.0.0".into(),
                upgrade_from: Some("0.9.0".into()),
            },
            Operation::StorageNamespace {
                package: "pkg".into(),
                key: "k".into(),
            },
            Operation::AbiExport {
                function: "f".into(),
                params: vec!["u64".into()],
                ret: "()".into(),
            },
            Operation::DocEmbed { content: "docs".into() },
            Operation::GasAdaptive {
                high_gas_ops: vec![Operation::Nop],
                low_gas_ops: vec![Operation::Nop],
            },
            Operation::Bounty {
                amount: 6,
                condition: "done".into(),
            },
        ];

        let bytecode = emit_x3ir(&ir).expect("capability operations should emit");
        // Past the version byte and the version-binding record the emitter always writes
        // (PHASE 45), which is where the first instruction starts.
        let mut cursor = 1 + VERSIONS_RECORD_LEN;
        for expected in 0x80u8..=0x9A {
            assert_eq!(bytecode[cursor], expected);
            let len = u16::from_le_bytes([bytecode[cursor + 1], bytecode[cursor + 2]]) as usize;
            let payload = decode_capability_payload(expected, &bytecode[cursor + 3..cursor + 3 + len])
                .expect("emitted capability payload should decode");
            if expected == 0x94 {
                assert_eq!(payload, CapabilityPayload::MultisigCheck { required: 2, total: 3 });
            }
            cursor += 3 + len;
            while cursor % 4 != 0 {
                cursor += 1;
            }
        }
    }

    #[test]
    fn nonce_metadata_does_not_shift_the_instruction_stream() {
        // Regression for `x3c explain`: the reader aligned the metadata block
        // while the writer does not, so the walk started one byte late and read
        // payload bytes as opcodes. A 15-byte nonce makes the difference
        // visible, since `[version][META_NONCE][u16 len][nonce]` is then 19
        // bytes rather than a multiple of four.
        let mut ir = X3IR::new();
        ir.metadata.nonce = Some("simple_swap_001".to_string());
        // A non-zero opcode: `NOP` encodes as four zero bytes, which the
        // disassembler treats as padding and skips.
        ir.operations = vec![Operation::AtomicBegin];

        let bytecode = emit_x3ir(&ir).expect("should emit");
        assert_eq!(bytecode.len() % 4, 0, "the stream must be 4-byte aligned");

        let trace = disassemble(&bytecode).expect("should disassemble");
        assert!(trace.contains("meta.nonce"), "the nonce must be reported: {trace}");
        assert!(
            trace.contains("ATOMIC_BEGIN"),
            "the instruction after the metadata must decode: {trace}"
        );
    }

    #[test]
    fn chain_id_metadata_is_read_at_its_real_width() {
        // The disassembler read `chain_id` as a u32 and advanced five bytes,
        // while the writer emits a u64. Any stream carrying a chain id desynced
        // immediately after it.
        let mut ir = X3IR::new();
        ir.metadata.nonce = Some("nonce_1".to_string());
        ir.metadata.chain_id = Some(0x0123_4567_89AB_CDEF);
        ir.operations = vec![Operation::AtomicBegin];

        let bytecode = emit_x3ir(&ir).expect("should emit");
        let trace = disassemble(&bytecode).expect("should disassemble");
        assert!(
            trace.contains("81985529216486895"),
            "chain_id must be reported as the u64 that was written: {trace}"
        );
        assert!(
            trace.contains("ATOMIC_BEGIN"),
            "the instruction after the chain id must decode: {trace}"
        );
    }

    #[test]
    fn a_fixed_frame_operator_with_a_non_zero_operand_does_not_truncate_the_walk() {
        // `REQUIRE` is a fixed four-byte frame whose second and third bytes are
        // an operand, not a payload length. Listing it as a payload opcode made
        // the walker read `[comparison][threshold_lo]` as a length: a guard with
        // comparison 1 and threshold 4 (`0x0401`) jumped a kilobyte past the end
        // of the stream and everything after it vanished from the listing.
        //
        // The stream below is the one the emitter writes for a guard at the head
        // of a program: the guard's four bytes at offset 1 — over the version
        // byte — then the padding that puts the next instruction on the next
        // absolute multiple of four, then `HALT`. Because the guard's content is
        // four bytes rather than three, the instruction after it starts at 8, so
        // a reader that assumed three bytes landed on the padding instead.
        let mut bytes = vec![BYTECODE_VERSION_1];
        bytes.extend_from_slice(&[REQUIRE, 0x01, 0x04, 0x00]); // comparison GE, threshold 4
        bytes.extend_from_slice(&[0x00, 0x00, 0x00]); // the padding `pad_to_4` writes
        bytes.extend_from_slice(&[HALT, 0x00, 0x00, 0x00]);

        let trace = disassemble(&bytes).expect("should disassemble");
        assert!(trace.contains("REQUIRE"), "the guard must be listed: {trace}");
        assert!(
            trace.contains("HALT"),
            "the instruction after it must be listed: {trace}"
        );
    }

    #[test]
    fn a_payload_first_program_is_walked_at_the_writers_boundaries() {
        // The same property as the guard-first test above, for the other shape a
        // program can start with, and asserted against the emitter rather than
        // against a hand-built stream: every operation the lowering emitted must
        // appear once, and nothing else may appear.
        //
        // A payload frame at offset 1 is `3 + len` bytes and the reader advances
        // by `align4(pc + 3 + len)`, whatever `pc` is, so this shape was already
        // walked correctly — it is here to keep the two shapes measured
        // together, because the defect was a reader that treated "offset 1" as
        // if it were "offset 0".
        let source = "intent payload_first {\n    from ethereum.USDC amount 1\n    to \
                      solana.SOL\n    route {\n        swap uniswap ethereum.USDC -> solana.SOL \
                      amount 1 min_output 1\n    }\n}\n";
        let program = crate::parser::parse_source(source).expect("source should parse");
        let ir = crate::compile_to_ir(&program).expect("source should lower");
        let bytecode = emit_x3ir(&ir).expect("should emit");

        let trace = disassemble(&bytecode).expect("should disassemble");
        assert!(
            !trace.contains("UNKNOWN"),
            "no byte of the stream may be presented as an instruction the language does not have: {trace}"
        );
        assert_eq!(
            instruction_lines(&trace),
            ir.operations.iter().filter(|op| !matches!(op, Operation::Nop)).count(),
            "the walk must visit exactly the instructions the writer wrote: {trace}"
        );
    }

    #[test]
    fn a_guard_first_program_is_walked_at_the_writers_boundaries() {
        // `risk_policy { max_slippage 120 }` lowers the guard to the program's
        // first instruction, so the stream is `[version][REQUIRE][flags][threshold
        // u16]` followed by padding. The reader advanced four bytes from offset 1
        // — onto the guard's own padding — and printed the padding and the next
        // operation's payload as opcodes: eighteen lines of `UNKNOWN` for the ten
        // instructions below. The property is agreement, so it is asserted as a
        // count of listed instructions against the IR, and as the absence of any
        // name the language does not define.
        let source = "risk_policy {\n    max_slippage 120\n}\n\nintent guard_first {\n    from \
                      ethereum.USDC amount 1\n    to solana.SOL\n    route {\n        swap uniswap \
                      ethereum.USDC -> solana.SOL amount 1 min_output 1\n    }\n    require \
                      slippage <= 120\n    on_fail refund ethereum.USDC to sender\n}\n";
        let program = crate::parser::parse_source(source).expect("source should parse");
        let ir = crate::compile_to_ir(&program).expect("source should lower");
        let bytecode = emit_x3ir(&ir).expect("should emit");

        // The version binding sits between the version byte and the first instruction
        // (PHASE 45), so the guard is the first *instruction* rather than the second byte.
        assert_eq!(
            bytecode[1 + VERSIONS_RECORD_LEN],
            REQUIRE,
            "the fixture is only a regression test if the guard really is the first instruction"
        );
        let trace = disassemble(&bytecode).expect("should disassemble");
        assert!(
            !trace.contains("UNKNOWN"),
            "no byte of the stream may be presented as an instruction the language does not have: {trace}"
        );
        assert_eq!(
            instruction_lines(&trace),
            ir.operations.iter().filter(|op| !matches!(op, Operation::Nop)).count(),
            "the walk must visit exactly the instructions the writer wrote: {trace}"
        );
    }

    #[test]
    fn payload_carrying_instructions_are_walked_by_their_length() {
        // `EMIT` and `CALL_HOST` carry a length-prefixed payload, and the two
        // readers used to disagree about them: this crate's list named `0x66`,
        // which no emitter arm produces, instead of `CALL_HOST` (`0x61`), and the
        // VM's list named neither. A reader that calls them fixed-width walks
        // four bytes into the payload and then reads payload text as opcodes.
        for opcode in [EMIT, CALL_HOST] {
            let mut bytes = vec![BYTECODE_VERSION_1];
            bytes.extend_from_slice(&[opcode, 5, 0]); // five-byte payload
            bytes.extend_from_slice(b"hello");
            while bytes.len() % 4 != 0 {
                bytes.push(0);
            }
            bytes.extend_from_slice(&[HALT, 0, 0, 0]);

            let trace = disassemble(&bytes).expect("should disassemble");
            assert!(
                trace.contains("HALT"),
                "opcode 0x{opcode:02x} must be walked by its payload length, not as a fixed frame: {trace}"
            );
        }
    }
}
