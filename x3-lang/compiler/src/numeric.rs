//! X3Lang 1.0 numeric literal and direct-call compatibility policy.
//!
//! Rules (RFC t5-6, amended 2026-09-26):
//! - an unsuffixed integer literal takes the integer type its parameter requires, provided its
//!   value fits that type — so `1` satisfies `u64`, `i64` and `u32`, while `300` does not satisfy
//!   `u8` and `-1` (negation of a literal) satisfies no unsigned type;
//! - a suffixed literal has exactly its suffix's type;
//! - integer widths and signedness of *typed* values must match exactly: there is no implicit
//!   widening, narrowing or signed/unsigned conversion;
//! - direct call-site incompatibility reports `X3E0202`.

use std::collections::HashMap;

use x3_lang_ast::ast::{Block, Expression, Item, LiteralExpr, Program, Statement, TypeExpr};
use x3_lang_common::{IntSuffix, Span, UnOp};

use crate::diagnostic::{CompilerDiagnostic, DiagnosticCode};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum NumericKind {
    U8,
    U16,
    U32,
    U64,
    U128,
    U256,
    Usize,
    I8,
    I16,
    I32,
    I64,
    I128,
    I256,
    Isize,
}

/// Verify numeric compatibility for direct calls to functions declared in the
/// same program. This is intentionally narrow: it establishes the X3Lang 1.0
/// literal/coercion contract without pretending to be a complete type checker.
pub fn verify_numeric_policy(program: &Program) -> Vec<CompilerDiagnostic> {
    let signatures = function_signatures(program);
    let mut diagnostics = Vec::new();

    for item in &program.items {
        if let Item::Function(function) = &item.node {
            verify_block(&function.body, &signatures, &mut diagnostics);
        }
    }

    diagnostics
}

fn function_signatures(program: &Program) -> HashMap<String, Vec<Option<NumericKind>>> {
    let mut signatures = HashMap::new();
    for item in &program.items {
        if let Item::Function(function) = &item.node {
            let params = function
                .params
                .iter()
                .map(|param| param.ty.as_ref().and_then(numeric_type))
                .collect();
            signatures.insert(function.name.as_str().to_owned(), params);
        }
    }
    signatures
}

fn numeric_type(ty: &TypeExpr) -> Option<NumericKind> {
    let name = match ty {
        TypeExpr::Primitive(name) => name.as_str(),
        TypeExpr::Path(parts) if parts.len() == 1 => parts[0].as_str(),
        _ => return None,
    };

    match name {
        "u8" => Some(NumericKind::U8),
        "u16" => Some(NumericKind::U16),
        "u32" => Some(NumericKind::U32),
        "u64" => Some(NumericKind::U64),
        "u128" => Some(NumericKind::U128),
        "u256" => Some(NumericKind::U256),
        "usize" => Some(NumericKind::Usize),
        "i8" => Some(NumericKind::I8),
        "i16" => Some(NumericKind::I16),
        "i32" => Some(NumericKind::I32),
        "i64" => Some(NumericKind::I64),
        "i128" => Some(NumericKind::I128),
        "i256" => Some(NumericKind::I256),
        "isize" => Some(NumericKind::Isize),
        _ => None,
    }
}

fn literal_numeric_kind(suffix: Option<IntSuffix>) -> NumericKind {
    match suffix {
        None | Some(IntSuffix::U64) => NumericKind::U64,
        Some(IntSuffix::U8) => NumericKind::U8,
        Some(IntSuffix::U16) => NumericKind::U16,
        Some(IntSuffix::U32) => NumericKind::U32,
        Some(IntSuffix::U128) => NumericKind::U128,
        Some(IntSuffix::U256) => NumericKind::U256,
        Some(IntSuffix::Usize) => NumericKind::Usize,
        Some(IntSuffix::I8) => NumericKind::I8,
        Some(IntSuffix::I16) => NumericKind::I16,
        Some(IntSuffix::I32) => NumericKind::I32,
        Some(IntSuffix::I64) => NumericKind::I64,
        Some(IntSuffix::I128) => NumericKind::I128,
        Some(IntSuffix::I256) => NumericKind::I256,
        Some(IntSuffix::Isize) => NumericKind::Isize,
    }
}

fn negated_kind(kind: NumericKind) -> NumericKind {
    match kind {
        NumericKind::U8 => NumericKind::I8,
        NumericKind::U16 => NumericKind::I16,
        NumericKind::U32 => NumericKind::I32,
        NumericKind::U64 => NumericKind::I64,
        NumericKind::U128 => NumericKind::I128,
        NumericKind::U256 => NumericKind::I256,
        NumericKind::Usize => NumericKind::Isize,
        signed => signed,
    }
}

/// The value of an unsuffixed integer literal, or of the negation of one.
fn unsuffixed_literal_value(expr: &Expression) -> Option<i128> {
    match expr {
        Expression::Literal(LiteralExpr::Int {
            value, suffix: None, ..
        }) => i128::try_from(*value).ok(),
        Expression::Unary { op: UnOp::Neg, expr } => unsuffixed_literal_value(expr).map(|v| -v),
        _ => None,
    }
}

/// Whether `value` is representable in `kind`. `usize`/`isize` are 64-bit, the X3VM's width.
fn fits(value: i128, kind: NumericKind) -> bool {
    let (min, max): (i128, i128) = match kind {
        NumericKind::U8 => (0, u8::MAX.into()),
        NumericKind::U16 => (0, u16::MAX.into()),
        NumericKind::U32 => (0, u32::MAX.into()),
        NumericKind::U64 | NumericKind::Usize => (0, u64::MAX.into()),
        NumericKind::U128 | NumericKind::U256 => (0, i128::MAX),
        NumericKind::I8 => (i8::MIN.into(), i8::MAX.into()),
        NumericKind::I16 => (i16::MIN.into(), i16::MAX.into()),
        NumericKind::I32 => (i32::MIN.into(), i32::MAX.into()),
        NumericKind::I64 | NumericKind::Isize => (i64::MIN.into(), i64::MAX.into()),
        NumericKind::I128 | NumericKind::I256 => (i128::MIN, i128::MAX),
    };
    (min..=max).contains(&value)
}

/// Whether the argument `arg` satisfies an integer parameter of type `param`, if the argument's
/// type is decided by its own syntax (a literal); `None` when it is not.
fn argument_satisfies(arg: &Expression, param: NumericKind) -> Option<bool> {
    if let Some(value) = unsuffixed_literal_value(arg) {
        return Some(fits(value, param));
    }
    expression_numeric_kind(arg).map(|actual| actual == param)
}

fn expression_numeric_kind(expr: &Expression) -> Option<NumericKind> {
    match expr {
        Expression::Literal(LiteralExpr::Int { suffix, .. }) => Some(literal_numeric_kind(*suffix)),
        Expression::Unary { op: UnOp::Neg, expr } => expression_numeric_kind(expr).map(negated_kind),
        _ => None,
    }
}

fn verify_block(
    block: &Block,
    signatures: &HashMap<String, Vec<Option<NumericKind>>>,
    diagnostics: &mut Vec<CompilerDiagnostic>,
) {
    for stmt in &block.stmts {
        verify_statement(stmt, signatures, diagnostics);
    }
}

fn verify_statement(
    stmt: &Statement,
    signatures: &HashMap<String, Vec<Option<NumericKind>>>,
    diagnostics: &mut Vec<CompilerDiagnostic>,
) {
    match stmt {
        Statement::Let { expr: Some(expr), .. } | Statement::Expr(expr) => {
            verify_expression(expr, signatures, diagnostics)
        }
        Statement::Return(Some(expr)) => verify_expression(expr, signatures, diagnostics),
        Statement::If {
            cond,
            then_block,
            else_block,
        } => {
            verify_expression(cond, signatures, diagnostics);
            verify_block(then_block, signatures, diagnostics);
            if let Some(block) = else_block {
                verify_block(block, signatures, diagnostics);
            }
        }
        Statement::While { cond, body } => {
            verify_expression(cond, signatures, diagnostics);
            verify_block(body, signatures, diagnostics);
        }
        Statement::For { iterable, body, .. } => {
            verify_expression(iterable, signatures, diagnostics);
            verify_block(body, signatures, diagnostics);
        }
        Statement::Loop(block) => verify_block(block, signatures, diagnostics),
        Statement::Atomic(atomic) => verify_block(&atomic.body, signatures, diagnostics),
        _ => {}
    }
}

fn verify_expression(
    expr: &Expression,
    signatures: &HashMap<String, Vec<Option<NumericKind>>>,
    diagnostics: &mut Vec<CompilerDiagnostic>,
) {
    match expr {
        Expression::Call { callee, args } => {
            if let Expression::Ident(name) = callee.as_ref() {
                if let Some(params) = signatures.get(name.as_str()) {
                    for (index, arg) in args.iter().enumerate() {
                        if let Some(param) = params.get(index).and_then(|kind| *kind) {
                            if let Some(satisfied) = argument_satisfies(arg, param) {
                                if !satisfied {
                                    diagnostics.push(
                                        CompilerDiagnostic::error(
                                            DiagnosticCode::ArgumentTypeMismatch,
                                            format!("argument {} to `{}` has incompatible integer type", index + 1, name.as_str()),
                                            Span::DUMMY,
                                        )
                                        .with_help("an unsuffixed literal takes its parameter's integer type only when its value fits that type; typed integers require exact compatibility, with no implicit widening, narrowing, or signed/unsigned coercion"),
                                    );
                                }
                            }
                        }
                        verify_expression(arg, signatures, diagnostics);
                    }
                    return;
                }
            }
            for arg in args {
                verify_expression(arg, signatures, diagnostics);
            }
        }
        Expression::Binary { lhs, rhs, .. } => {
            verify_expression(lhs, signatures, diagnostics);
            verify_expression(rhs, signatures, diagnostics);
        }
        Expression::Unary { expr, .. } | Expression::Await(expr) | Expression::Async(expr) | Expression::Try(expr) => {
            verify_expression(expr, signatures, diagnostics)
        }
        Expression::MethodCall { receiver, args, .. } => {
            verify_expression(receiver, signatures, diagnostics);
            for arg in args {
                verify_expression(arg, signatures, diagnostics);
            }
        }
        Expression::FieldAccess { target, .. } => verify_expression(target, signatures, diagnostics),
        Expression::Index { target, index } => {
            verify_expression(target, signatures, diagnostics);
            verify_expression(index, signatures, diagnostics);
        }
        Expression::IfExpr {
            cond,
            then_block,
            else_block,
        } => {
            verify_expression(cond, signatures, diagnostics);
            verify_block(then_block, signatures, diagnostics);
            if let Some(block) = else_block {
                verify_block(block, signatures, diagnostics);
            }
        }
        Expression::BlockExpr(block) => verify_block(block, signatures, diagnostics),
        Expression::Closure { body, .. } => verify_expression(body, signatures, diagnostics),
        Expression::Match { expr, arms } => {
            verify_expression(expr, signatures, diagnostics);
            for (_, arm) in arms {
                verify_expression(arm, signatures, diagnostics);
            }
        }
        Expression::Atomic(atomic) => verify_block(&atomic.body, signatures, diagnostics),
        Expression::Literal(_) | Expression::Ident(_) => {}
    }
}
