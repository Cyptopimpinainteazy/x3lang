use serde_json::{json, Value};
use sha3::{Digest, Keccak256};
use std::env;
use std::fs;

#[derive(Clone, Debug)]
enum Node {
    Leaf(Vec<u8>, Vec<u8>),
    Extension(Vec<u8>, Box<Node>),
    Branch(Vec<Option<Box<Node>>>, Option<Vec<u8>>),
}

fn main() {
    if let Err(err) = run() {
        eprintln!("{err}");
        std::process::exit(1);
    }
}

fn run() -> Result<(), String> {
    let args: Vec<String> = env::args().collect();
    let input_json = arg_value(&args, "--input-json");
    let rpc_url = arg_value(&args, "--rpc-url");
    let block = arg_value(&args, "--block");
    let tx_hash = normalize_hex(&arg_value(&args, "--tx-hash").ok_or("--tx-hash is required")?);
    let captured_via = rpc_url
        .clone()
        .or_else(|| input_json.clone())
        .unwrap_or_else(|| "local".to_string());

    let dataset = if let Some(input_json) = input_json {
        serde_json::from_str::<Value>(&fs::read_to_string(input_json).map_err(|err| err.to_string())?)
            .map_err(|err| err.to_string())?
    } else {
        let rpc_url = rpc_url.ok_or("--rpc-url or --input-json is required")?;
        let block = block.ok_or("--block is required with --rpc-url")?;
        fetch_block_receipts(&rpc_url, &block)?
    };

    let block = dataset.get("block").ok_or("dataset missing block")?;
    let receipts = dataset
        .get("receipts")
        .and_then(Value::as_array)
        .ok_or("dataset missing receipts array")?;
    let target_index = receipts
        .iter()
        .position(|receipt| {
            receipt
                .get("transactionHash")
                .and_then(Value::as_str)
                .map(normalize_hex)
                == Some(tx_hash.clone())
        })
        .ok_or("tx hash not found in receipts")?;

    let mut pairs = Vec::new();
    let mut receipt_rlps = Vec::new();
    for (index, receipt) in receipts.iter().enumerate() {
        let receipt_rlp = receipt_rlp(receipt)?;
        pairs.push((bytes_to_nibbles(&rlp_index(index)), receipt_rlp.clone()));
        receipt_rlps.push(receipt_rlp);
    }

    let trie = build_trie(&pairs);
    let root_rlp = encode_node(&trie);
    let computed_root = hex_prefixed(&keccak256(&root_rlp));
    let expected_root = normalize_hex(expect_str(block, "receiptsRoot")?);
    if computed_root != expected_root {
        return Err(format!(
            "computed receiptsRoot {computed_root} does not match block {expected_root}"
        ));
    }

    let header_rlp = header_rlp(block)?;
    let header_hash = hex_prefixed(&keccak256(&header_rlp));
    let expected_header = normalize_hex(expect_str(block, "hash")?);
    if header_hash != expected_header {
        return Err(format!(
            "computed header hash {header_hash} does not match block {expected_header}"
        ));
    }

    let receipt_key = rlp_index(target_index);
    let mut trie_nodes = Vec::new();
    collect_proof_nodes(&trie, &bytes_to_nibbles(&receipt_key), &mut trie_nodes);
    let target = &receipts[target_index];
    let mut archive = json!({
        "header_hash": header_hash,
        "receipts_root": expected_root,
        "rlp_header": hex_prefixed(&header_rlp),
        "receipt_key": hex_prefixed(&receipt_key),
        "receipt_rlp": hex_prefixed(&receipt_rlps[target_index]),
        "receipt_hash": hex_prefixed(&keccak256(&receipt_rlps[target_index])),
        "trie_nodes": trie_nodes.iter().map(|node| hex_prefixed(node)).collect::<Vec<_>>(),
        "source": {
            "network": "ethereum-mainnet",
            "block_number": parse_hex_u64(expect_str(block, "number")?),
            "block_hash": expected_header,
            "transaction_hash": normalize_hex(expect_str(target, "transactionHash")?),
            "transaction_index": expect_str(target, "transactionIndex")?,
            "captured_via": captured_via,
        }
    });
    if let Some(log) = first_transfer_log(target) {
        archive["log"] = log.clone();
    }

    println!(
        "{}",
        serde_json::to_string_pretty(&archive).map_err(|err| err.to_string())?
    );
    Ok(())
}

fn fetch_block_receipts(rpc_url: &str, block: &str) -> Result<Value, String> {
    let block_param = if block.starts_with("0x") {
        block.to_string()
    } else {
        format!("0x{:x}", block.parse::<u64>().map_err(|err| err.to_string())?)
    };
    let block_value = rpc(rpc_url, "eth_getBlockByNumber", json!([block_param, false]))?;
    let txs = block_value
        .get("transactions")
        .and_then(Value::as_array)
        .ok_or("block response missing transactions")?;
    let mut receipts = Vec::with_capacity(txs.len());
    for tx in txs {
        receipts.push(rpc(
            rpc_url,
            "eth_getTransactionReceipt",
            json!([tx.as_str().ok_or("transaction hash is not a string")?]),
        )?);
    }
    Ok(json!({ "block": block_value, "receipts": receipts }))
}

fn rpc(rpc_url: &str, method: &str, params: Value) -> Result<Value, String> {
    let client = reqwest::blocking::Client::builder()
        .user_agent("x3-proof-fixture-generator/1.0")
        .build()
        .map_err(|err| err.to_string())?;
    let response: Value = client
        .post(rpc_url)
        .json(&json!({"jsonrpc":"2.0","id":1,"method":method,"params":params}))
        .send()
        .map_err(|err| err.to_string())?
        .json()
        .map_err(|err| err.to_string())?;
    if let Some(error) = response.get("error") {
        return Err(format!("{method} RPC error: {error}"));
    }
    response
        .get("result")
        .cloned()
        .ok_or_else(|| format!("{method} response missing result"))
}

fn arg_value(args: &[String], name: &str) -> Option<String> {
    args.windows(2)
        .find(|window| window[0] == name)
        .map(|window| window[1].clone())
}

fn receipt_rlp(receipt: &Value) -> Result<Vec<u8>, String> {
    let status_or_root = if let Some(status) = receipt.get("status").and_then(Value::as_str) {
        uint_rlp_hex(status)
    } else {
        Ok(rlp_bytes(&hex_to_bytes(expect_str(receipt, "root")?)?))
    };
    let payload = rlp_list(vec![
        status_or_root?,
        uint_rlp_hex(expect_str(receipt, "cumulativeGasUsed")?)?,
        rlp_bytes(&hex_to_bytes(expect_str(receipt, "logsBloom")?)?),
        rlp_list(
            receipt
                .get("logs")
                .and_then(Value::as_array)
                .ok_or("receipt logs missing")?
                .iter()
                .map(rlp_log)
                .collect::<Result<Vec<_>, _>>()?,
        ),
    ]);
    let receipt_type = parse_hex_u64(receipt.get("type").and_then(Value::as_str).unwrap_or("0x0"));
    if receipt_type == 0 {
        Ok(payload)
    } else {
        Ok([vec![receipt_type as u8], payload].concat())
    }
}

fn rlp_log(log: &Value) -> Result<Vec<u8>, String> {
    let topics = log
        .get("topics")
        .and_then(Value::as_array)
        .ok_or("log topics missing")?
        .iter()
        .map(|topic| hex_to_bytes(topic.as_str().ok_or("topic is not a string")?).map(|bytes| rlp_bytes(&bytes)))
        .collect::<Result<Vec<_>, _>>()?;
    Ok(rlp_list(vec![
        rlp_bytes(&hex_to_bytes(expect_str(log, "address")?)?),
        rlp_list(topics),
        rlp_bytes(&hex_to_bytes(expect_str(log, "data")?)?),
    ]))
}

fn header_rlp(block: &Value) -> Result<Vec<u8>, String> {
    let mut fields = vec![
        rlp_bytes(&hex_to_bytes(expect_str(block, "parentHash")?)?),
        rlp_bytes(&hex_to_bytes(expect_str(block, "sha3Uncles")?)?),
        rlp_bytes(&hex_to_bytes(expect_str(block, "miner")?)?),
        rlp_bytes(&hex_to_bytes(expect_str(block, "stateRoot")?)?),
        rlp_bytes(&hex_to_bytes(expect_str(block, "transactionsRoot")?)?),
        rlp_bytes(&hex_to_bytes(expect_str(block, "receiptsRoot")?)?),
        rlp_bytes(&hex_to_bytes(expect_str(block, "logsBloom")?)?),
        uint_rlp_hex(expect_str(block, "difficulty")?)?,
        uint_rlp_hex(expect_str(block, "number")?)?,
        uint_rlp_hex(expect_str(block, "gasLimit")?)?,
        uint_rlp_hex(expect_str(block, "gasUsed")?)?,
        uint_rlp_hex(expect_str(block, "timestamp")?)?,
        rlp_bytes(&hex_to_bytes(expect_str(block, "extraData")?)?),
        rlp_bytes(&hex_to_bytes(expect_str(block, "mixHash")?)?),
        rlp_bytes(&hex_to_bytes(expect_str(block, "nonce")?)?),
    ];
    if let Some(base_fee) = block.get("baseFeePerGas").and_then(Value::as_str) {
        fields.push(uint_rlp_hex(base_fee)?);
    }
    if let Some(withdrawals_root) = block.get("withdrawalsRoot").and_then(Value::as_str) {
        fields.push(rlp_bytes(&hex_to_bytes(withdrawals_root)?));
    }
    if let Some(blob_gas_used) = block.get("blobGasUsed").and_then(Value::as_str) {
        fields.push(uint_rlp_hex(blob_gas_used)?);
    }
    if let Some(excess_blob_gas) = block.get("excessBlobGas").and_then(Value::as_str) {
        fields.push(uint_rlp_hex(excess_blob_gas)?);
    }
    if let Some(parent_beacon_block_root) = block.get("parentBeaconBlockRoot").and_then(Value::as_str) {
        fields.push(rlp_bytes(&hex_to_bytes(parent_beacon_block_root)?));
    }
    Ok(rlp_list(fields))
}

fn first_transfer_log(receipt: &Value) -> Option<&Value> {
    receipt.get("logs").and_then(Value::as_array)?.iter().find(|log| {
        log.get("topics")
            .and_then(Value::as_array)
            .and_then(|topics| topics.first())
            .and_then(Value::as_str)
            .map(|topic| {
                topic.eq_ignore_ascii_case("0xddf252ad1be2c89b69c2b068fc378daa952ba7f163c4a11628f55a4df523b3ef")
            })
            .unwrap_or(false)
    })
}

fn build_trie(pairs: &[(Vec<u8>, Vec<u8>)]) -> Node {
    if pairs.len() == 1 {
        return Node::Leaf(pairs[0].0.clone(), pairs[0].1.clone());
    }
    let prefix = common_prefix(pairs);
    if !prefix.is_empty() {
        let stripped = pairs
            .iter()
            .map(|(key, value)| (key[prefix.len()..].to_vec(), value.clone()))
            .collect::<Vec<_>>();
        return Node::Extension(prefix, Box::new(build_trie(&stripped)));
    }
    let mut children: Vec<Option<Box<Node>>> = (0..16).map(|_| None).collect();
    let mut value = None;
    for nibble in 0..16u8 {
        let group = pairs
            .iter()
            .filter(|(key, _)| !key.is_empty() && key[0] == nibble)
            .map(|(key, value)| (key[1..].to_vec(), value.clone()))
            .collect::<Vec<_>>();
        if !group.is_empty() {
            children[nibble as usize] = Some(Box::new(build_trie(&group)));
        }
    }
    for (key, branch_value) in pairs {
        if key.is_empty() {
            value = Some(branch_value.clone());
        }
    }
    Node::Branch(children, value)
}

fn common_prefix(pairs: &[(Vec<u8>, Vec<u8>)]) -> Vec<u8> {
    let mut out = Vec::new();
    let min_len = pairs.iter().map(|(key, _)| key.len()).min().unwrap_or(0);
    'outer: for index in 0..min_len {
        let nibble = pairs[0].0[index];
        for (key, _) in pairs {
            if key[index] != nibble {
                break 'outer;
            }
        }
        out.push(nibble);
    }
    out
}

fn encode_node(node: &Node) -> Vec<u8> {
    match node {
        Node::Leaf(path, value) => rlp_list(vec![rlp_bytes(&compact_path(path, true)), rlp_bytes(value)]),
        Node::Extension(path, child) => {
            let child = encode_node(child);
            rlp_list(vec![rlp_bytes(&compact_path(path, false)), child_ref(&child)])
        }
        Node::Branch(children, value) => {
            let mut fields = Vec::new();
            for child in children {
                if let Some(child) = child {
                    fields.push(child_ref(&encode_node(child)));
                } else {
                    fields.push(rlp_bytes(&[]));
                }
            }
            fields.push(rlp_bytes(value.as_deref().unwrap_or(&[])));
            rlp_list(fields)
        }
    }
}

fn child_ref(encoded: &[u8]) -> Vec<u8> {
    if encoded.len() < 32 {
        encoded.to_vec()
    } else {
        rlp_bytes(&keccak256(encoded))
    }
}

fn collect_proof_nodes(node: &Node, target: &[u8], out: &mut Vec<Vec<u8>>) {
    out.push(encode_node(node));
    match node {
        Node::Leaf(_, _) => {}
        Node::Extension(path, child) => {
            if target.starts_with(path) {
                collect_proof_nodes(child, &target[path.len()..], out);
            }
        }
        Node::Branch(children, _) => {
            if let Some(nibble) = target.first() {
                if let Some(child) = &children[*nibble as usize] {
                    collect_proof_nodes(child, &target[1..], out);
                }
            }
        }
    }
}

fn compact_path(nibbles: &[u8], leaf: bool) -> Vec<u8> {
    let base = if leaf { 2 } else { 0 };
    let mut prefixed = if nibbles.len() % 2 == 1 {
        vec![base + 1]
    } else {
        vec![base, 0]
    };
    prefixed.extend_from_slice(nibbles);
    nibbles_to_bytes(&prefixed)
}

fn bytes_to_nibbles(bytes: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(bytes.len() * 2);
    for byte in bytes {
        out.push(byte >> 4);
        out.push(byte & 0x0f);
    }
    out
}

fn nibbles_to_bytes(nibbles: &[u8]) -> Vec<u8> {
    nibbles
        .chunks(2)
        .map(|chunk| (chunk[0] << 4) | chunk.get(1).copied().unwrap_or(0))
        .collect()
}

fn rlp_index(index: usize) -> Vec<u8> {
    if index == 0 {
        return rlp_bytes(&[]);
    }
    let mut bytes = Vec::new();
    let mut value = index;
    while value > 0 {
        bytes.push((value & 0xff) as u8);
        value >>= 8;
    }
    bytes.reverse();
    rlp_bytes(&bytes)
}

fn rlp_bytes(bytes: &[u8]) -> Vec<u8> {
    if bytes.len() == 1 && bytes[0] < 0x80 {
        return bytes.to_vec();
    }
    let mut out = rlp_len(bytes.len(), 0x80);
    out.extend_from_slice(bytes);
    out
}

fn rlp_list(items: Vec<Vec<u8>>) -> Vec<u8> {
    let payload = items.concat();
    let mut out = rlp_len(payload.len(), 0xc0);
    out.extend_from_slice(&payload);
    out
}

fn rlp_len(len: usize, offset: u8) -> Vec<u8> {
    if len < 56 {
        return vec![offset + len as u8];
    }
    let len_bytes = len.to_be_bytes();
    let first_nonzero = len_bytes
        .iter()
        .position(|byte| *byte != 0)
        .unwrap_or(len_bytes.len() - 1);
    let encoded = &len_bytes[first_nonzero..];
    let mut out = vec![offset + 55 + encoded.len() as u8];
    out.extend_from_slice(encoded);
    out
}

fn uint_rlp_hex(value: &str) -> Result<Vec<u8>, String> {
    let mut bytes = hex_to_bytes(value)?;
    while bytes.first() == Some(&0) {
        bytes.remove(0);
    }
    Ok(rlp_bytes(&bytes))
}

fn hex_to_bytes(value: &str) -> Result<Vec<u8>, String> {
    let mut value = value.trim_start_matches("0x").to_string();
    if value.is_empty() {
        return Ok(Vec::new());
    }
    if value.len() % 2 == 1 {
        value = format!("0{value}");
    }
    let mut out = Vec::with_capacity(value.len() / 2);
    for index in (0..value.len()).step_by(2) {
        out.push(u8::from_str_radix(&value[index..index + 2], 16).map_err(|err| err.to_string())?);
    }
    Ok(out)
}

fn hex_prefixed(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(2 + bytes.len() * 2);
    out.push_str("0x");
    for byte in bytes {
        out.push_str(&format!("{byte:02x}"));
    }
    out
}

fn keccak256(bytes: &[u8]) -> [u8; 32] {
    let digest = Keccak256::digest(bytes);
    let mut out = [0u8; 32];
    out.copy_from_slice(&digest);
    out
}

fn normalize_hex(value: &str) -> String {
    format!("0x{}", value.trim_start_matches("0x").to_ascii_lowercase())
}

fn parse_hex_u64(value: &str) -> u64 {
    u64::from_str_radix(value.trim_start_matches("0x"), 16).unwrap_or(0)
}

fn expect_str<'a>(value: &'a Value, key: &str) -> Result<&'a str, String> {
    value
        .get(key)
        .and_then(Value::as_str)
        .ok_or_else(|| format!("{key} missing or not a string"))
}
