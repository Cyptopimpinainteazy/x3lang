//! What each annotation in this language does to an artifact, and the one that cannot do it
//! (TICKET-111).
//!
//! `lowering::lower_annotations_prefix` reduces seven of the twenty-one annotations the parser can
//! build to nothing. For six of them that is *policy*: they are properties of the code — no heap, no
//! recursion, on-chain, off-chain, concurrent, payable — and the artifact is not the place a property
//! of the code is stated. `@gas_adaptive` was the seventh and it is not the same thing: it claims the
//! program has two gas paths, which is a claim about the artifact, and until TICKET-110 it was
//! "honoured" by a record whose two bodies were four zero bytes that no reader can see. The claim
//! cannot be honoured, because the annotation names no bodies — so it is *reported* instead, through
//! the warning path TICKET-104 built: a real finding about a program that is still a program.
//!
//! The table below is the enumeration, because the decision is one decision: a modifier with no
//! artifact form is either stated as policy or reported, and never silently nothing.

use x3_lang_ast::ast::{Annotation, Item, Program};
use x3_lang_common::{Span, X3Error};

use crate::diagnostic::{CompilerDiagnostic, DiagnosticCode};
use crate::semantic::VerifyOutcome;

/// What this build does with an annotation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Disposition {
    /// It lowers into a record the artifact carries.
    Carried,
    /// It lowers to nothing by policy: a property of the code the artifact need not state. The string
    /// is the reason, and it is printed in the test that holds this table against the lowerer.
    Invisible(&'static str),
    /// It lowers to nothing and is **reported**, because it claims something the artifact would have
    /// to state and this build cannot.
    Reported(&'static str),
}

/// Every annotation the parser can build, with what this build does with it.
///
/// A row per *spelling*, because that is what a program writes and what a diagnostic has to name.
/// `tests/test_annotation_policy.rs` holds three facts against this table: every parser arm has a row,
/// every row spelling parses back to an annotation this module names, and the lowerer's output is
/// non-empty exactly for the `Carried` rows.
pub const DISPOSITIONS: &[(&str, Disposition)] = &[
    (
        "no_heap",
        Disposition::Invisible("a property of the code, not of the artifact"),
    ),
    (
        "no_recursion",
        Disposition::Invisible("a property of the code, not of the artifact"),
    ),
    ("hot", Disposition::Carried),
    ("audit", Disposition::Carried),
    ("role", Disposition::Carried),
    ("multisig", Disposition::Carried),
    ("version", Disposition::Carried),
    (
        "upgrade_from",
        // Measured, not inferred: `lower_annotations_suffix` collects this value and pushes the
        // `VersionMeta` record **only when a version was stated too**, so on its own the version it
        // upgrades from is dropped. With `@version` beside it the record carries both, which
        // `test_annotation_policy` asserts as a pair.
        Disposition::Reported(
            "it states the version this artifact upgrades from, and that claim travels only with \
             `@version`: on its own the artifact states neither",
        ),
    ),
    (
        "on_chain",
        Disposition::Invisible("a property of the code, not of the artifact"),
    ),
    (
        "off_chain",
        Disposition::Invisible("a property of the code, not of the artifact"),
    ),
    ("sandbox", Disposition::Carried),
    ("whitelist", Disposition::Carried),
    (
        "concurrent",
        Disposition::Invisible("a property of the code, not of the artifact"),
    ),
    ("scheduled", Disposition::Carried),
    ("extern", Disposition::Carried),
    (
        "payable",
        Disposition::Invisible("a property of the code, not of the artifact"),
    ),
    ("simd", Disposition::Carried),
    ("subscribe", Disposition::Carried),
    ("sponsor", Disposition::Carried),
    (
        "gas_adaptive",
        Disposition::Reported(
            "it claims the program has two gas paths, and the annotation names no bodies, so the \
             paths cannot be written: the artifact states no record for it rather than one whose \
             bodies are placeholders (TICKET-110)",
        ),
    ),
];

/// The disposition of a spelling, if the table has a row for it.
pub fn disposition(spelling: &str) -> Option<Disposition> {
    DISPOSITIONS
        .iter()
        .find(|(name, _)| *name == spelling)
        .map(|(_, disposition)| *disposition)
}

/// The word a program writes for an annotation.
///
/// The inverse of the parser's name map, and the reason the test parses each spelling back: a renderer
/// that drifted from the parser would produce a diagnostic naming a modifier the language does not
/// spell that way.
pub fn spelling(annotation: &Annotation) -> &'static str {
    match annotation {
        Annotation::NoHeap => "no_heap",
        Annotation::NoRecursion(_) => "no_recursion",
        Annotation::Hot => "hot",
        Annotation::Audit => "audit",
        Annotation::Role(_) => "role",
        Annotation::Multisig(_, _) => "multisig",
        Annotation::Version(_) => "version",
        Annotation::UpgradeFrom(_) => "upgrade_from",
        Annotation::OnChain => "on_chain",
        Annotation::OffChain => "off_chain",
        Annotation::Sandbox => "sandbox",
        Annotation::Whitelist(_) => "whitelist",
        Annotation::Concurrent => "concurrent",
        Annotation::Scheduled(_) => "scheduled",
        Annotation::Subscription(_, _) => "subscription",
        Annotation::Extern => "extern",
        Annotation::Payable => "payable",
        Annotation::Simd => "simd",
        Annotation::Subscribe(_) => "subscribe",
        Annotation::Sponsor => "sponsor",
        Annotation::GasAdaptive => "gas_adaptive",
    }
}

/// The findings a program's annotations raise: one warning per annotation whose claim the artifact
/// cannot carry.
///
/// A warning and not an error, and the distinction is the whole point of the ticket: the program is
/// valid and the *claim* is what is lost, so refusing to compile it would punish the wrong thing. The
/// finding goes through `VerifyOutcome::push_diagnostic`, which files it by its own severity — the
/// first production use of the warning path TICKET-104 built.
pub fn warnings(program: &Program) -> Vec<X3Error> {
    let mut outcome = VerifyOutcome {
        errors: Vec::new(),
        warnings: Vec::new(),
    };
    for item in &program.items {
        match &item.node {
            Item::Function(function) => report(&function.annotations, item.span, &mut outcome),
            Item::Agent(agent) => {
                report(&agent.annotations, item.span, &mut outcome);
                for method in &agent.methods {
                    report(&method.node.annotations, method.span, &mut outcome);
                }
            }
            _ => {}
        }
    }
    // Only warnings can come out of this pass: every finding it raises is a warning, and
    // `push_diagnostic` decides that from the diagnostic, not from the caller. The assertion is here
    // so a future finding added to this pass as an *error* fails loudly rather than disappearing into
    // a vector whose caller only reads the warnings.
    debug_assert!(
        outcome.errors.is_empty(),
        "the annotation pass raises findings, not rejections: {:?}",
        outcome.errors
    );
    outcome.warnings
}

fn report(annotations: &[Annotation], span: Span, outcome: &mut VerifyOutcome) {
    for annotation in annotations {
        let spelling = spelling(annotation);
        let Some(Disposition::Reported(reason)) = disposition(spelling) else {
            continue;
        };
        outcome.push_diagnostic(CompilerDiagnostic::warning(
            DiagnosticCode::DeclarationHasNoArtifactForm,
            format!("`@{spelling}` has no effect on this artifact: {reason}"),
            span,
        ));
    }
}
