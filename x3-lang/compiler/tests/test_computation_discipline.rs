//! Determinism and fixed-point discipline — PHASE 42 and 43, as a scan.
//!
//! Both phases state prohibitions rather than features: consensus-affecting
//! decisions must not depend on wall-clock jitter, randomness or floating-point
//! ambiguity, and money must be computed exactly. A property of the whole tree
//! cannot be proven by one feature's tests, so it is checked by reading the tree.
//!
//! The scan is deliberately narrow — it looks for *code shapes*, on lines that
//! are not comments — because a scan that cannot tell a comment from an
//! expression is a scan whose failures have to be argued about individually. If a
//! legitimate need for one of these appears, it should appear here as an
//! exemption with a reason, which is a conversation worth having.

use std::fs;
use std::path::{Path, PathBuf};

/// The two crates whose decisions reach consensus: the compiler and the VM.
fn scanned_roots() -> Vec<PathBuf> {
    let workspace = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("the compiler lives in a workspace");
    vec![workspace.join("compiler/src"), workspace.join("vm/src")]
}

/// Every non-comment line of every `.rs` file under the given roots.
fn code_lines() -> Vec<(PathBuf, usize, String)> {
    fn walk(dir: &Path, out: &mut Vec<(PathBuf, usize, String)>) {
        let Ok(entries) = fs::read_dir(dir) else {
            return;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                walk(&path, out);
            } else if path.extension().is_some_and(|extension| extension == "rs") {
                let Ok(text) = fs::read_to_string(&path) else {
                    continue;
                };
                for (index, line) in text.lines().enumerate() {
                    out.push((path.clone(), index + 1, line.to_string()));
                }
            }
        }
    }

    let mut out = Vec::new();
    for root in scanned_roots() {
        walk(&root, &mut out);
    }
    assert!(!out.is_empty(), "the scan found no sources to read");
    out
}

fn offenders(needles: &[&str], allowed: &[&str]) -> Vec<String> {
    let mut found = Vec::new();
    // Comment lines are read — a justification lives in one — but a comment is
    // never itself an offence: the scan is about what the code does, and
    // forbidding a word in a comment would make explaining the rule illegal.
    // A justification may sit on the line or anywhere in the comment block
    // directly above it, because a reason for a wire-format field does not fit on
    // one line and a scan that forced it to would be argued with rather than
    // satisfied.
    let mut comment_block = String::new();
    for (path, line_number, line) in code_lines() {
        if line.trim_start().starts_with("//") {
            comment_block.push_str(&line);
            comment_block.push('\n');
            continue;
        }
        let justified = allowed
            .iter()
            .any(|allowed| line.contains(allowed) || comment_block.contains(allowed));
        if !justified && needles.iter().any(|needle| line.contains(needle)) {
            found.push(format!("{}:{line_number}: {}", path.display(), line.trim()));
        }
        comment_block.clear();
    }
    found
}

#[test]
fn no_consensus_decision_reads_a_clock() {
    // PHASE 42: "consensus-affecting decisions must never depend on ... wall
    // clock jitter". There was exactly one read in these crates, a production
    // bridge backend deriving its VRF seed from `SystemTime::now()`, and it is
    // gone: a seed from the clock is reproducible by anyone who guesses the time.
    let found = offenders(&["SystemTime", "Instant::now", "chrono::"], &[]);
    assert!(
        found.is_empty(),
        "a clock read reaches consensus:\n{}",
        found.join("\n")
    );
}

#[test]
fn no_consensus_decision_reads_randomness() {
    // PHASE 42: "... randomness". Nothing here generates it; every "random"
    // value this language needs has to come from a declared input.
    let found = offenders(&["rand::", "thread_rng", "OsRng", "from_entropy"], &[]);
    assert!(found.is_empty(), "randomness reaches consensus:\n{}", found.join("\n"));
}

#[test]
fn no_money_or_policy_conversion_goes_through_a_float() {
    // PHASE 43: fixed-point financial math, and PHASE 42's "... floating-point
    // ambiguity". Every one of these was a real defect: a fractional amount
    // truncated to an integer, a `f64` comparison deciding whether a mainnet
    // program is rejected, a percentage formatted through a float, a fractional
    // timeout truncated to blocks.
    let exemption = "// float-exemption:";
    let found = offenders(
        &[
            "parse::<f64>",
            "parse::<f32>",
            "as f64",
            "as f32",
            ": f64",
            ": f32",
            "f64::",
            "f32::",
        ],
        &[exemption],
    );
    assert!(
        found.is_empty(),
        "a float converts money or policy:\n{}\n(add `{exemption} <reason>` to the line if it is \
         genuinely not a money or policy conversion)",
        found.join("\n")
    );
}

#[test]
fn the_scan_actually_reads_the_tree() {
    // Non-vacuous: a scan that silently reads nothing would pass every check
    // above. This one asserts it found the files and the code it should.
    let lines = code_lines();
    assert!(lines.len() > 1000, "the scan read only {} lines", lines.len());
    assert!(
        lines.iter().any(|(path, _, _)| path.ends_with("semantic.rs")),
        "the scan did not reach the compiler's most important file"
    );
    assert!(
        lines.iter().any(|(path, _, _)| path.ends_with("executor.rs")),
        "the scan did not reach the VM's executor"
    );
}
