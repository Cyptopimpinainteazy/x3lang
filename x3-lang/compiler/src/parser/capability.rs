//! Dedicated policy artifact entry point using the canonical X3 lexer/parser.
//! Executable programs cannot consume this declaration until runtime lowering exists.
use super::{parse_err_at, tokenize, Parser, Tok};
use std::collections::BTreeSet;
use x3_lang_ast::policy_ir::{AmountLimit, Grant, Requirement, Resource};
use x3_lang_common::X3Error;

fn error(p: &Parser<'_>, msg: &str) -> X3Error {
    parse_err_at(msg.into(), p.peek(), p.current_span())
}
fn keyword(p: &mut Parser<'_>, name: &str) -> Result<(), X3Error> {
    p.expect(Tok::Ident(name.into()), name)
}
fn string(p: &mut Parser<'_>) -> Result<String, X3Error> {
    match p.peek() {
        Tok::String_(s) => {
            p.advance();
            Ok(s)
        }
        _ => Err(error(p, "expected quoted string")),
    }
}
fn number(p: &mut Parser<'_>) -> Result<u128, X3Error> {
    match p.peek() {
        Tok::Int(n) => {
            p.advance();
            Ok(n)
        }
        _ => Err(error(p, "expected unsuffixed unsigned integer")),
    }
}
fn semi(p: &mut Parser<'_>) -> Result<(), X3Error> {
    p.expect(Tok::Semicolon, "expected ';'")
}
fn hex<const N: usize>(p: &mut Parser<'_>) -> Result<[u8; N], X3Error> {
    let s = string(p)?;
    if s.len() != N * 2 || !s.bytes().all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b)) {
        return Err(error(p, "invalid fixed-width lowercase hex"));
    }
    let mut out = [0; N];
    for (i, b) in out.iter_mut().enumerate() {
        *b = u8::from_str_radix(&s[2 * i..2 * i + 2], 16).map_err(|_| error(p, "invalid hash"))?;
    }
    Ok(out)
}

/// Parse exactly one tool or EVM capability with explicit no-asset authority.
/// No executable AST or implicit fallback is produced.
pub fn parse_capability_source(source: &str) -> Result<Grant, X3Error> {
    if source.is_empty() || source.len() > 65536 {
        return Err(X3Error::SemanticError {
            message: "capability source size outside 1..65536".into(),
            span: x3_lang_common::Span::DUMMY,
        });
    }
    let tokens = tokenize(source)?;
    let p = &mut Parser::new(&tokens);
    keyword(p, "capability")?;
    p.expect(Tok::LBrace, "expected capability body")?;
    keyword(p, "subject")?;
    let subject = string(p)?;
    semi(p)?;
    keyword(p, "parent")?;
    let parent = if p.peek() == Tok::Ident("none".into()) {
        p.advance();
        None
    } else {
        Some(string(p)?)
    };
    semi(p)?;
    keyword(p, "scope")?;
    let scope = string(p)?;
    semi(p)?;
    let resource = match p.expect_ident("resource")?.as_str() {
        "tool" => Resource::Tool {
            adapter: string(p)?,
            artifact_hash: hex(p)?,
        },
        "evm_call" => Resource::EvmCall {
            chain_id: u64::try_from(number(p)?).map_err(|_| error(p, "chain id exceeds u64"))?,
            executor: hex(p)?,
            target: hex(p)?,
            selector: hex(p)?,
            calldata_hash: hex(p)?,
        },
        _ => return Err(error(p, "unsupported capability resource")),
    };
    semi(p)?;
    keyword(p, "no_asset")?;
    semi(p)?;
    keyword(p, "expires")?;
    let expires_at = u64::try_from(number(p)?).map_err(|_| error(p, "expiry exceeds u64"))?;
    semi(p)?;
    keyword(p, "depth")?;
    let delegation_depth = u8::try_from(number(p)?).map_err(|_| error(p, "depth exceeds u8"))?;
    semi(p)?;
    keyword(p, "requirements")?;
    p.expect(Tok::LBracket, "expected requirement list")?;
    let mut requirements = BTreeSet::new();
    if p.peek() != Tok::RBracket {
        loop {
            let name = p.expect_ident("requirement")?;
            let value = match name.as_str() {
                "exact_code_identity" => Requirement::ExactCodeIdentity,
                "atomic_rollback" => Requirement::AtomicRollback,
                "authenticated_finality" => Requirement::AuthenticatedFinality,
                _ => return Err(error(p, "unsupported requirement")),
            };
            if !requirements.insert(value) {
                return Err(error(p, "duplicate requirement"));
            }
            if p.peek() != Tok::Comma {
                break;
            }
            p.advance();
        }
    }
    p.expect(Tok::RBracket, "expected end of requirement list")?;
    semi(p)?;
    p.expect(Tok::RBrace, "expected end of capability")?;
    if p.peek() != Tok::Eof {
        return Err(error(p, "expected end of source after one capability"));
    }
    Ok(Grant {
        subject,
        parent,
        scope,
        resource,
        amount: AmountLimit::NoAsset,
        expires_at,
        delegation_depth,
        requirements,
    })
}
