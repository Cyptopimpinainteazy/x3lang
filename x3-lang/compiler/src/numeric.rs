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

use x3_lang_ast::ast::{Block, Expression, Item, LiteralExpr, Parameter, Pattern, Program, Statement, TypeExpr};
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
            let mut bindings = Bindings::new();
            bind_parameters(&function.params, &mut bindings);
            verify_block(&function.body, &signatures, &bindings, &mut diagnostics);
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

/// Preserve magnitude, sign and suffix until a destination type is known.
fn literal_value(expr: &Expression) -> Option<(bool, u128, Option<IntSuffix>)> {
    match expr {
        Expression::Literal(LiteralExpr::Int { value, suffix, .. }) => Some((false, *value, *suffix)),
        Expression::Unary { op: UnOp::Neg, expr } => {
            literal_value(expr).map(|(negative, magnitude, suffix)| (!negative, magnitude, suffix))
        }
        _ => None,
    }
}

/// Check the sign and magnitude without narrowing through `i128`. The magnitude of
/// `i128::MIN` and the upper half of `u128` cannot be represented as positive `i128`s.
/// `usize`/`isize` are 64-bit, the X3VM's width.
fn fits((negative, magnitude): (bool, u128), kind: NumericKind) -> bool {
    let (signed, bits) = match kind {
        NumericKind::U8 => (false, 8),
        NumericKind::U16 => (false, 16),
        NumericKind::U32 => (false, 32),
        NumericKind::U64 | NumericKind::Usize => (false, 64),
        NumericKind::U128 => (false, 128),
        NumericKind::U256 => (false, 256),
        NumericKind::I8 => (true, 8),
        NumericKind::I16 => (true, 16),
        NumericKind::I32 => (true, 32),
        NumericKind::I64 | NumericKind::Isize => (true, 64),
        NumericKind::I128 => (true, 128),
        NumericKind::I256 => (true, 256),
    };
    if negative && magnitude != 0 && !signed {
        return false;
    }
    if bits > 128 || (!signed && bits == 128) {
        return true;
    }
    let max = if signed {
        let minimum_magnitude = 1u128 << (bits - 1);
        if negative {
            minimum_magnitude
        } else {
            minimum_magnitude - 1
        }
    } else {
        (1u128 << bits) - 1
    };
    magnitude <= max
}

/// A signed minimum may be written as a single negated literal. Further negations
/// must also fit: a later negation cannot hide an overflowing intermediate value.
fn literal_fits(expr: &Expression, kind: NumericKind) -> bool {
    let Some((negative, magnitude, _)) = literal_value(expr) else {
        return false;
    };
    if !fits((negative, magnitude), kind) {
        return false;
    }
    match expr {
        Expression::Unary { op: UnOp::Neg, expr } if !matches!(expr.as_ref(), Expression::Literal(_)) => {
            literal_fits(expr, kind)
        }
        _ => true,
    }
}

// `None` is a real shadowing binding whose type is outside this numeric checker.
type Bindings = HashMap<String, Option<NumericKind>>;

fn bind_parameters(params: &[Parameter], bindings: &mut Bindings) {
    for param in params {
        if let Some(name) = &param.name {
            bindings.insert(name.as_str().to_owned(), param.ty.as_ref().and_then(numeric_type));
        }
    }
}

fn bind_pattern(pattern: &Pattern, bindings: &mut Bindings) {
    match pattern {
        Pattern::Ident(name) => {
            bindings.insert(name.as_str().to_owned(), None);
        }
        Pattern::Tuple(parts) => {
            for part in parts {
                bind_pattern(part, bindings);
            }
        }
        Pattern::Wildcard | Pattern::Literal(_) => {}
    }
}

/// Only unsuffixed literals take the parameter's type. Named values and suffixed
/// literals retain their type, even when their value would fit a different width.
fn argument_satisfies(arg: &Expression, param: NumericKind, bindings: &Bindings) -> Option<bool> {
    if let Some((_, _, None)) = literal_value(arg) {
        return Some(literal_fits(arg, param));
    }
    expression_numeric_kind(arg, bindings).map(|actual| actual == param)
}

fn expression_numeric_kind(expr: &Expression, bindings: &Bindings) -> Option<NumericKind> {
    if let Some((negative, _, suffix)) = literal_value(expr) {
        return Some(match suffix {
            Some(suffix) => literal_numeric_kind(Some(suffix)),
            None if negative => NumericKind::I64,
            None => NumericKind::U64,
        });
    }
    match expr {
        Expression::Ident(name) => bindings.get(name.as_str()).copied().flatten(),
        Expression::Unary { op: UnOp::Neg, expr } => expression_numeric_kind(expr, bindings),
        _ => None,
    }
}

fn verify_block(
    block: &Block,
    signatures: &HashMap<String, Vec<Option<NumericKind>>>,
    bindings: &Bindings,
    diagnostics: &mut Vec<CompilerDiagnostic>,
) {
    let mut bindings = bindings.clone();
    for stmt in &block.stmts {
        verify_statement(stmt, signatures, &mut bindings, diagnostics);
    }
}

fn verify_statement(
    stmt: &Statement,
    signatures: &HashMap<String, Vec<Option<NumericKind>>>,
    bindings: &mut Bindings,
    diagnostics: &mut Vec<CompilerDiagnostic>,
) {
    match stmt {
        Statement::Let { name, ty, expr, .. } => {
            if let Some(expr) = expr {
                verify_expression(expr, signatures, bindings, diagnostics);
                if let Some(kind) = ty.as_ref().and_then(numeric_type) {
                    if argument_satisfies(expr, kind, bindings) == Some(false) {
                        diagnostics.push(CompilerDiagnostic::error(
                            DiagnosticCode::InvalidNumericCoercion,
                            format!("initializer for `{}` has incompatible integer type", name.as_str()),
                            Span::DUMMY,
                        ));
                    }
                }
            }
            let kind = match ty {
                Some(ty) => numeric_type(ty),
                None => expr.as_ref().and_then(|expr| expression_numeric_kind(expr, bindings)),
            };
            bindings.insert(name.as_str().to_owned(), kind);
        }
        Statement::Expr(expr) => verify_expression(expr, signatures, bindings, diagnostics),
        Statement::Return(Some(expr)) => verify_expression(expr, signatures, bindings, diagnostics),
        Statement::If {
            cond,
            then_block,
            else_block,
        } => {
            verify_expression(cond, signatures, bindings, diagnostics);
            verify_block(then_block, signatures, bindings, diagnostics);
            if let Some(block) = else_block {
                verify_block(block, signatures, bindings, diagnostics);
            }
        }
        Statement::While { cond, body } => {
            verify_expression(cond, signatures, bindings, diagnostics);
            verify_block(body, signatures, bindings, diagnostics);
        }
        Statement::For {
            pattern,
            iterable,
            body,
        } => {
            verify_expression(iterable, signatures, bindings, diagnostics);
            let mut loop_bindings = bindings.clone();
            bind_pattern(pattern, &mut loop_bindings);
            verify_block(body, signatures, &loop_bindings, diagnostics);
        }
        Statement::Loop(block) => verify_block(block, signatures, bindings, diagnostics),
        Statement::Atomic(atomic) => verify_block(&atomic.body, signatures, bindings, diagnostics),
        _ => {}
    }
}

fn verify_expression(
    expr: &Expression,
    signatures: &HashMap<String, Vec<Option<NumericKind>>>,
    bindings: &Bindings,
    diagnostics: &mut Vec<CompilerDiagnostic>,
) {
    if let Some((_, _, suffix)) = literal_value(expr) {
        if let Some(suffix) = suffix {
            if !literal_fits(expr, literal_numeric_kind(Some(suffix))) {
                diagnostics.push(CompilerDiagnostic::error(
                    DiagnosticCode::InvalidNumericCoercion,
                    "integer literal does not fit its explicit suffix",
                    Span::DUMMY,
                ).with_help("choose an integer suffix that can represent the literal; unsigned types cannot represent negative values"));
            }
        }
        return;
    }
    match expr {
        Expression::Call { callee, args } => {
            verify_expression(callee, signatures, bindings, diagnostics);
            if let Expression::Ident(name) = callee.as_ref() {
                if let Some(params) = signatures
                    .get(name.as_str())
                    .filter(|_| !bindings.contains_key(name.as_str()))
                {
                    for (index, arg) in args.iter().enumerate() {
                        if let Some(param) = params.get(index).and_then(|kind| *kind) {
                            if let Some(satisfied) = argument_satisfies(arg, param, bindings) {
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
                        verify_expression(arg, signatures, bindings, diagnostics);
                    }
                    return;
                }
            }
            for arg in args {
                verify_expression(arg, signatures, bindings, diagnostics);
            }
        }
        Expression::Binary { lhs, rhs, .. } => {
            verify_expression(lhs, signatures, bindings, diagnostics);
            verify_expression(rhs, signatures, bindings, diagnostics);
        }
        Expression::Unary { expr, .. } | Expression::Await(expr) | Expression::Async(expr) | Expression::Try(expr) => {
            verify_expression(expr, signatures, bindings, diagnostics)
        }
        Expression::MethodCall { receiver, args, .. } => {
            verify_expression(receiver, signatures, bindings, diagnostics);
            for arg in args {
                verify_expression(arg, signatures, bindings, diagnostics);
            }
        }
        Expression::FieldAccess { target, .. } => verify_expression(target, signatures, bindings, diagnostics),
        Expression::Index { target, index } => {
            verify_expression(target, signatures, bindings, diagnostics);
            verify_expression(index, signatures, bindings, diagnostics);
        }
        Expression::IfExpr {
            cond,
            then_block,
            else_block,
        } => {
            verify_expression(cond, signatures, bindings, diagnostics);
            verify_block(then_block, signatures, bindings, diagnostics);
            if let Some(block) = else_block {
                verify_block(block, signatures, bindings, diagnostics);
            }
        }
        Expression::BlockExpr(block) => verify_block(block, signatures, bindings, diagnostics),
        Expression::Closure { params, body, .. } => {
            let mut closure_bindings = bindings.clone();
            bind_parameters(params, &mut closure_bindings);
            verify_expression(body, signatures, &closure_bindings, diagnostics);
        }
        Expression::Match { expr, arms } => {
            verify_expression(expr, signatures, bindings, diagnostics);
            for (pattern, arm) in arms {
                let mut arm_bindings = bindings.clone();
                bind_pattern(pattern, &mut arm_bindings);
                verify_expression(arm, signatures, &arm_bindings, diagnostics);
            }
        }
        Expression::Atomic(atomic) => verify_block(&atomic.body, signatures, bindings, diagnostics),
        Expression::Literal(_) | Expression::Ident(_) => {}
    }
}
