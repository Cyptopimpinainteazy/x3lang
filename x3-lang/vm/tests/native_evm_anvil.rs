//! Explicit local chain proof. The lending protocol and executor are genuine;
//! collateral, loan token, oracle and DEX are fixture contracts. No public RPC.
use serde_json::{json, Value};
use std::{
    io::{BufRead, BufReader, Write},
    net::{TcpListener, TcpStream},
    path::PathBuf,
    process::{Child, ChildStdin, Command, Stdio},
    sync::{atomic::AtomicBool, mpsc, Arc},
    thread,
    time::Duration,
};
use x3_lang_common::liquidation::LiquidationPolicy;
use x3_lang_vm::{
    liquidation::*,
    liquidation_evm::{EphemeralLocalSigner, EvmTransport, EvmTransportConfig, TransportMode},
    VMConfig, VM,
};

struct Fixture {
    child: Child,
    input: ChildStdin,
    lines: mpsc::Receiver<String>,
    manifest: Value,
}
impl Fixture {
    fn start() -> Self {
        let directory = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/native-liquidation");
        assert!(
            directory.join("out/artifacts.json").is_file(),
            "run npm ci --ignore-scripts && npm run build in the standalone fixture first"
        );
        let mut child = Command::new("node")
            .arg("fixture.mjs")
            .arg("--serve")
            .current_dir(directory)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit())
            .spawn()
            .expect("Node and a built standalone fixture are required; no integration skip");
        let input = child.stdin.take().unwrap();
        let output = child.stdout.take().unwrap();
        let (sender, lines) = mpsc::channel();
        thread::spawn(move || {
            for line in BufReader::new(output).lines() {
                match line {
                    Ok(line) => {
                        if sender.send(line).is_err() {
                            break;
                        }
                    }
                    Err(_) => break,
                }
            }
        });
        let manifest = serde_json::from_str(
            &lines
                .recv_timeout(Duration::from_secs(90))
                .expect("local fixture did not start; inspect inherited stderr"),
        )
        .unwrap();
        Self {
            child,
            input,
            lines,
            manifest,
        }
    }
    fn command(&mut self, value: Value) -> Value {
        writeln!(self.input, "{value}").unwrap();
        self.input.flush().unwrap();
        let reply: Value = serde_json::from_str(
            &self
                .lines
                .recv_timeout(Duration::from_secs(30))
                .expect("local fixture command timed out"),
        )
        .unwrap();
        assert_eq!(reply["ok"], true, "{reply}");
        reply
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = writeln!(self.input, "{{\"op\":\"stop\"}}");
        let _ = self.input.flush();
        for _ in 0..100 {
            if self.child.try_wait().ok().flatten().is_some() {
                return;
            }
            thread::sleep(Duration::from_millis(20));
        }
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

fn s(value: &Value, key: &str) -> String {
    value[key].as_str().unwrap().to_owned()
}
fn n(value: &Value, key: &str) -> u128 {
    value[key].as_str().unwrap().parse().unwrap()
}
fn rpc(url: &str, method: &str, params: Value) -> Value {
    let value: Value = reqwest::blocking::Client::builder()
        .no_proxy()
        .timeout(Duration::from_secs(10))
        .build()
        .unwrap()
        .post(url)
        .json(&json!({"jsonrpc":"2.0","id":1,"method":method,"params":params}))
        .send()
        .unwrap()
        .json()
        .unwrap();
    assert!(value.get("error").is_none(), "{method}: {value}");
    value["result"].clone()
}
fn artifact(policy: &LiquidationPolicy) -> Vec<u8> {
    let fields = serde_json::to_value(policy)
        .unwrap()
        .as_object()
        .unwrap()
        .iter()
        .map(|(key, value)| format!("{key}: {value};"))
        .collect::<Vec<_>>()
        .join("\n");
    x3_lang_compiler::compile_source(&format!("native_liquidation NativeProof {{\n{fields}\n}}")).unwrap()
}
fn fixture_inputs(manifest: &Value) -> (LiquidationPolicy, Batch) {
    let m = &manifest["market"];
    let market = MarketParams {
        loan_token: s(m, "loan_token"),
        collateral_token: s(m, "collateral_token"),
        oracle: s(m, "oracle"),
        irm: s(m, "irm"),
        lltv: n(m, "lltv"),
    };
    let gas_limit = n(manifest, "gas_limit") as u64;
    let max_fee_per_gas = 2_000_000_000;
    // Local fixture valuation is explicit trusted host input; not a live USDC quote.
    let cost = ExecutionCost {
        native_wei: u128::from(gas_limit) * max_fee_per_gas,
        loan_units: 1_000,
        extra_native_wei: Some(0),
    };
    let policy = LiquidationPolicy {
        chain_id: 31337,
        executor: s(manifest, "executor"),
        executor_code_hash: s(manifest, "executor_code_hash"),
        keeper: s(manifest, "keeper"),
        owner: s(manifest, "owner"),
        morpho: s(manifest, "morpho"),
        loan_token: market.loan_token.clone(),
        markets: vec![market_id(&market).unwrap()],
        routers: vec![s(manifest, "router")],
        min_repay: 1_000_000,
        max_repay: 1_000_000_000_000,
        min_profit: 1_000_000,
        max_gas: gas_limit,
        max_fee_per_gas,
        max_priority_fee_per_gas: 1_000_000_000,
        max_total_fee_wei: cost.native_wei,
        max_age_blocks: 0,
        expires_at_block: n(manifest, "block_number") as u64 + 100,
        max_candidates: 4,
    };
    let candidate = Candidate {
        id: "genuine-morpho-local".into(),
        market,
        borrower: s(manifest, "borrower"),
        seized_assets: n(manifest, "seized_assets"),
        repay_assets: n(manifest, "repay_assets"),
        gross_profit: n(manifest, "gross_profit"),
        router: s(manifest, "router"),
        swap_data: decode_hex(&s(manifest, "swap_data")),
        gas_limit,
        max_fee_per_gas,
        max_priority_fee_per_gas: 1_000_000_000,
        cost: Some(cost),
    };
    let batch = Batch {
        snapshot: Snapshot {
            chain_id: 31337,
            block_number: n(manifest, "block_number") as u64,
            block_hash: s(manifest, "block_hash"),
            nonce: 0,
        },
        current_block: n(manifest, "block_number") as u64,
        candidates: vec![candidate],
    };
    (policy, batch)
}
fn decode_hex(value: &str) -> Vec<u8> {
    value.as_bytes()[2..]
        .chunks_exact(2)
        .map(|pair| u8::from_str_radix(std::str::from_utf8(pair).unwrap(), 16).unwrap())
        .collect()
}
fn config(url: &str, mode: TransportMode) -> EvmTransportConfig {
    EvmTransportConfig {
        read_url: url.into(),
        submit_url: (mode == TransportMode::LocalBroadcast).then(|| url.into()),
        mode,
        timeout: Duration::from_secs(5),
    }
}
fn prepare(url: &str, policy: &LiquidationPolicy, batch: &Batch) -> Result<EffectReceipt, String> {
    let code = artifact(policy);
    let digest = artifact_hash(&code);
    let authorized = authorize(policy, digest, batch)?;
    EvmTransport::new(
        config(url, TransportMode::PrepareOnly),
        &code,
        digest,
        batch.clone(),
        None,
        Arc::new(AtomicBool::new(false)),
    )?
    .execute(&authorized)
}

#[test]
#[ignore = "requires Node, Anvil and built standalone fixture; run the explicit native EVM integration gate"]
fn native_evm_compiled_vm_signs_mines_and_reconciles_only_on_owned_anvil() {
    let mut fixture = Fixture::start();
    // An explicit host input: the VM itself never generates entropy.
    let signer = EphemeralLocalSigner::from_declared_secret(secp256k1::SecretKey::new(&mut rand::thread_rng()));
    let keeper = signer.address().to_string().to_ascii_lowercase();
    let response = fixture.command(json!({"op":"setKeeper","address":keeper}));
    fixture.manifest = response["manifest"].clone();
    let manifest = fixture.manifest.clone();
    let url = s(&manifest, "rpc_url");
    assert!(url.starts_with("http://127.0.0.1:"));
    let (policy, batch) = fixture_inputs(&manifest);
    let code = artifact(&policy);
    let digest = artifact_hash(&code);
    let authorized = authorize(&policy, digest, &batch).unwrap();
    let before = fixture.command(json!({"op":"state"}));
    let kill = Arc::new(AtomicBool::new(false));
    let mut unsigned = EvmTransport::new(
        config(&url, TransportMode::PrepareOnly),
        &code,
        digest,
        batch.clone(),
        None,
        kill.clone(),
    )
    .unwrap();
    let prepared = unsigned.execute(&authorized).unwrap();
    assert_eq!(prepared.status, EffectStatus::Prepared);
    assert_eq!(prepared.transaction_hash, None);
    assert!(!unsigned.nonce_reserved(31337, 0));
    assert!(unsigned.last_attempt().is_none());
    let request = unsigned.prepared_request().unwrap().clone();
    assert_eq!(request.simulated_repaid, n(&manifest, "repay_assets"));
    assert_eq!(request.simulated_profit, n(&manifest, "gross_profit"));
    assert!(request.simulated_gas_used > 21_000);
    assert_eq!(rpc(&url, "eth_getTransactionCount", json!([keeper, "latest"])), "0x0");

    let mut negatives = Vec::new();
    for (name, mutate) in [
        ("wrong chain", 0),
        ("wrong head", 1),
        ("wrong nonce", 2),
        ("wrong executor code", 3),
        ("wrong owner", 4),
        ("wrong keeper", 5),
        ("wrong Morpho", 6),
        ("understated repayment", 7),
        ("overstated profit", 8),
    ] {
        let mut p = policy.clone();
        let mut b = batch.clone();
        match mutate {
            0 => {
                p.chain_id = 8453;
                b.snapshot.chain_id = 8453;
            }
            1 => b.snapshot.block_hash = format!("0x{}", "22".repeat(32)),
            2 => b.snapshot.nonce = 1,
            3 => p.executor_code_hash = format!("0x{}", "33".repeat(32)),
            4 => p.owner = p.keeper.clone(),
            5 => p.keeper = p.owner.clone(),
            6 => p.morpho = p.owner.clone(),
            7 => b.candidates[0].repay_assets -= 1,
            8 => b.candidates[0].gross_profit += 1,
            _ => unreachable!(),
        }
        let error = prepare(&url, &p, &b).unwrap_err();
        negatives.push(json!({"case":name,"error":error}));
    }
    kill.store(true, std::sync::atomic::Ordering::SeqCst);
    assert!(unsigned.execute(&authorized).unwrap_err().contains("KILLED"));
    kill.store(false, std::sync::atomic::Ordering::SeqCst);
    let mut unknown = batch.clone();
    unknown.candidates[0].cost.as_mut().unwrap().extra_native_wei = None;
    assert!(prepare(&url, &policy, &unknown).is_err());
    let mut tampered = authorized.clone();
    tampered.calldata[0] ^= 1;
    assert!(unsigned
        .execute(&tampered)
        .unwrap_err()
        .contains("AUTHORIZATION_CHANGED"));
    assert_eq!(
        fixture.command(json!({"op":"state"})),
        before,
        "all refused and prepared calls leave chain state untouched"
    );

    // A different local node can use the same chain ID while serving another
    // fork. Reject that submission endpoint before signing or reserving a nonce.
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let wrong_url = format!("http://{}", listener.local_addr().unwrap());
    let upstream = url.clone();
    let wrong_head = thread::spawn(move || {
        for stream in listener.incoming().take(2) {
            let mut stream = stream.unwrap();
            let input = read_http_json(&mut stream);
            let method = input["method"].as_str().unwrap();
            let mut result = rpc(&upstream, method, input["params"].clone());
            if method == "eth_getBlockByNumber" {
                result["hash"] = json!(format!("0x{}", "44".repeat(32)));
            }
            let output = json!({"jsonrpc":"2.0","id":1,"result":result}).to_string();
            write!(stream,"HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",output.len(),output).unwrap();
        }
    });
    let mut wrong_config = config(&url, TransportMode::LocalBroadcast);
    wrong_config.submit_url = Some(wrong_url);
    let mut wrong_host = EvmTransport::new(
        wrong_config,
        &code,
        digest,
        batch.clone(),
        Some(signer.clone()),
        kill.clone(),
    )
    .unwrap();
    assert!(wrong_host.execute(&authorized).unwrap_err().contains("HEAD_CHANGED"));
    wrong_head.join().unwrap();
    assert!(wrong_host.last_attempt().is_none());
    assert!(!wrong_host.nonce_reserved(31337, 0));
    negatives.push(json!({"case":"different submission fork","result":"refused before signing and nonce reservation"}));

    let transport = EvmTransport::new(
        config(&url, TransportMode::LocalBroadcast),
        &code,
        digest,
        batch.clone(),
        Some(signer.clone()),
        kill,
    )
    .unwrap();
    let mut vm = VM::new(code.clone(), VMConfig::default(), 1_000_000);
    vm.set_liquidation_host(Box::new(transport));
    vm.execute().unwrap();
    let decision = vm.liquidation_receipt.clone().unwrap();
    assert_eq!(decision.effect.status, EffectStatus::Submitted);
    let transaction_hash = decision.effect.transaction_hash.clone().unwrap();
    let receipt = rpc(&url, "eth_getTransactionReceipt", json!([transaction_hash]));
    assert_eq!(receipt["status"], "0x1");
    let tx = rpc(&url, "eth_getTransactionByHash", json!([transaction_hash]));
    assert_eq!(tx["type"], "0x2");
    assert_eq!(tx["chainId"], "0x7a69");
    assert_eq!(tx["nonce"], "0x0");
    assert_eq!(tx["from"].as_str().unwrap().to_ascii_lowercase(), keeper);
    assert_eq!(tx["to"].as_str().unwrap().to_ascii_lowercase(), policy.executor);
    assert_eq!(decode_hex(tx["input"].as_str().unwrap()), authorized.calldata);
    let after = fixture.command(json!({"op":"state"}));
    assert_eq!(
        n(&after, "owner_loan") - n(&before, "owner_loan"),
        request.simulated_profit
    );
    assert_eq!(
        n(&after, "morpho_loan") - n(&before, "morpho_loan"),
        request.simulated_repaid
    );
    assert!(n(&after, "borrow_shares") < n(&before, "borrow_shares"));
    assert_eq!(n(&after, "executor_loan"), n(&before, "executor_loan"));
    assert_eq!(n(&after, "executor_collateral"), n(&before, "executor_collateral"));
    assert_eq!(n(&after, "router_allowance"), 0);
    let mined_gas = u128::from_str_radix(receipt["gasUsed"].as_str().unwrap().trim_start_matches("0x"), 16).unwrap();
    let mined_price = u128::from_str_radix(
        receipt["effectiveGasPrice"].as_str().unwrap().trim_start_matches("0x"),
        16,
    )
    .unwrap();
    assert_eq!(
        n(&before, "keeper_native") - n(&after, "keeper_native"),
        mined_gas * mined_price,
        "local native balance exactly pays the mined receipt fee"
    );
    vm.execute().unwrap(); // A halted VM returns without executing another effect.
    assert_eq!(
        fixture.command(json!({"op":"state"})),
        after,
        "re-entering a halted VM cannot resend"
    );

    // Lose the RPC acknowledgement AFTER the actual node accepts the signed tx.
    // Reservation survives this uncertain result and prevents a blind resend.
    let reset = fixture.command(json!({"op":"reset"}));
    let (p, b) = fixture_inputs(&reset["manifest"]);
    let code2 = artifact(&p);
    let digest2 = artifact_hash(&code2);
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let proxy_url = format!("http://{}", listener.local_addr().unwrap());
    let upstream = url.clone();
    let proxy = thread::spawn(move || {
        for stream in listener.incoming().take(4) {
            let mut stream = stream.unwrap();
            let input = read_http_json(&mut stream);
            let method = input["method"].as_str().unwrap();
            let result = rpc(&upstream, method, input["params"].clone());
            if method == "eth_sendRawTransaction" {
                // The upstream already mined it; keep the acknowledgement
                // connection open past the transport's five-second deadline.
                thread::sleep(Duration::from_secs(6));
                break;
            }
            let output = json!({"jsonrpc":"2.0","id":1,"result":result}).to_string();
            write!(stream,"HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",output.len(),output).unwrap();
        }
    });
    let mut cfg = config(&url, TransportMode::LocalBroadcast);
    cfg.submit_url = Some(proxy_url);
    let mut uncertain = EvmTransport::new(
        cfg,
        &code2,
        digest2,
        b.clone(),
        Some(signer),
        Arc::new(AtomicBool::new(false)),
    )
    .unwrap();
    let a = authorize(&p, digest2, &b).unwrap();
    let error = uncertain.execute(&a).unwrap_err();
    assert!(error.contains("NONCE_RESERVED"), "{error}");
    proxy.join().unwrap();
    assert!(uncertain.nonce_reserved(31337, 0));
    let attempt = uncertain.last_attempt().unwrap().clone();
    assert!(!attempt.acknowledged);
    assert_eq!(
        rpc(&url, "eth_getTransactionReceipt", json!([attempt.transaction_hash]))["status"],
        "0x1"
    );
    assert!(uncertain.execute(&a).unwrap_err().contains("NO_RESEND"));

    let evidence = json!({"scope":"owned loopback Anvil31337; genuine Morpho and MonkeyLiquidatorSwap; fixture tokens/oracle/DEX; no public signing",
        "artifact_hash":hex_string(&digest),"artifact_bytes_hex":hex_string(&code),"policy":policy,"authorization":authorized,
        "policy_hash":hex_string(&authorized.policy_hash),"request_hash":hex_string(&prepared.request_hash),
        "transaction_hash":transaction_hash,"receipt":receipt,"prepared_request":request,"before":before,"after":after,
        "native_fee_wei":(mined_gas*mined_price).to_string(),"fixture_source_manifest_sha256":manifest["source_manifest_sha256"],
        "signer":"ephemeral libsecp256k1 SDK; Alloy EIP1559/Signature encoding; no imported or persisted key",
        "negative_preflights":negatives,"kill_switch":"refused","calldata_tamper":"refused","unknown_extra_fee":"refused",
        "lost_acknowledgement":{"error":error,"attempt":attempt,"mined":true,"blind_resend":"refused"}});
    if let Some(path) = std::env::var_os("X3_NATIVE_PROOF_OUTPUT") {
        std::fs::write(path, serde_json::to_vec_pretty(&evidence).unwrap()).unwrap();
    }
    println!(
        "{}",
        json!({"local_native_proof":"passed","chain_id":31337,"transaction_hash":transaction_hash,
        "simulated_repaid":request.simulated_repaid.to_string(),"owner_delta":request.simulated_profit.to_string(),"gas_used":receipt["gasUsed"]})
    );
}

fn read_http_json(stream: &mut TcpStream) -> Value {
    use std::io::Read;
    stream.set_read_timeout(Some(Duration::from_secs(5))).unwrap();
    let mut reader = BufReader::new(stream);
    let mut line = String::new();
    let mut length = 0;
    loop {
        line.clear();
        assert!(reader.read_line(&mut line).unwrap() > 0, "incomplete HTTP request");
        if line == "\r\n" {
            break;
        }
        if let Some((name, value)) = line.split_once(':') {
            if name.eq_ignore_ascii_case("content-length") {
                length = value.trim().parse::<usize>().unwrap();
            }
        }
    }
    assert!(length > 0 && length < 1_000_000);
    let mut bytes = vec![0; length];
    reader.read_exact(&mut bytes).unwrap();
    serde_json::from_slice(&bytes).unwrap()
}
