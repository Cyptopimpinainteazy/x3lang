//! Offline, read-only NEAR testnet candidate verification; NEVER authorizes funds.
//! Usage: cargo run --bin check_candidate_epochs -- <rpc-view-1.json> <rpc-view-2.json> [...]
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::{env, path::PathBuf};
use x3_near_lightclient_approval_core::epoch::{parse_rpc_light_view, UntrustedEpochChain};

fn run() -> Result<(), String> {
    let mut paths = env::args_os().skip(1).map(PathBuf::from);
    let checkpoint = paths
        .next()
        .ok_or("usage: check_candidate_epochs <starting-candidate.json> <later-candidate.json> [...]".to_owned())?;
    let updates = paths.collect::<Vec<_>>();
    if updates.is_empty() || updates.len() > 128 {
        return Err("one to 128 subsequent candidate views required".to_owned());
    }
    let read = |p: &PathBuf| -> Result<(Value, String), String> {
        let bytes = std::fs::read(p).map_err(|e| format!("reading {}: {e}", p.display()))?;
        if bytes.len() > 1024 * 1024 {
            return Err("oversized RPC view".to_owned());
        }
        let digest = Sha256::digest(&bytes);
        let hex = digest.iter().map(|x| format!("{x:02x}")).collect::<String>();
        let parsed = serde_json::from_slice(&bytes).map_err(|e| format!("invalid RPC JSON: {e}"))?;
        Ok((parsed, hex))
    };
    let (raw, sha) = read(&checkpoint)?;
    let first = parse_rpc_light_view(&raw).map_err(str::to_owned)?;
    let mut chain = UntrustedEpochChain::bootstrap_rpc_candidate(&first).map_err(str::to_owned)?;
    println!(
        "CANDIDATE RPC checkpoint height={} sha256={} authenticated=false authorized_to_sign=false",
        first.height(),
        sha
    );
    for path in updates {
        let (raw, sha) = read(&path)?;
        let view = parse_rpc_light_view(&raw).map_err(str::to_owned)?;
        let result = chain.advance_candidate(&view).map_err(str::to_owned)?;
        println!("ACCEPTED RPC candidate height={} real_ed25519_signatures={} crossed_epoch={} sha256={} authenticated={} authorized_to_sign={}",
            result.height, result.valid_signatures, result.crossed_epoch, sha,
            result.authenticated, result.authorized_to_sign);
    }
    println!(
        "UNTRUSTED: no external checkpoint provenance or financial authority; this is cryptographic consistency only"
    );
    Ok(())
}

fn main() {
    if let Err(error) = run() {
        eprintln!("REJECTED UNTRUSTED NEAR EVIDENCE: {error}");
        std::process::exit(1);
    }
}
