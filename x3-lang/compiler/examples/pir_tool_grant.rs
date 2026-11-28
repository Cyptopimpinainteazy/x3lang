//! Offline operator tool: compile a narrow capability; never signs or runs it.
use std::{
    collections::BTreeSet,
    io::{self, Read},
};
use x3_lang_compiler::compile_capability_source;
fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect::<Vec<_>>().join("")
}
fn run() -> Result<(), Box<dyn std::error::Error>> {
    let mut args = std::env::args().skip(1);
    let scope = args.next().ok_or("missing scope")?;
    let now: u64 = args.next().ok_or("missing time")?.parse()?;
    if args.next().is_some() {
        return Err("unexpected argument".into());
    }
    let mut input = String::new();
    io::stdin().take(65537).read_to_string(&mut input)?;
    let compiled = compile_capability_source(&input, &scope, &BTreeSet::new(), now)?;
    if !matches!(compiled.grant.resource, x3_lang_ast::policy_ir::Resource::Tool { .. }) {
        return Err("tool resource required".into());
    }
    println!(
        "{}",
        serde_json::json!({"encoded":hex(&compiled.commitment.encoded),"source_sha256":hex(&compiled.source_sha256)})
    );
    Ok(())
}
fn main() {
    if let Err(e) = run() {
        eprintln!("capability refused: {e}");
        std::process::exit(2);
    }
}
