//! Offline EVM proposal exporter. No wallet, RPC, signing or broadcasting.
use std::io::{self, Read};
use x3_lang_compiler::evm_capability::compile_evm_action_source;
fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect::<Vec<_>>().join("")
}
fn run() -> Result<(), Box<dyn std::error::Error>> {
    let mut args = std::env::args().skip(1);
    let scope = args.next().ok_or("missing scope")?;
    let now: u64 = args.next().ok_or("missing time")?.parse()?;
    let version: u64 = args.next().ok_or("missing policy version")?.parse()?;
    let nonce: u64 = args.next().ok_or("missing nonce")?.parse()?;
    let raw = args.next().ok_or("missing calldata")?;
    if args.next().is_some() {
        return Err("unexpected argument".into());
    }
    let s = raw.strip_prefix("0x").ok_or("missing calldata prefix")?;
    if s.len() > 1032 || s.len() % 2 != 0 || !s.bytes().all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b)) {
        return Err("invalid calldata hex".into());
    }
    let data = (0..s.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&s[i..i + 2], 16))
        .collect::<Result<Vec<_>, _>>()?;
    let mut source = String::new();
    io::stdin().take(65537).read_to_string(&mut source)?;
    let p = compile_evm_action_source(&source, &scope, now, version, nonce, &data)?;
    println!(
        "{}",
        serde_json::json!({"abi":format!("0x{}",hex(&p.abi)),"action_id":format!("0x{}",hex(&p.action_id)),"policy_hash":format!("0x{}",hex(&p.policy_hash))})
    );
    Ok(())
}
fn main() {
    if let Err(e) = run() {
        eprintln!("proposal refused: {e}");
        std::process::exit(2);
    }
}
