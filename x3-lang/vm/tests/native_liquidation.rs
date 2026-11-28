//! Public compiler -> verified VM -> typed native host tests. These fixture
//! assertions do not stand in for the separate real-Anvil transport proof.
use std::{cell::RefCell, rc::Rc};
use x3_lang_common::liquidation::LiquidationPolicy;
use x3_lang_vm::{liquidation::*, spec::opcodes::*, InstructionStream, VMConfig, VM};

fn address(n: u8) -> String {
    format!("0x{n:02x}{}", "00".repeat(19))
}
fn hash(n: u8) -> String {
    format!("0x{n:02x}{}", "00".repeat(31))
}

fn fixture() -> (LiquidationPolicy, Batch) {
    let market = MarketParams {
        loan_token: address(1),
        collateral_token: address(2),
        oracle: address(3),
        irm: address(4),
        lltv: 860_000_000_000_000_000,
    };
    let policy = LiquidationPolicy {
        chain_id: 31337,
        executor: address(5),
        executor_code_hash: hash(6),
        keeper: address(7),
        owner: address(8),
        morpho: address(9),
        loan_token: address(1),
        markets: vec![market_id(&market).unwrap()],
        routers: vec![address(10)],
        min_repay: 1_000_000_000,
        max_repay: 100_000_000_000,
        min_profit: 5_000_000,
        max_gas: 500_000,
        max_fee_per_gas: 10,
        max_priority_fee_per_gas: 2,
        max_total_fee_wei: 5_000_100,
        max_age_blocks: 1,
        expires_at_block: 200,
        max_candidates: 64,
    };
    let candidate = Candidate {
        id: "best".into(),
        market,
        borrower: address(11),
        seized_assets: 10_000_000,
        repay_assets: 2_000_000_000,
        gross_profit: 9_000_000,
        router: address(10),
        swap_data: vec![1, 2, 3, 4],
        gas_limit: 500_000,
        max_fee_per_gas: 10,
        max_priority_fee_per_gas: 1,
        cost: Some(ExecutionCost {
            native_wei: 5_000_100,
            loan_units: 1_000_000,
            extra_native_wei: Some(100),
        }),
    };
    (
        policy,
        Batch {
            snapshot: Snapshot {
                chain_id: 31337,
                block_number: 100,
                block_hash: hash(12),
                nonce: 3,
            },
            current_block: 101,
            candidates: vec![candidate],
        },
    )
}

fn source(policy: &LiquidationPolicy) -> String {
    let value = serde_json::to_value(policy).unwrap();
    let fields = value
        .as_object()
        .unwrap()
        .iter()
        .map(|(name, value)| format!("{name}: {value};"))
        .collect::<Vec<_>>()
        .join("\n");
    format!("native_liquidation Monkey {{\n{fields}\n}}")
}

#[test]
fn runtime_ranks_net_keep_not_scanner_order_or_gross_bonus() {
    let (policy, mut batch) = fixture();
    let mut worse = batch.candidates[0].clone();
    worse.id = "larger-gross-worse-net".into();
    worse.gross_profit = 10_000_000;
    worse.cost.as_mut().unwrap().loan_units = 3_000_000;
    batch.candidates.insert(0, worse);
    let selected = authorize(&policy, [1; 32], &batch).unwrap();
    assert_eq!(selected.candidate.id, "best");
    assert_eq!(selected.net_profit, 8_000_000);
    batch.candidates.reverse();
    assert_eq!(selected, authorize(&policy, [1; 32], &batch).unwrap());
    let mut equal = batch.candidates[0].clone();
    equal.id = "a-tie".into();
    batch.candidates.push(equal);
    assert_eq!(authorize(&policy, [1; 32], &batch).unwrap().candidate.id, "a-tie");
}

#[test]
fn authority_and_economics_fail_closed() {
    let (policy, good) = fixture();
    let mutations: Vec<Box<dyn Fn(&mut Batch)>> = vec![
        Box::new(|b| b.snapshot.chain_id = 8453),
        Box::new(|b| b.current_block = 201),
        Box::new(|b| b.current_block = 102),
        Box::new(|b| b.current_block = 99),
        Box::new(|b| b.candidates[0].market.loan_token = address(20)),
        Box::new(|b| b.candidates[0].market.lltv -= 1),
        Box::new(|b| b.candidates[0].router = address(20)),
        Box::new(|b| b.candidates[0].repay_assets = 999_999_999),
        Box::new(|b| b.candidates[0].repay_assets = 100_000_000_001),
        Box::new(|b| b.candidates[0].seized_assets = 0),
        Box::new(|b| b.candidates[0].gas_limit += 1),
        Box::new(|b| b.candidates[0].max_fee_per_gas += 1),
        Box::new(|b| b.candidates[0].max_priority_fee_per_gas = 3),
        Box::new(|b| b.candidates[0].cost = None),
        Box::new(|b| b.candidates[0].cost.as_mut().unwrap().extra_native_wei = None),
        Box::new(|b| b.candidates[0].cost.as_mut().unwrap().native_wei -= 1),
        Box::new(|b| b.candidates[0].cost.as_mut().unwrap().loan_units = 0),
        Box::new(|b| b.candidates[0].gross_profit = 5_999_999),
        Box::new(|b| b.candidates[0].swap_data.clear()),
    ];
    for (index, mutation) in mutations.iter().enumerate() {
        let mut batch = good.clone();
        mutation(&mut batch);
        assert!(authorize(&policy, [1; 32], &batch).is_err(), "mutation {index}");
    }
    let mut duplicate = good.clone();
    duplicate.candidates.push(duplicate.candidates[0].clone());
    assert!(authorize(&policy, [1; 32], &duplicate).is_err());
    let mut bounded = good.clone();
    bounded.candidates.resize(65, good.candidates[0].clone());
    assert!(authorize(&policy, [1; 32], &bounded).is_err());
    // One malformed lead does not hide a valid opportunity later in the batch.
    let mut batch = good.clone();
    let mut invalid = batch.candidates[0].clone();
    invalid.id = "bad-lead".into();
    invalid.cost = None;
    batch.candidates.insert(0, invalid);
    assert_eq!(authorize(&policy, [1; 32], &batch).unwrap().candidate.id, "best");
}

#[derive(Clone)]
struct TestHost {
    digest: [u8; 32],
    batch: Batch,
    effects: Rc<RefCell<usize>>,
    fail: bool,
}
impl LiquidationHost for TestHost {
    fn artifact_hash(&self) -> [u8; 32] {
        self.digest
    }
    fn candidates(&mut self, _: &LiquidationPolicy) -> Result<Batch, String> {
        Ok(self.batch.clone())
    }
    fn execute(&mut self, _: &AuthorizedLiquidation) -> Result<EffectReceipt, String> {
        *self.effects.borrow_mut() += 1;
        if self.fail {
            return Err("uncertain submission".into());
        }
        Ok(EffectReceipt {
            request_hash: [2; 32],
            transaction_hash: None,
            status: EffectStatus::Prepared,
        })
    }
}

#[test]
fn compiled_x3_controls_verified_vm_native_effect_and_pins_artifact() {
    let (policy, batch) = fixture();
    let code = x3_lang_compiler::compile_source(&source(&policy)).unwrap();
    assert_eq!(code[0], 3);
    let effects = Rc::new(RefCell::new(0));
    let host = TestHost {
        digest: artifact_hash(&code),
        batch,
        effects: effects.clone(),
        fail: false,
    };
    let mut vm = VM::new(code.clone(), VMConfig::default(), 1_000_000);
    assert!(format!("{:?}", vm.execute().unwrap_err()).contains("HOST_REQUIRED"));
    let mut vm = VM::new(code.clone(), VMConfig::default(), 1_000_000);
    vm.set_liquidation_host(Box::new(host.clone()));
    // HALT must not report a runtime timeout after the native effect succeeded.
    vm.state.timeout_deadline = Some(1);
    vm.execute().unwrap();
    vm.execute().unwrap(); // completed programs cannot repeat the native effect
    assert_eq!(*effects.borrow(), 1);
    assert_eq!(vm.liquidation_receipt.unwrap().authorization.net_profit, 8_000_000);
    let mut wrong = host.clone();
    wrong.digest = [0; 32];
    let mut vm = VM::new(code.clone(), VMConfig::default(), 1_000_000);
    vm.set_liquidation_host(Box::new(wrong));
    assert!(format!("{:?}", vm.execute().unwrap_err()).contains("ARTIFACT_NOT_APPROVED"));
    assert_eq!(*effects.borrow(), 1);
    // Altering the compiled mandate cannot use the old host's authority.
    let mut permissive = policy;
    permissive.min_repay = 1;
    let altered = x3_lang_compiler::compile_source(&source(&permissive)).unwrap();
    let mut vm = VM::new(altered, VMConfig::default(), 1_000_000);
    vm.set_liquidation_host(Box::new(host));
    assert!(vm.execute().is_err());
    assert_eq!(*effects.borrow(), 1);
}

#[test]
fn unconfigured_invalid_unfunded_or_composed_program_never_reaches_effect() {
    let (policy, batch) = fixture();
    let code = x3_lang_compiler::compile_source(&source(&policy)).unwrap();
    let effects = Rc::new(RefCell::new(0));
    let host = TestHost {
        digest: artifact_hash(&code),
        batch,
        effects: effects.clone(),
        fail: false,
    };
    let mut vm = VM::new(code.clone(), VMConfig::default(), code.len() as u128);
    vm.set_liquidation_host(Box::new(host.clone()));
    assert!(vm.execute().is_err());
    assert_eq!(*effects.borrow(), 0);
    // A native opcode cannot be followed by another effect or failure, even in
    // a hand-edited artifact approved by a caller who bypassed the compiler.
    let halt_pc = code.len() - 4;
    for suffix in [&[REQUIRE, 0, 0, 0][..], &[ON_FAIL, 0, 0, 0][..], &[NOP, 0, 0, 0][..]] {
        let mut altered = code[..halt_pc].to_vec();
        altered.extend_from_slice(suffix);
        altered.extend_from_slice(&[HALT, 0, 0, 0]);
        assert!(x3_lang_vm::verifier::verify(&InstructionStream::new(altered)).is_err());
    }
    let mut invalid = host.clone();
    invalid.batch.candidates[0].cost = None;
    let mut vm = VM::new(code.clone(), VMConfig::default(), 1_000_000);
    vm.set_liquidation_host(Box::new(invalid));
    assert!(vm.execute().is_err());
    assert_eq!(*effects.borrow(), 0);
    let mut failed = host;
    failed.fail = true;
    let halt_pc = code.len() - 4;
    let mut vm = VM::new(code, VMConfig::default(), 1_000_000);
    vm.set_liquidation_host(Box::new(failed));
    // Embedders can seed generic VM recovery handlers. A native effect's
    // uncertain submission must remain an error even if HALT is a handler.
    vm.state.failure_handlers.push(halt_pc);
    assert!(format!("{:?}", vm.execute().unwrap_err()).contains("uncertain submission"));
    assert!(format!("{:?}", vm.execute().unwrap_err()).contains("ALREADY_ATTEMPTED"));
    assert_eq!(*effects.borrow(), 1);
}

#[test]
#[ignore = "explicit local decision benchmark; excludes RPC, simulation and signing"]
fn benchmark_compiled_vm_selection_64_candidates() {
    let (policy, mut batch) = fixture();
    let template = batch.candidates[0].clone();
    batch.candidates = (0..64)
        .map(|index| {
            let mut candidate = template.clone();
            candidate.id = format!("candidate-{index:02}");
            candidate.gross_profit += index;
            candidate
        })
        .collect();
    let code = x3_lang_compiler::compile_source(&source(&policy)).unwrap();
    let effects = Rc::new(RefCell::new(0));
    let host = TestHost {
        digest: artifact_hash(&code),
        batch,
        effects,
        fail: false,
    };
    let mut times = Vec::with_capacity(2_000);
    let mut baseline = Vec::with_capacity(2_000);
    for index in 0..2_100 {
        let start = std::time::Instant::now();
        let direct = authorize(&policy, host.digest, &host.batch).unwrap();
        let direct_elapsed = start.elapsed().as_nanos();
        assert_eq!(direct.candidate.id, "candidate-63");
        let mut vm = VM::new(code.clone(), VMConfig::default(), 1_000_000);
        vm.set_liquidation_host(Box::new(host.clone()));
        let start = std::time::Instant::now();
        vm.execute().unwrap();
        let elapsed = start.elapsed().as_nanos();
        assert_eq!(
            vm.liquidation_receipt.unwrap().authorization.candidate.id,
            "candidate-63"
        );
        if index >= 100 {
            times.push(elapsed);
            baseline.push(direct_elapsed);
        }
    }
    times.sort_unstable();
    baseline.sort_unstable();
    println!(
        "{}",
        serde_json::json!({"scope":"verified VM selection plus fixture native effect; no network or signing",
        "candidates":64,"samples":times.len(),"warmup":100,"p50_ns":times[1000],
        "p95_ns":times[1900],"p99_ns":times[1980],"direct_authorize_baseline":{
            "p50_ns":baseline[1000],"p95_ns":baseline[1900],"p99_ns":baseline[1980]}})
    );
}
