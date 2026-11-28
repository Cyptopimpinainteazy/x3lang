//! Offline capability compilation. This is not execution authorization.
use crate::parser::capability::parse_capability_source;
use crate::policy_encoding::{encode_grant, Commitment};
use sha2::{Digest, Sha256};
use std::collections::BTreeSet;
use x3_lang_ast::policy_ir::{check_support, Grant, Requirement, Resource};
use x3_lang_common::{Span, X3Error};

#[derive(Debug)]
pub struct CompiledCapability {
    pub grant: Grant,
    pub commitment: Commitment,
    pub source_sha256: [u8; 32],
}

/// Use a trusted independently selected scope, support set and clock.
/// Support describes adapter capability, not evidence that execution satisfied it.
/// Parent identity, actual delegation and authorization remain runtime responsibilities.
pub fn compile_capability_source(
    source: &str,
    trusted_scope: &str,
    supported: &BTreeSet<Requirement>,
    now: u64,
) -> Result<CompiledCapability, X3Error> {
    let grant = parse_capability_source(source)?;
    let refuse = |message: String| X3Error::SemanticError {
        message,
        span: Span::DUMMY,
    };
    if matches!(&grant.resource, Resource::NearBscWindow { .. }) {
        return Err(refuse(
            "settlement requires blocked plan compiler; native verifier unavailable".into(),
        ));
    }
    if grant.scope != trusted_scope {
        return Err(refuse("capability scope mismatch".into()));
    }
    check_support(&grant, supported, now).map_err(|e| refuse(format!("capability refused: {e:?}")))?;
    let commitment = encode_grant(&grant).map_err(|e| refuse(format!("capability encoding refused: {e:?}")))?;
    Ok(CompiledCapability {
        grant,
        commitment,
        source_sha256: Sha256::digest(source.as_bytes()).into(),
    })
}
