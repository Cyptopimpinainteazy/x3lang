//! Repo-source locks for the local forcefield.
//!
//! This module is `#[cfg(test)]`. It is absent from release WASM and it does
//! not change the deployed Borsh layout.

#[test]
fn deployed_source_has_no_bypass_and_no_verifier_setter() {
    let lib = include_str!("lib.rs");
    let mandate = include_str!("mandate.rs");
    let combined = format!("{lib}\n{mandate}");
    for name in [
        "fn owner_execute",
        "fn approve_pending",
        "fn set_manager",
        "fn set_policy",
    ] {
        assert!(!combined.contains(name), "{name} is present");
    }
    assert!(!has_token_withdraw(&combined), "fn withdraw is present");
    for name in [
        "pub fn revoke_mandate",
        "pub fn approve_intent",
        "pub fn withdraw_agent_storage",
    ] {
        assert!(mandate.contains(name), "{name} is missing");
    }
    assert!(lib.contains("pub const STATE_VERSION: u16 = 2;"));
    assert!(lib.contains(
        "pub const ONE_CLICK_MANAGER_PUBLIC_KEY: &str = \"ed25519:reYaWhvwu8Jzo3WUM3zhn6VrhuMEF4eADL17qtRVifc\";"
    ));
    assert!(lib.contains("has no setter"));
    assert!(lib.contains("No consumption occurs on a failed signature or rejected policy."));
    let approve = signature_of(mandate, "approve_intent");
    for word in ["recipient", "per_payment", "daily_cap", "refund_to"] {
        assert!(!approve.contains(word), "approve_intent gained {word}");
    }
    for name in ["agent_accept_quote", "agent_record_outcome"] {
        let signature = signature_of(mandate, name);
        for word in ["reason", "purpose", "decision_hash"] {
            assert!(!signature.contains(word), "{name} gained {word}");
        }
    }
}

fn has_token_withdraw(source: &str) -> bool {
    let needle = "fn withdraw";
    let mut start = 0;
    while let Some(index) = source[start..].find(needle) {
        let at = start + index;
        let after = &source[at + needle.len()..];
        if !after.starts_with('_') && after.trim_start().starts_with('(') {
            return true;
        }
        start = at + needle.len();
    }
    false
}

fn signature_of<'a>(source: &'a str, name: &str) -> &'a str {
    let marker = format!("pub fn {name}");
    let start = source.find(&marker).unwrap_or_else(|| panic!("missing {name}"));
    let paren = source[start..].find('(').expect("parameter list") + start;
    let mut depth = 0;
    for (index, character) in source[paren..].char_indices() {
        match character {
            '(' => depth += 1,
            ')' => {
                depth -= 1;
                if depth == 0 {
                    return &source[paren..paren + index + 1];
                }
            }
            _ => {}
        }
    }
    panic!("unclosed {name}");
}
