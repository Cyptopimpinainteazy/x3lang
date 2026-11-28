//! Unsigned native effect preflight. Public-chain signing is deliberately absent.
use clap::Parser;
use std::{
    fs,
    path::Path,
    sync::{atomic::AtomicBool, Arc},
    time::Duration,
};
use x3_lang_vm::{
    liquidation::{artifact_hash, hex_bytes, Batch},
    liquidation_evm::{EvmTransport, EvmTransportConfig, TransportMode},
    VMConfig, VM,
};

#[derive(Parser)]
#[command(about = "Execute a reviewed X3 liquidation artifact through native unsigned EVM preflight")]
struct Args {
    #[arg(long)]
    artifact: String,
    /// Independently approved Keccak-256 digest. Editing the artifact requires new approval.
    #[arg(long)]
    approved_artifact_hash: String,
    /// Bounded scanner batch. Costs and conversion rates are trusted host evidence.
    #[arg(long)]
    batch: String,
    /// Environment variable holding the read RPC URL (provider credentials are never logged).
    #[arg(long, default_value = "X3_LIQUIDATION_RPC_URL")]
    rpc_url_env: String,
    #[arg(long, default_value_t = 10)]
    timeout_seconds: u64,
}

fn bounded_read(path: &Path, max: u64) -> Result<Vec<u8>, String> {
    use std::io::Read;
    let file = fs::File::open(path).map_err(|_| "cannot open input file")?;
    let mut bytes = Vec::new();
    file.take(max + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| "cannot read input file")?;
    if bytes.len() as u64 > max {
        return Err("input exceeds size limit".into());
    }
    Ok(bytes)
}

fn run(args: Args) -> Result<(), String> {
    let code = bounded_read(Path::new(&args.artifact), 1_048_576)?;
    let approved = hex_bytes::<32>(&args.approved_artifact_hash)?;
    if approved != artifact_hash(&code) {
        return Err("X3_LIQUIDATION_ARTIFACT_NOT_APPROVED".into());
    }
    let batch: Batch =
        serde_json::from_slice(&bounded_read(Path::new(&args.batch), 4_194_304)?).map_err(|_| "invalid batch JSON")?;
    let read_url = std::env::var(&args.rpc_url_env).map_err(|_| "RPC URL environment variable is unset")?;
    let host = EvmTransport::new(
        EvmTransportConfig {
            read_url,
            submit_url: None,
            mode: TransportMode::PrepareOnly,
            timeout: Duration::from_secs(args.timeout_seconds),
        },
        &code,
        approved,
        batch,
        None,
        Arc::new(AtomicBool::new(false)),
    )?;
    let mut vm = VM::new(code, VMConfig::default(), 1_000_000);
    vm.set_liquidation_host(Box::new(host));
    vm.execute().map_err(|error| format!("{error:?}"))?;
    let receipt = vm
        .liquidation_receipt
        .ok_or("artifact contained no native liquidation effect")?;
    println!(
        "{}",
        serde_json::to_string_pretty(&receipt).map_err(|_| "cannot encode decision receipt")?
    );
    Ok(())
}

fn main() {
    if let Err(error) = run(Args::parse()) {
        eprintln!("{error}");
        std::process::exit(1);
    }
}
