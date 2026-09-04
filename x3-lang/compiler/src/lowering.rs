//! AST -> X3IR lowering pipeline.
//!
//! This module lowers X3 AST into X3 Intermediate Representation (X3IR),
//! which is a semantic representation suitable for verification, optimization,
//! and code generation.

use crate::intent_emit;
use crate::ir::{
    self, ChainMetricKind, Condition, CrdtKind as IrCrdtKind, EmergencyKind, LifecycleKind, Operation, ProofKind,
    SerialFormat, StorageKind, VectorOp, X3IR,
};
use x3_lang_ast::ast;
use x3_lang_ast::ast::*;
use x3_lang_common::Span;

pub type LoweredInstr = Operation;

/// Context for lowering operations
pub struct LowerCtx {
    /// Unique nonce for replay protection
    pub nonce: Option<String>,
    /// Chain ID for this context
    pub chain_id: Option<u64>,
}

impl LowerCtx {
    pub fn new() -> Self {
        LowerCtx {
            nonce: None,
            chain_id: None,
        }
    }
}

/// Lower an entire program to X3IR
pub fn lower_program(program: &Program, ctx: LowerCtx) -> Result<X3IR, x3_lang_common::X3Error> {
    let mut ir = X3IR::new();
    ir.metadata.nonce = ctx.nonce;
    ir.metadata.chain_id = ctx.chain_id;

    // Lower top-level declarations into operations
    for item in &program.items {
        match &item.node {
            Item::Function(func) => {
                // Lower function body as a sequence of operations
                lower_annotations_prefix(&func.annotations, &mut ir)?;
                lower_function_body(&func.body, &mut ir)?;
                lower_annotations_suffix(&func.annotations, &mut ir)?;
            }
            Item::GpuBlock(block) => {
                ir.push(Operation::GpuDispatch {
                    kernel: "inline_gpu_block".to_string(),
                    args: vec![format!("{:?}", block.body)],
                    is_simd: block.is_simd,
                });
            }
            Item::SimulateDecl(sim) => {
                let mut body_ir = X3IR::new();
                lower_function_body(&sim.body, &mut body_ir)?;
                ir.push(Operation::Simulate {
                    body: body_ir.operations,
                    receipt_slot: sim
                        .receipt
                        .as_ref()
                        .map(|sym| sym.as_str().to_string())
                        .unwrap_or_else(|| format!("{}_receipt", sim.name.as_str())),
                });
            }
            Item::ScheduledTask(task) => {
                let mut entry_ir = X3IR::new();
                lower_function_body(&task.body, &mut entry_ir)?;
                ir.push(Operation::ScheduledDispatch {
                    period_blocks: task.period_blocks as u32,
                    entry: entry_ir.operations,
                });
            }
            Item::IntentDecl(intent) => {
                ir.push(Operation::IntentResolve {
                    constraints: intent.constraints.iter().map(expression_to_string).collect(),
                    resolver: intent.name.as_str().to_string(),
                });
                lower_function_body(&intent.body, &mut ir)?;
            }
            Item::SubscriptionDecl(sub) => {
                ir.push(Operation::Call {
                    function: "charge_subscription".to_string(),
                    args: vec![sub.name.as_str().to_string(), sub.amount.to_string()],
                });
                lower_function_body(&sub.body, &mut ir)?;
            }
            Item::AtomicSwap(atomic) => {
                // ── Semantic validation ──────────────────────────────────
                // Validate hash function name if hashlock is specified
                if let Some(hashlock) = &atomic.hashlock {
                    let hash_fn = hashlock.hash_fn.as_str();
                    match hash_fn {
                        "sha256" | "blake2b" | "keccak256" | "ripemd160" => {}
                        _ => {
                            return Err(semantic(&format!(
                                "unsupported hash function '{hash_fn}' in atomic swap \
                                 hashlock; supported: sha256, blake2b, keccak256, ripemd160"
                            )));
                        }
                    }
                }

                // Validate source and destination chains map to known VM families
                // via parse_vm_family (called during parsing, but verify non-None)
                let src_chain = atomic.from_asset.chain.as_str();
                let dst_chain = atomic.to_asset.chain.as_str();
                let src_known = KNOWN_CHAIN_PREFIXES.iter().any(|p| src_chain.starts_with(p));
                let dst_known = KNOWN_CHAIN_PREFIXES.iter().any(|p| dst_chain.starts_with(p));
                if !src_known {
                    return Err(semantic(&format!(
                        "source chain '{src_chain}' is not a known chain prefix; \
                         expected one of: {}",
                        KNOWN_CHAIN_PREFIXES.join(", ")
                    )));
                }
                if !dst_known {
                    return Err(semantic(&format!(
                        "destination chain '{dst_chain}' is not a known chain prefix; \
                         expected one of: {}",
                        KNOWN_CHAIN_PREFIXES.join(", ")
                    )));
                }

                // Validate timeout ranges (non-zero, not absurdly large)
                if let Some(timeout) = &atomic.timeout_source {
                    let blocks = expression_to_blocks(timeout)?;
                    if blocks == 0 {
                        return Err(semantic("source timeout must be greater than 0 blocks"));
                    }
                    if blocks > MAX_TIMEOUT_BLOCKS {
                        return Err(semantic(&format!(
                            "source timeout of {blocks} blocks exceeds maximum {}",
                            MAX_TIMEOUT_BLOCKS
                        )));
                    }
                }
                if let Some(timeout) = &atomic.timeout_destination {
                    let blocks = expression_to_blocks(timeout)?;
                    if blocks == 0 {
                        return Err(semantic("destination timeout must be greater than 0 blocks"));
                    }
                    if blocks > MAX_TIMEOUT_BLOCKS {
                        return Err(semantic(&format!(
                            "destination timeout of {blocks} blocks exceeds maximum {}",
                            MAX_TIMEOUT_BLOCKS
                        )));
                    }
                }

                // Validate source and destination chains are different
                let src_chain_str = atomic.from_asset.chain.as_str();
                let dst_chain_str = atomic.to_asset.chain.as_str();
                if src_chain_str == dst_chain_str {
                    return Err(semantic(&format!(
                        "atomic swap source and destination chains are the same \
                         ('{src_chain_str}'); cross-chain swap must target a different chain"
                    )));
                }

                // Validate amount is positive if specified
                if let Some(amt_expr) = &atomic.amount {
                    let amount_val = expression_to_u128(amt_expr)?;
                    if amount_val == 0 {
                        return Err(semantic("atomic swap amount must be greater than 0"));
                    }
                }

                // Wrap in atomic block and lower body
                ir.push(Operation::AtomicBegin);

                // Add requires guards first
                for require in &atomic.requires {
                    ir.push(Operation::Require {
                        kind: require_kind_to_ir(&require.kind),
                        subject: require.subject.as_ref().map(|s| s.as_str().to_string()),
                        condition: expression_to_condition(&require.value)?,
                        error_msg: None,
                    });
                }

                // If amount is specified, add a lock operation for the source asset
                if let Some(amt) = &atomic.amount {
                    let amount_val = expression_to_u128(amt)?;
                    let receiver_str = atomic
                        .receiver
                        .as_ref()
                        .map(expression_to_string)
                        .unwrap_or_else(|| "receiver".to_string());
                    // Lock on source chain: funds come from the sender, not the receiver.
                    ir.push(Operation::Lock {
                        chain: atomic.from_asset.chain.as_str().to_string(),
                        asset: atomic.from_asset.name.as_str().to_string(),
                        amount: amount_val,
                        from: "sender".to_string(),
                    });
                    // Release on destination chain to the receiver.
                    ir.push(Operation::Release {
                        chain: atomic.to_asset.chain.as_str().to_string(),
                        asset: atomic.to_asset.name.as_str().to_string(),
                        to: receiver_str,
                    });
                }

                // Lower body statements
                for stmt in &atomic.body {
                    lower_statement(stmt, &mut ir)?;
                }

                // Add source timeout handling if specified
                if let Some(timeout_expr) = &atomic.timeout_source {
                    ir.push(Operation::OnTimeout {
                        duration_blocks: expression_to_blocks(timeout_expr)?,
                        action: ir::FailureAction::Refund {
                            chain: atomic.from_asset.chain.as_str().to_string(),
                            asset: atomic.from_asset.name.as_str().to_string(),
                            to: "sender".to_string(),
                        },
                    });
                }

                // Add destination timeout handling if specified
                if let Some(timeout_expr) = &atomic.timeout_destination {
                    ir.push(Operation::OnTimeout {
                        duration_blocks: expression_to_blocks(timeout_expr)?,
                        action: ir::FailureAction::Refund {
                            chain: atomic.to_asset.chain.as_str().to_string(),
                            asset: atomic.to_asset.name.as_str().to_string(),
                            to: "sender".to_string(),
                        },
                    });
                }

                // Add failure handling if specified
                if let Some(failure) = &atomic.on_fail {
                    ir.push(Operation::OnFail {
                        action: failure_action_to_ir(failure),
                    });
                }
                ir.push(Operation::AtomicEnd);
            }
            Item::Bridge(bridge) => {
                // Lower bridge as atomic sequence with requires
                ir.push(Operation::AtomicBegin);

                // Add requires guards first
                for require in &bridge.requires {
                    ir.push(Operation::Require {
                        kind: require_kind_to_ir(&require.kind),
                        subject: require.subject.as_ref().map(|s| s.as_str().to_string()),
                        condition: expression_to_condition(&require.value)?,
                        error_msg: None,
                    });
                }

                // Lower bridge body
                for stmt in &bridge.body {
                    lower_statement(stmt, &mut ir)?;
                }

                // Add failure handling if specified
                if let Some(failure) = &bridge.on_fail {
                    ir.push(Operation::OnFail {
                        action: failure_action_to_ir(failure),
                    });
                }

                // Add timeout handling if specified
                if let Some(timeout_expr) = &bridge.timeout {
                    ir.push(Operation::OnTimeout {
                        duration_blocks: expression_to_blocks(timeout_expr)?,
                        action: ir::FailureAction::Refund {
                            chain: bridge.from_asset.chain.as_str().to_string(),
                            asset: bridge.from_asset.name.as_str().to_string(),
                            to: "sender".to_string(),
                        },
                    });
                }

                ir.push(Operation::AtomicEnd);
            }
            Item::Strategy(strategy) => {
                // Lower strategy as constrained execution
                ir.push(Operation::AtomicBegin);

                // Add requires guards first
                for require in &strategy.requires {
                    ir.push(Operation::Require {
                        kind: require_kind_to_ir(&require.kind),
                        subject: require.subject.as_ref().map(|s| s.as_str().to_string()),
                        condition: expression_to_condition(&require.value)?,
                        error_msg: None,
                    });
                }

                // Lower strategy body (limited by max_steps)
                for stmt in &strategy.body {
                    lower_statement(stmt, &mut ir)?;
                }

                // Add failure handling if specified
                if let Some(failure) = &strategy.on_fail {
                    ir.push(Operation::OnFail {
                        action: failure_action_to_ir(failure),
                    });
                }

                ir.push(Operation::AtomicEnd);
            }
            _ => {} // Other items (types, imports, etc.) don't generate operations
        }
    }

    Ok(ir)
}

/// Lower a statement to IR operations
fn lower_statement(stmt: &Statement, ir: &mut X3IR) -> Result<(), x3_lang_common::X3Error> {
    match stmt {
        Statement::Expr(expr) => {
            // Lower expression (may produce multiple operations)
            lower_expression(expr, ir)?;
        }
        Statement::If {
            cond,
            then_block,
            else_block,
        } => {
            let cond_ir = expression_to_condition(cond)?;
            let then_ops = {
                let mut temp_ir = X3IR::new();
                lower_function_body(then_block, &mut temp_ir)?;
                temp_ir.operations
            };

            let else_ops = if let Some(else_blk) = else_block {
                let mut temp_ir = X3IR::new();
                lower_function_body(else_blk, &mut temp_ir)?;
                Some(temp_ir.operations)
            } else {
                None
            };

            ir.push(Operation::If {
                condition: cond_ir,
                then_ops,
                else_ops,
            });
        }
        Statement::While { cond, body } => {
            let _cond_ir = expression_to_condition(cond)?;
            let body_ops = {
                let mut temp_ir = X3IR::new();
                lower_function_body(body, &mut temp_ir)?;
                temp_ir.operations
            };

            ir.push(Operation::Loop {
                max_iterations: 1000, // Safe default limit
                body: body_ops,
            });
        }
        Statement::Atomic(atomic) => {
            ir.push(Operation::AtomicBegin);
            lower_function_body(&atomic.body, ir)?;
            ir.push(Operation::AtomicEnd);
        }
        Statement::Emit(event) => {
            let mut data = std::collections::HashMap::new();
            for (i, arg) in event.payload.iter().enumerate() {
                data.insert(format!("arg{}", i), format!("{:?}", arg));
            }
            ir.push(Operation::Emit {
                name: event.name.as_str().to_string(),
                data,
            });
        }
        Statement::Lock {
            chain,
            asset,
            amount,
            from,
        } => {
            // lock CHAIN.ASSET amount VALUE from ADDR
            ir.push(Operation::Lock {
                chain: chain_to_string(chain),
                asset: asset.name.as_str().to_string(),
                amount: expression_to_u128(amount)?,
                from: expression_to_string(from),
            });
        }
        Statement::Mint { asset, amount, to } => {
            // mint ASSET amount VALUE to ADDR
            ir.push(Operation::Mint {
                chain: chain_to_string(&asset.chain),
                asset: asset.name.as_str().to_string(),
                amount: expression_to_u128(amount)?,
                to: expression_to_string(to),
            });
        }
        Statement::Burn { asset, amount, from } => {
            // burn ASSET amount VALUE from ADDR
            ir.push(Operation::Burn {
                chain: chain_to_string(&asset.chain),
                asset: asset.name.as_str().to_string(),
                amount: expression_to_u128(amount)?,
                from: expression_to_string(from),
            });
        }
        Statement::Release { chain, asset, to } => {
            // release CHAIN.ASSET to ADDR
            ir.push(Operation::Release {
                chain: chain_to_string(chain),
                asset: asset.name.as_str().to_string(),
                to: expression_to_string(to),
            });
        }
        Statement::Swap {
            from,
            to,
            route,
            min_output,
            dex,
        } => {
            // swap FROM -> TO [route ...] [dex ...]
            ir.push(Operation::Swap {
                from_chain: chain_to_string(&from.chain),
                from_asset: from.name.as_str().to_string(),
                to_asset: to.name.as_str().to_string(),
                input_amount: route.as_ref().and_then(expression_to_u128_opt).unwrap_or(0),
                min_output: min_output.as_ref().and_then(expression_to_u128_opt).unwrap_or(0),
                dex: dex.as_ref().map(expression_to_string),
            });
        }
        Statement::Bridge {
            via,
            from,
            to,
            amount,
            receiver,
            source_finality_proof,
            transfer_proof,
        } => {
            ir.push(Operation::Bridge {
                via: via.as_str().to_string(),
                from_chain: chain_to_string(&from.chain),
                from_asset: from.name.as_str().to_string(),
                to_chain: chain_to_string(&to.chain),
                to_asset: to.name.as_str().to_string(),
                amount: expression_to_u128(amount)?,
                receiver: expression_to_string(receiver),
                source_finality_proof: source_finality_proof
                    .as_ref()
                    .map(expression_to_string)
                    .unwrap_or_default()
                    .into_bytes(),
                transfer_proof: transfer_proof
                    .as_ref()
                    .map(expression_to_string)
                    .unwrap_or_default()
                    .into_bytes(),
            });
        }
        Statement::Require(guard) => {
            // Surface `require nonce <subject> <value>` into the
            // program-level metadata so the replay-protection rule in
            // the semantic verifier sees it. The value is the nonce
            // string the consumer is committing to.
            if matches!(guard.kind, RequireKind::Nonce) {
                ir.metadata.nonce = Some(expression_to_string(&guard.value));
            }
            ir.push(Operation::Require {
                kind: require_kind_to_ir(&guard.kind),
                subject: guard.subject.as_ref().map(|s| s.as_str().to_string()),
                condition: expression_to_condition(&guard.value)?,
                error_msg: None,
            });
        }
        Statement::OnFail(action) => {
            ir.push(Operation::OnFail {
                action: failure_action_to_ir(action),
            });
        }
        Statement::OnTimeout { duration, action } => {
            // Accept either a u32 or u128 integer literal; clamp to u32.
            let dur_blocks: u32 = match &duration {
                Expression::Literal(LiteralExpr::Int { value, .. }) => {
                    if *value > u32::MAX as u128 {
                        u32::MAX
                    } else {
                        *value as u32
                    }
                }
                _ => expression_to_blocks(&duration)?,
            };
            ir.push(Operation::OnTimeout {
                duration_blocks: dur_blocks,
                action: failure_action_to_ir(action),
            });
            if let ast::FailureAction::Refund(expr) = action {
                if let crate::ir::FailureAction::Refund { chain, asset, to } = refund_expression_to_ir(expr) {
                    ir.push(Operation::Release {
                        chain: chain.to_ascii_lowercase(),
                        asset,
                        to,
                    });
                }
            }
        }
        Statement::Snapshot => {
            ir.push(Operation::ChainMetric {
                metric: ChainMetricKind::Snapshot,
            });
        }
        Statement::Diff { before, after } => {
            ir.push(Operation::Call {
                function: "diff".to_string(),
                args: vec![expression_to_string(before), expression_to_string(after)],
            });
        }
        Statement::CrdtOp(op) => {
            ir.push(Operation::CrdtOp {
                kind: crdt_kind_to_ir(&op.kind),
                key: expression_to_string(&op.key),
                value: op.value.as_ref().map(expression_to_string),
            });
        }
        Statement::ZkVerify {
            proof,
            public_input,
            key,
        } => {
            ir.push(Operation::ProofVerify {
                kind: ProofKind::Zk,
                proof: expression_to_string(proof),
                input: expression_to_string(public_input),
                key_or_threshold: expression_to_string(key),
            });
        }
        Statement::MpcVerify {
            result,
            signatures,
            threshold,
        } => {
            ir.push(Operation::ProofVerify {
                kind: ProofKind::Mpc,
                proof: expression_to_string(result),
                input: expression_to_string(signatures),
                key_or_threshold: expression_to_string(threshold),
            });
        }
        Statement::StorageRef { op, data } => {
            ir.push(Operation::StorageOp {
                kind: storage_kind_to_ir(op),
                data: expression_to_string(data),
            });
        }
        Statement::Pathfind { from, to, max_depth } => {
            ir.push(Operation::Pathfind {
                from: expression_to_string(from),
                to: expression_to_string(to),
                max_depth: expression_to_blocks(max_depth)?,
            });
        }
        Statement::MempoolScan { max_results } => {
            ir.push(Operation::MempoolScan {
                max_results: expression_to_blocks(max_results)?,
            });
        }
        Statement::OracleRequest { token, reward } => {
            ir.push(Operation::OracleRequest {
                token: expression_to_string(token),
                reward: expression_to_u128(reward)?,
            });
        }
        Statement::Pause => {
            ir.push(Operation::EmergencyControl {
                kind: EmergencyKind::Pause,
            });
        }
        Statement::Resume => {
            ir.push(Operation::EmergencyControl {
                kind: EmergencyKind::Resume,
            });
        }
        Statement::SelfDestruct => {
            ir.push(Operation::Lifecycle {
                kind: LifecycleKind::Destroy,
                target: None,
            });
        }
        Statement::Migrate { new_contract } => {
            ir.push(Operation::Lifecycle {
                kind: LifecycleKind::Migrate,
                target: Some(expression_to_string(new_contract)),
            });
        }
        _ => {
            // Other statement types (return, break, etc.)
            ir.push(Operation::Nop);
        }
    }
    Ok(())
}

/// Lower a function body (block of statements)
fn lower_function_body(block: &Block, ir: &mut X3IR) -> Result<(), x3_lang_common::X3Error> {
    for stmt in &block.stmts {
        lower_statement(stmt, ir)?;
    }
    Ok(())
}

/// Lower an expression to IR (may produce operations or just return values)
fn lower_expression(expr: &Expression, ir: &mut X3IR) -> Result<(), x3_lang_common::X3Error> {
    match expr {
        Expression::Literal(_) => {
            // Literals don't produce operations, just values
            Ok(())
        }
        Expression::Ident(_) => {
            // Variables don't produce operations
            Ok(())
        }
        Expression::Call { callee, args } => {
            lower_builtin_call(callee, args, ir)?;
            Ok(())
        }
        Expression::Binary { lhs: _, op: _, rhs: _ } => {
            // Binary operations don't produce direct IR ops (used in conditions)
            Ok(())
        }
        _ => Ok(()),
    }
}

fn lower_annotations_prefix(annotations: &[Annotation], ir: &mut X3IR) -> Result<(), x3_lang_common::X3Error> {
    for annotation in annotations {
        match annotation {
            Annotation::Role(role) => ir.push(Operation::RoleCheck {
                role: role.as_str().to_string(),
            }),
            Annotation::Multisig(required, total) => {
                if required > total {
                    return Err(semantic("@multisig requires required <= total"));
                }
                ir.push(Operation::MultisigCheck {
                    required: *required,
                    total: *total,
                });
            }
            Annotation::Subscription(amount, period) => ir.push(Operation::Call {
                function: "charge_subscription".to_string(),
                args: vec![amount.to_string(), period.to_string()],
            }),
            Annotation::Subscribe(event) => ir.push(Operation::Call {
                function: "subscribe_event".to_string(),
                args: vec![event.as_str().to_string()],
            }),
            Annotation::Sponsor => ir.push(Operation::Call {
                function: "deduct_sponsor_fee".to_string(),
                args: vec![],
            }),
            Annotation::Sandbox => ir.push(Operation::Require {
                kind: ir::RequireKind::Custom("sandbox_gas_limit".to_string()),
                subject: None,
                condition: Condition::True,
                error_msg: Some("sandbox gas limit exceeded".to_string()),
            }),
            Annotation::Whitelist(entries) => ir.push(Operation::Require {
                kind: ir::RequireKind::Custom("whitelist".to_string()),
                subject: None,
                condition: Condition::Expression {
                    expr: entries.iter().map(|sym| sym.as_str()).collect::<Vec<_>>().join(","),
                },
                error_msg: Some("call target not whitelisted".to_string()),
            }),
            Annotation::Hot => ir.push(Operation::Emit {
                name: "hot_enter".to_string(),
                data: Default::default(),
            }),
            Annotation::Audit => ir.push(Operation::Emit {
                name: "audit_enter".to_string(),
                data: Default::default(),
            }),
            Annotation::Extern => ir.push(Operation::AbiExport {
                function: "extern".to_string(),
                params: vec![],
                ret: "()".to_string(),
            }),
            Annotation::GasAdaptive => ir.push(Operation::GasAdaptive {
                high_gas_ops: vec![Operation::Nop],
                low_gas_ops: vec![Operation::Nop],
            }),
            Annotation::NoHeap
            | Annotation::NoRecursion(_)
            | Annotation::OnChain
            | Annotation::OffChain
            | Annotation::Concurrent
            | Annotation::Scheduled(_)
            | Annotation::Version(_)
            | Annotation::UpgradeFrom(_)
            | Annotation::Payable
            | Annotation::Simd => {}
        }
    }
    Ok(())
}

fn lower_annotations_suffix(annotations: &[Annotation], ir: &mut X3IR) -> Result<(), x3_lang_common::X3Error> {
    let mut version = None;
    let mut upgrade_from = None;
    for annotation in annotations {
        match annotation {
            Annotation::Version(value) => version = Some(value.as_str().to_string()),
            Annotation::UpgradeFrom(value) => upgrade_from = Some(value.as_str().to_string()),
            Annotation::Hot => ir.push(Operation::Emit {
                name: "hot_exit".to_string(),
                data: Default::default(),
            }),
            Annotation::Audit => ir.push(Operation::Emit {
                name: "audit_exit".to_string(),
                data: Default::default(),
            }),
            Annotation::Scheduled(period) => {
                let entry = std::mem::take(&mut ir.operations);
                ir.push(Operation::ScheduledDispatch {
                    period_blocks: *period as u32,
                    entry,
                });
            }
            Annotation::Simd => {
                let args = std::mem::take(&mut ir.operations)
                    .into_iter()
                    .map(|op| format!("{:?}", op))
                    .collect();
                ir.push(Operation::GpuDispatch {
                    kernel: "simd_function".to_string(),
                    args,
                    is_simd: true,
                });
            }
            _ => {}
        }
    }
    if let Some(version) = version {
        ir.push(Operation::VersionMeta { version, upgrade_from });
    }
    Ok(())
}

fn lower_builtin_call(callee: &Expression, args: &[Expression], ir: &mut X3IR) -> Result<(), x3_lang_common::X3Error> {
    let name = expression_to_string(callee);
    match name.as_str() {
        "encode_rlp" => emit_serialize(ir, SerialFormat::Rlp, args),
        "decode_rlp" => emit_deserialize(ir, SerialFormat::Rlp, args),
        "encode_cbor" => emit_serialize(ir, SerialFormat::Cbor, args),
        "decode_cbor" => emit_deserialize(ir, SerialFormat::Cbor, args),
        "encode_json" => emit_serialize(ir, SerialFormat::Json, args),
        "decode_json" => emit_deserialize(ir, SerialFormat::Json, args),
        "encode_ssz" => emit_serialize(ir, SerialFormat::Ssz, args),
        "decode_ssz" => emit_deserialize(ir, SerialFormat::Ssz, args),
        "estimate_evm_gas" => emit_gas_estimate(ir, "evm", args),
        "estimate_svm_gas" => emit_gas_estimate(ir, "svm", args),
        "estimate_x3_gas" => emit_gas_estimate(ir, "x3", args),
        "get_chain_congestion" => emit_metric(ir, ChainMetricKind::Congestion),
        "get_base_fee" => emit_metric(ir, ChainMetricKind::BaseFee),
        "get_finality_lag" => emit_metric(ir, ChainMetricKind::FinalityLag),
        "get_block_time" => emit_metric(ir, ChainMetricKind::BlockTime),
        "generate_event_proof" => ir.push(Operation::EventProvenance {
            event_type: arg_string(args, 0),
            data: arg_string(args, 1),
        }),
        "multi_hop_swap" => ir.push(Operation::MultiHopSwap {
            path: vec![arg_string(args, 0)],
            amount: arg_u128(args, 1)?,
        }),
        "resolve_intent" => ir.push(Operation::IntentResolve {
            constraints: vec![arg_string(args, 0)],
            resolver: "default".to_string(),
        }),
        "run_ai_model" => ir.push(Operation::GpuDispatch {
            kernel: arg_string(args, 0),
            args: args.iter().skip(1).map(expression_to_string).collect(),
            is_simd: false,
        }),
        "get_crdt" => ir.push(Operation::CrdtOp {
            kind: IrCrdtKind::Get,
            key: arg_string(args, 0),
            value: None,
        }),
        "set_crdt" => ir.push(Operation::CrdtOp {
            kind: IrCrdtKind::Set,
            key: arg_string(args, 0),
            value: Some(arg_string(args, 1)),
        }),
        "storage_store" => ir.push(Operation::StorageOp {
            kind: StorageKind::Store,
            data: arg_string(args, 0),
        }),
        "storage_load" => ir.push(Operation::StorageOp {
            kind: StorageKind::Load,
            data: arg_string(args, 0),
        }),
        "pathfind" => ir.push(Operation::Pathfind {
            from: arg_string(args, 0),
            to: arg_string(args, 1),
            max_depth: arg_u128(args, 2)? as u32,
        }),
        "mempool_scan" => ir.push(Operation::MempoolScan {
            max_results: arg_u128(args, 0)? as u32,
        }),
        "oracle_request" => ir.push(Operation::OracleRequest {
            token: arg_string(args, 0),
            reward: arg_u128(args, 1)?,
        }),
        "pause" => ir.push(Operation::EmergencyControl {
            kind: EmergencyKind::Pause,
        }),
        "resume" => ir.push(Operation::EmergencyControl {
            kind: EmergencyKind::Resume,
        }),
        "self_destruct" => ir.push(Operation::Lifecycle {
            kind: LifecycleKind::Destroy,
            target: None,
        }),
        "verify_zk" => ir.push(Operation::ProofVerify {
            kind: ProofKind::Zk,
            proof: arg_string(args, 0),
            input: arg_string(args, 1),
            key_or_threshold: arg_string(args, 2),
        }),
        "verify_mpc" => ir.push(Operation::ProofVerify {
            kind: ProofKind::Mpc,
            proof: arg_string(args, 0),
            input: arg_string(args, 1),
            key_or_threshold: arg_string(args, 2),
        }),
        "calculate_portfolio_value" => ir.push(Operation::VectorMath {
            op: VectorOp::DotProduct,
            a: arg_string(args, 0),
            b: arg_string(args, 1),
            size: args.len() as u32,
        }),
        _ => ir.push(Operation::Call {
            function: name,
            args: args.iter().map(expression_to_string).collect(),
        }),
    }
    Ok(())
}

/// Convert an AST expression to an IR Condition
fn expression_to_condition(expr: &Expression) -> Result<Condition, x3_lang_common::X3Error> {
    match expr {
        Expression::Literal(LiteralExpr::Bool(true)) => Ok(Condition::True),
        Expression::Literal(LiteralExpr::Bool(false)) => Ok(Condition::False),
        Expression::Call { callee, args } => condition_from_call(callee, args),
        _ => Ok(Condition::Expression {
            expr: expression_to_string(expr),
        }),
    }
}

/// Convert AST RequireKind to IR RequireKind
fn require_kind_to_ir(kind: &ast::RequireKind) -> ir::RequireKind {
    match kind {
        ast::RequireKind::CanonicalSupply => ir::RequireKind::CanonicalSupply,
        ast::RequireKind::Nonce => ir::RequireKind::NonceUnused,
        ast::RequireKind::BridgeLiquidity => ir::RequireKind::BridgeLiquidity,
        ast::RequireKind::Slippage => ir::RequireKind::SlippageTolerance,
        ast::RequireKind::Profit => ir::RequireKind::ProfitThreshold,
        ast::RequireKind::Finality => ir::RequireKind::Finality,
        ast::RequireKind::Custom(name) => ir::RequireKind::Custom(name.as_str().to_string()),
        ast::RequireKind::InvariantCheck => ir::RequireKind::Custom("invariant".to_string()),
        ast::RequireKind::RiskScore => ir::RequireKind::Custom("risk_score".to_string()),
        ast::RequireKind::AuditGate => ir::RequireKind::Custom("audit_gate".to_string()),
        ast::RequireKind::RelayerQuorum => ir::RequireKind::Custom("relayer_quorum".to_string()),
    }
}

/// Convert AST FailureAction to IR FailureAction
fn failure_action_to_ir(action: &ast::FailureAction) -> ir::FailureAction {
    match action {
        ast::FailureAction::Rollback => ir::FailureAction::Rollback,
        ast::FailureAction::Refund(expr) => refund_expression_to_ir(expr),
        ast::FailureAction::Halt => ir::FailureAction::Halt,
        ast::FailureAction::Quarantine => ir::FailureAction::Quarantine,
    }
}

fn expression_to_string(expr: &Expression) -> String {
    match expr {
        Expression::Literal(LiteralExpr::Int { value, .. }) => value.to_string(),
        Expression::Literal(LiteralExpr::Float { raw, .. }) => raw.as_str().to_string(),
        Expression::Literal(LiteralExpr::String(s)) => s.as_str().to_string(),
        Expression::Literal(LiteralExpr::Address(s)) => s.as_str().to_string(),
        Expression::Literal(LiteralExpr::Hash(s)) => s.as_str().to_string(),
        Expression::Literal(LiteralExpr::Percentage { value }) => value.as_str().to_string(),
        Expression::Literal(LiteralExpr::Duration { value, unit }) => {
            format!("{}{:?}", value, unit)
        }
        Expression::Literal(LiteralExpr::Bool(b)) => b.to_string(),
        Expression::Ident(s) => s.as_str().to_string(),
        Expression::Binary { op, lhs, rhs } => {
            format!("{} {:?} {}", expression_to_string(lhs), op, expression_to_string(rhs))
        }
        Expression::Call { callee, args } => format!(
            "{}({})",
            expression_to_string(callee),
            args.iter().map(expression_to_string).collect::<Vec<_>>().join(",")
        ),
        _ => format!("{:?}", expr),
    }
}

fn expression_to_u128(expr: &Expression) -> Result<u128, x3_lang_common::X3Error> {
    match expr {
        Expression::Literal(LiteralExpr::Int { value, .. }) => Ok(*value),
        Expression::Literal(LiteralExpr::Float { raw, .. }) => raw
            .as_str()
            .parse::<f64>()
            .map(|v| v as u128)
            .map_err(|_| semantic("invalid numeric literal")),
        _ => expression_to_string(expr)
            .parse::<u128>()
            .map_err(|_| semantic("expected numeric expression")),
    }
}
fn expression_to_u128_opt(expr: &Expression) -> Option<u128> {
    expression_to_u128(expr).ok()
}
fn chain_to_string(chain: &ChainRef) -> String {
    chain.as_str().to_ascii_lowercase()
}
fn expression_to_blocks(expr: &Expression) -> Result<u32, x3_lang_common::X3Error> {
    expression_to_u128(expr).map(|v| v as u32)
}
fn semantic(message: &str) -> x3_lang_common::X3Error {
    x3_lang_common::X3Error::SemanticError {
        message: message.to_string(),
        span: Span::DUMMY,
    }
}

/// Known chain prefixes (from parser VM family mapping).
const KNOWN_CHAIN_PREFIXES: &[&str] = &[
    "eth",
    "ethereum",
    "polygon",
    "arb",
    "optimism",
    "base",
    "bsc",
    "avax",
    "sol",
    "solana",
    "svm",
    "substrate",
    "dot",
    "ksm",
    "polkadot",
    "kusama",
    "btc",
    "bitcoin",
    "x3",
    "x3vm",
    "move",
    "sui",
    "aptos",
    "cosmwasm",
    "cosmos",
    "atom",
    "osmo",
    "cairo",
    "starknet",
    "ada",
    "cardano",
    "plutus",
    "ton",
    "fuel",
    "near",
    "xlm",
    "stellar",
    "soroban",
    "ink",
    "pvm",
    "zk",
    "zkvm",
    "risc0",
    "sp1",
];

/// Maximum allowed timeout in blocks (24 hours at 6s/block = 14400 blocks).
const MAX_TIMEOUT_BLOCKS: u32 = 14400;
fn crdt_kind_to_ir(kind: &CrdtOpKind) -> IrCrdtKind {
    match kind {
        CrdtOpKind::Get => IrCrdtKind::Get,
        CrdtOpKind::Set => IrCrdtKind::Set,
        CrdtOpKind::Append => IrCrdtKind::Append,
        CrdtOpKind::Merge => IrCrdtKind::Merge,
    }
}
fn storage_kind_to_ir(kind: &StorageRefOp) -> StorageKind {
    match kind {
        StorageRefOp::Store => StorageKind::Store,
        StorageRefOp::Load => StorageKind::Load,
    }
}
fn arg_string(args: &[Expression], idx: usize) -> String {
    args.get(idx).map(expression_to_string).unwrap_or_default()
}
fn arg_u128(args: &[Expression], idx: usize) -> Result<u128, x3_lang_common::X3Error> {
    args.get(idx).map(expression_to_u128).unwrap_or(Ok(0))
}
fn emit_serialize(ir: &mut X3IR, format: SerialFormat, args: &[Expression]) {
    ir.push(Operation::Serialize {
        format,
        data: arg_string(args, 0),
    });
}
fn emit_deserialize(ir: &mut X3IR, format: SerialFormat, args: &[Expression]) {
    ir.push(Operation::Deserialize {
        format,
        data: arg_string(args, 0),
    });
}
fn emit_gas_estimate(ir: &mut X3IR, chain: &str, args: &[Expression]) {
    ir.push(Operation::GasEstimate {
        chain: chain.to_string(),
        route: arg_string(args, 0),
    });
}
fn emit_metric(ir: &mut X3IR, metric: ChainMetricKind) {
    ir.push(Operation::ChainMetric { metric });
}
fn condition_from_call(callee: &Expression, args: &[Expression]) -> Result<Condition, x3_lang_common::X3Error> {
    let name = expression_to_string(callee);
    if name == "verify_proof" && args.len() >= 2 {
        return Ok(Condition::ProofValid {
            proof: expression_to_string(&args[0]),
            expected_hash: expression_to_string(&args[1]),
        });
    }
    if name == "nonce" && args.len() >= 2 {
        return Ok(Condition::NonceEq {
            account: expression_to_string(&args[0]),
            expected: expression_to_u128(&args[1])? as u64,
        });
    }
    Ok(Condition::Expression {
        expr: format!(
            "{}({})",
            name,
            args.iter().map(expression_to_string).collect::<Vec<_>>().join(",")
        ),
    })
}
fn refund_expression_to_ir(expr: &Expression) -> ir::FailureAction {
    let value = expression_to_string(expr);
    let mut parts = value.split(':');
    let asset = parts.next().unwrap_or("unknown.UNKNOWN");
    let to = parts.next().unwrap_or("sender").to_string();
    let mut asset_parts = asset.split('.');
    ir::FailureAction::Refund {
        chain: asset_parts.next().unwrap_or("unknown").to_string(),
        asset: asset_parts.next().unwrap_or(asset).to_string(),
        to,
    }
}
