//! X3IR -> Bytecode/Runtime Dispatch emitter.
//!
//! This module converts X3IR operations into executable bytecode suitable
//! for the X3 runtime or specific chain emitters (EVM, SVM, etc.).

use crate::ir::{
    ChainMetricKind, CrdtKind, EmergencyKind, LifecycleKind, Operation, ProofKind, SerialFormat, StorageKind, VectorOp,
    X3IR,
};
// Import shared opcode constants
use crate::spec::opcodes::*;
use std::io::Write;
use x3_lang_common::{
    encode_asset_op_payload, encode_bridge_payload, encode_capability_payload, AssetOpPayload, BridgePayload,
    CapabilityPayload, X3Error,
};
/// Pad `bytecode` so its length is a multiple of 4. The X3 VM verifier
/// requires every instruction to start on a 4-byte boundary.
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
        Operation::AtomicEnd => {
            bytecode.write_all(&[ATOMIC_END])?;
            bytecode.write_all(&0u16.to_le_bytes())?;
        }
        Operation::If {
            condition,
            then_ops,
            else_ops,
        } => {
            bytecode.write_all(&[IF])?;
            let cond_str = format!("{:?}", condition);
            bytecode.write_all(&(cond_str.len() as u16).to_le_bytes())?;
            bytecode.write_all(cond_str.as_bytes())?;

            // Then branch length
            let mut then_bytecode = Vec::new();
            for op in then_ops {
                emit_operation(op, &mut then_bytecode)?;
            }
            bytecode.write_all(&(then_bytecode.len() as u32).to_le_bytes())?;
            bytecode.write_all(&then_bytecode)?;

            // Else branch (if exists)
            if let Some(else_blk) = else_ops {
                let mut else_bytecode = Vec::new();
                for op in else_blk {
                    emit_operation(op, &mut else_bytecode)?;
                }
                bytecode.write_all(&(else_bytecode.len() as u32).to_le_bytes())?;
                bytecode.write_all(&else_bytecode)?;
            } else {
                bytecode.write_all(&0u32.to_le_bytes())?;
            }
        }
        Operation::Loop { max_iterations, body } => {
            bytecode.write_all(&[LOOP])?;
            bytecode.write_all(&max_iterations.to_le_bytes())?;

            // Loop body
            let mut body_bytecode = Vec::new();
            for op in body {
                emit_operation(op, &mut body_bytecode)?;
            }
            bytecode.write_all(&(body_bytecode.len() as u32).to_le_bytes())?;
            bytecode.write_all(&body_bytecode)?;
        }
        Operation::Require { .. } => {
            bytecode.write_all(&[REQUIRE])?;
            bytecode.write_all(&0u16.to_le_bytes())?;
        }
        Operation::OnFail { .. } => {
            bytecode.write_all(&[ON_FAIL])?;
            bytecode.write_all(&0u16.to_le_bytes())?;
        }
        Operation::OnTimeout { .. } => {
            bytecode.write_all(&[ON_TIMEOUT])?;
            bytecode.write_all(&0u16.to_le_bytes())?;
        }
        Operation::Emit { name, data } => {
            bytecode.write_all(&[EMIT])?;
            let payload = format!("{}:{:?}", name, data);
            bytecode.write_all(&(payload.len() as u16).to_le_bytes())?;
            bytecode.write_all(payload.as_bytes())?;
        }
        Operation::Call { function, args } => {
            bytecode.write_all(&[CALL_HOST])?;
            let payload = format!("{}:{:?}", function, args);
            bytecode.write_all(&(payload.len() as u16).to_le_bytes())?;
            bytecode.write_all(payload.as_bytes())?;
        }
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
        Operation::VmAdapterCall { .. } => emit_payload_op(VM_ADAPTER_CALL, op, bytecode)?,
        Operation::ModeCheck { .. } => emit_payload_op(MODE_CHECK, op, bytecode)?,
        Operation::PackageImport { .. } => emit_payload_op(PACKAGE_IMPORT, op, bytecode)?,
        Operation::RefundPolicy { .. } => emit_payload_op(REFUND_POLICY, op, bytecode)?,
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
            to_asset,
            input_amount,
            min_output,
            dex,
        } => AssetOpPayload::Swap {
            from_chain: from_chain.clone(),
            from_asset: from_asset.clone(),
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

/// Disassemble X3 bytecode into a human-readable trace.
///
/// This is the "explain" subcommand of the x3c CLI: a reviewer can run
/// `x3c explain program.x3b` and see per-instruction pseudo-code without
/// reading raw bytes.
pub fn disassemble(bytecode: &[u8]) -> Result<String, X3Error> {
    if bytecode.is_empty() {
        return Err(X3Error::CodegenError {
            message: "bytecode is empty".to_string(),
            span: None,
        });
    }
    let mut out = String::new();
    out.push_str(&format!("; x3-lang bytecode v0x{:02x}\n", bytecode[0]));
    let mut pc = 1usize;
    let mut idx = 0u32;
    // Walk optional metadata: any leading 0x10/0x11 byte is metadata; the
    // first non-metadata byte is the start of the operation stream.
    loop {
        if pc >= bytecode.len() {
            break;
        }
        match bytecode[pc] {
            0x10 => {
                let len = u16::from_le_bytes([bytecode[pc + 1], bytecode[pc + 2]]) as usize;
                let value = std::str::from_utf8(&bytecode[pc + 3..pc + 3 + len]).unwrap_or("<invalid utf-8>");
                out.push_str(&format!("  {idx:04}  meta.nonce   = {value:?}\n"));
                idx += 1;
                pc = align4(pc + 3 + len);
            }
            0x11 => {
                let id = u32::from_le_bytes([bytecode[pc + 1], bytecode[pc + 2], bytecode[pc + 3], bytecode[pc + 4]]);
                out.push_str(&format!("  {idx:04}  meta.chain_id = {id}\n"));
                idx += 1;
                pc += 5;
            }
            _ => break,
        }
    }
    while pc + 4 <= bytecode.len() {
        if bytecode[pc..pc + 4].iter().all(|b| *b == 0) {
            pc += 4;
            continue;
        }
        let opcode = bytecode[pc];
        let payload_len = u16::from_le_bytes([bytecode[pc + 1], bytecode[pc + 2]]) as usize;
        let payload_end = align4(pc + 3 + payload_len);
        let safe_end = payload_end.min(bytecode.len());
        let payload = &bytecode[pc + 3..safe_end.min(pc + 3 + payload_len)];
        let entry = disassemble_op(opcode, payload);
        out.push_str(&format!("  {idx:04}  0x{opcode:02x}  {entry}\n"));
        idx += 1;
        if is_payload_opcode(opcode) {
            pc = payload_end;
        } else {
            pc += 4;
        }
    }
    Ok(out)
}

fn is_payload_opcode(opcode: u8) -> bool {
    matches!(
        opcode,
        0x20..=0x25
            | 0x40
            | 0x41
            | 0x42
            | 0x50
            | 0x51
            | 0x52
            | 0x60
            | 0x66
            | 0x70..=0x7F
            | 0x80..=0x9B
            | 0xA0..=0xAB
    )
}

fn align4(value: usize) -> usize {
    (value + 3) & !3
}

fn disassemble_op(opcode: u8, payload: &[u8]) -> String {
    let payload_str = match decode_payload(opcode, payload) {
        Ok(s) => s,
        Err(_) => format!("<raw {} bytes>", payload.len()),
    };
    match opcode {
        0x01 => "ADD".into(),
        0x02 => "SUB".into(),
        0x10 => "META_NONCE".into(),
        0x11 => "META_CHAIN_ID".into(),
        0x20 => format!("LOCK     {payload_str}"),
        0x21 => format!("MINT     {payload_str}"),
        0x22 => format!("BURN     {payload_str}"),
        0x23 => format!("RELEASE  {payload_str}"),
        0x24 => format!("SWAP     {payload_str}"),
        0x25 => format!("BRIDGE   {payload_str}"),
        0x30 => "IF".into(),
        0x31 => "LOOP".into(),
        0x32 => "CALL".into(),
        0x33 => "RET".into(),
        0x40 => "REQUIRE".into(),
        0x41 => "ON_FAIL".into(),
        0x42 => "ON_TIMEOUT".into(),
        0x50 => "ATOMIC_BEGIN".into(),
        0x51 => "ATOMIC_END".into(),
        0x52 => "ATOMIC_ROLLBACK".into(),
        0x60 => format!("EMIT     {payload_str}"),
        0x66 => format!("CALL_HOST  {payload_str}"),
        0x70..=0x7F => format!("VECTOR   {payload_str}"),
        0x80..=0x9A => format!("CAP      {payload_str}"),
        0x9B => "SUB_EXEC".into(),
        0xA0 => format!("ROUTE_SCORE  {payload_str}"),
        0xA1 => format!("SOLVER_BID   {payload_str}"),
        0xA2 => format!("RELAYER_ATTEST {payload_str}"),
        0xA3 => format!("RPC_CONSENSUS {payload_str}"),
        0xA4 => format!("RISK_SCORE   {payload_str}"),
        0xA5 => format!("INVARIANT_CHECK {payload_str}"),
        0xA6 => format!("PRIVACY_COMMIT {payload_str}"),
        0xA7 => format!("PROOF_REQUIRED {payload_str}"),
        0xA8 => format!("VM_ADAPTER_CALL {payload_str}"),
        0xA9 => format!("MODE_CHECK   {payload_str}"),
        0xAA => format!("PACKAGE_IMPORT {payload_str}"),
        0xAB => format!("REFUND_POLICY {payload_str}"),
        0xFF => "HALT".into(),
        other => format!("OP(0x{other:02x})"),
    }
}

fn decode_payload(opcode: u8, payload: &[u8]) -> Result<String, X3Error> {
    use x3_lang_common::{decode_asset_op_payload, decode_bridge_payload, decode_capability_payload};
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
    if (0x80..=0x9B).contains(&opcode) || (0xA0..=0xAB).contains(&opcode) {
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
        let mut cursor = 1usize;
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
}
