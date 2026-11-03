//! Diagnostic reporting for the X3 compiler.

use crate::error::X3Error;
use crate::source::SourceMap;
use crate::span::Span;
use std::fmt;
use std::io::Write;

/// Diagnostic severity level.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum DiagnosticLevel {
    /// An error that prevents compilation.
    Error,
    /// A warning that doesn't prevent compilation.
    Warning,
    /// Informational note.
    Note,
    /// A help message with suggestions.
    Help,
}

impl fmt::Display for DiagnosticLevel {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            DiagnosticLevel::Error => write!(f, "error"),
            DiagnosticLevel::Warning => write!(f, "warning"),
            DiagnosticLevel::Note => write!(f, "note"),
            DiagnosticLevel::Help => write!(f, "help"),
        }
    }
}

/// A labeled span within a diagnostic.
#[derive(Debug, Clone)]
pub struct DiagnosticLabel {
    pub span: Span,
    pub message: Option<String>,
    pub is_primary: bool,
}

impl DiagnosticLabel {
    pub fn primary(span: Span, message: impl Into<String>) -> Self {
        DiagnosticLabel {
            span,
            message: Some(message.into()),
            is_primary: true,
        }
    }

    pub fn secondary(span: Span, message: impl Into<String>) -> Self {
        DiagnosticLabel {
            span,
            message: Some(message.into()),
            is_primary: false,
        }
    }

    pub fn primary_no_message(span: Span) -> Self {
        DiagnosticLabel {
            span,
            message: None,
            is_primary: true,
        }
    }
}

/// A compiler diagnostic message.
#[derive(Debug, Clone)]
pub struct Diagnostic {
    pub level: DiagnosticLevel,
    pub code: Option<String>,
    pub message: String,
    pub labels: Vec<DiagnosticLabel>,
    pub notes: Vec<String>,
    pub help: Vec<String>,
}

impl Diagnostic {
    pub fn error(message: impl Into<String>) -> Self {
        Diagnostic {
            level: DiagnosticLevel::Error,
            code: None,
            message: message.into(),
            labels: Vec::new(),
            notes: Vec::new(),
            help: Vec::new(),
        }
    }

    pub fn warning(message: impl Into<String>) -> Self {
        Diagnostic {
            level: DiagnosticLevel::Warning,
            code: None,
            message: message.into(),
            labels: Vec::new(),
            notes: Vec::new(),
            help: Vec::new(),
        }
    }

    pub fn note_msg(message: impl Into<String>) -> Self {
        Diagnostic {
            level: DiagnosticLevel::Note,
            code: None,
            message: message.into(),
            labels: Vec::new(),
            notes: Vec::new(),
            help: Vec::new(),
        }
    }

    pub fn with_code(mut self, code: impl Into<String>) -> Self {
        self.code = Some(code.into());
        self
    }

    pub fn with_label(mut self, label: DiagnosticLabel) -> Self {
        self.labels.push(label);
        self
    }

    pub fn with_primary_label(self, span: Span, message: impl Into<String>) -> Self {
        self.with_label(DiagnosticLabel::primary(span, message))
    }

    pub fn with_secondary_label(self, span: Span, message: impl Into<String>) -> Self {
        self.with_label(DiagnosticLabel::secondary(span, message))
    }

    pub fn with_note(mut self, note: impl Into<String>) -> Self {
        self.notes.push(note.into());
        self
    }

    pub fn with_help(mut self, help: impl Into<String>) -> Self {
        self.help.push(help.into());
        self
    }
}

impl From<X3Error> for Diagnostic {
    fn from(error: X3Error) -> Self {
        match error {
            X3Error::LexerError { message, span } => Diagnostic::error(message)
                .with_code("E0001")
                .with_primary_label(span, "lexer error occurred here"),
            X3Error::ParseError {
                message,
                span,
                expected,
                found,
            } => {
                let mut diag = Diagnostic::error(message)
                    .with_code("E0002")
                    .with_primary_label(span, format!("found `{}`", found));

                if !expected.is_empty() {
                    let exp_str = if expected.len() == 1 {
                        format!("expected `{}`", expected[0])
                    } else {
                        format!("expected one of: {}", expected.join(", "))
                    };
                    diag = diag.with_note(exp_str);
                }
                diag
            }
            X3Error::TypeError { message, span } => Diagnostic::error(message)
                .with_code("E0003")
                .with_primary_label(span, "type error"),
            X3Error::NameError {
                message,
                span,
                suggestions,
            } => {
                let mut diag = Diagnostic::error(message)
                    .with_code("E0004")
                    .with_primary_label(span, "not found in this scope");

                for suggestion in suggestions {
                    diag = diag.with_help(format!("did you mean `{}`?", suggestion));
                }
                diag
            }
            X3Error::SemanticError { message, span } => Diagnostic::error(message)
                .with_code("E0005")
                .with_primary_label(span, "semantic error"),
            X3Error::CodegenError { message, span } => {
                let mut diag = Diagnostic::error(message).with_code("E0006");
                if let Some(s) = span {
                    diag = diag.with_primary_label(s, "code generation failed");
                }
                diag
            }
            X3Error::IoError { message, path } => {
                let mut diag = Diagnostic::error(message).with_code("E0007");
                if let Some(p) = path {
                    diag = diag.with_note(format!("path: {}", p));
                }
                diag
            }
            X3Error::InternalError { message } => Diagnostic::error(format!("internal compiler error: {}", message))
                .with_code("E9999")
                .with_note("this is a bug in the X3 compiler")
                .with_help("please report this issue"),
            X3Error::AgentError {
                message,
                span,
                agent_name,
            } => Diagnostic::error(message)
                .with_code("E1001")
                .with_primary_label(span, format!("in agent `{}`", agent_name)),
            X3Error::AtomicError { message, span } => Diagnostic::error(message)
                .with_code("E1002")
                .with_primary_label(span, "in atomic block"),
            X3Error::CrossChainError {
                message,
                span,
                source_chain,
                target_chain,
            } => Diagnostic::error(message)
                .with_code("E1003")
                .with_primary_label(span, "cross-chain operation")
                .with_note(format!("from {} to {}", source_chain, target_chain)),
            X3Error::MevError {
                message,
                span,
                operation,
            } => Diagnostic::error(message)
                .with_code("E1004")
                .with_primary_label(span, format!("in `{}` operation", operation)),
        }
    }
}

/// Builds and emits diagnostics.
pub struct DiagnosticBuilder<'a> {
    source_map: &'a SourceMap,
    diagnostics: Vec<Diagnostic>,
}

impl<'a> DiagnosticBuilder<'a> {
    pub fn new(source_map: &'a SourceMap) -> Self {
        DiagnosticBuilder {
            source_map,
            diagnostics: Vec::new(),
        }
    }

    pub fn add(&mut self, diagnostic: Diagnostic) {
        self.diagnostics.push(diagnostic);
    }

    pub fn add_error(&mut self, error: X3Error) {
        self.add(Diagnostic::from(error));
    }

    pub fn has_errors(&self) -> bool {
        self.diagnostics.iter().any(|d| d.level == DiagnosticLevel::Error)
    }

    pub fn error_count(&self) -> usize {
        self.diagnostics
            .iter()
            .filter(|d| d.level == DiagnosticLevel::Error)
            .count()
    }

    pub fn warning_count(&self) -> usize {
        self.diagnostics
            .iter()
            .filter(|d| d.level == DiagnosticLevel::Warning)
            .count()
    }

    /// Emit all diagnostics to a writer with colored output.
    pub fn emit<W: Write>(&self, writer: &mut W, use_color: bool) -> std::io::Result<()> {
        for diag in &self.diagnostics {
            self.emit_diagnostic(writer, diag, use_color)?;
        }
        Ok(())
    }

    fn emit_diagnostic<W: Write>(&self, w: &mut W, diag: &Diagnostic, use_color: bool) -> std::io::Result<()> {
        // Level and code
        let level_str = match diag.level {
            DiagnosticLevel::Error => {
                if use_color {
                    "\x1b[1;31merror\x1b[0m"
                } else {
                    "error"
                }
            }
            DiagnosticLevel::Warning => {
                if use_color {
                    "\x1b[1;33mwarning\x1b[0m"
                } else {
                    "warning"
                }
            }
            DiagnosticLevel::Note => {
                if use_color {
                    "\x1b[1;36mnote\x1b[0m"
                } else {
                    "note"
                }
            }
            DiagnosticLevel::Help => {
                if use_color {
                    "\x1b[1;32mhelp\x1b[0m"
                } else {
                    "help"
                }
            }
        };

        if let Some(code) = &diag.code {
            writeln!(w, "{}[{}]: {}", level_str, code, diag.message)?;
        } else {
            writeln!(w, "{}: {}", level_str, diag.message)?;
        }

        // Labels with source context
        for label in &diag.labels {
            if let Some(file) = self.source_map.get_file(label.span.file_id) {
                let line_col = file.span_to_line_col(label.span);
                let arrow = if use_color { "\x1b[1;34m-->\x1b[0m" } else { "-->" };
                writeln!(
                    w,
                    " {} {}:{}:{}",
                    arrow,
                    file.path.display(),
                    line_col.start.line,
                    line_col.start.col
                )?;

                // Show source line
                if let Some(line_content) = file.line_content((line_col.start.line - 1) as usize) {
                    let line_num = format!("{}", line_col.start.line);
                    let padding = " ".repeat(line_num.len());

                    let pipe = if use_color { "\x1b[1;34m|\x1b[0m" } else { "|" };
                    writeln!(w, " {} {}", padding, pipe)?;
                    writeln!(w, " {} {} {}", line_num, pipe, line_content)?;

                    // Underline
                    let underline_start = (line_col.start.col - 1) as usize;
                    let underline_len = if line_col.start.line == line_col.end.line {
                        (line_col.end.col - line_col.start.col) as usize
                    } else {
                        line_content.len().saturating_sub(underline_start)
                    };
                    let underline_len = underline_len.max(1);

                    let underline_char = if label.is_primary { '^' } else { '-' };
                    let underline = underline_char.to_string().repeat(underline_len);

                    let underline_colored = if use_color {
                        if label.is_primary {
                            format!("\x1b[1;31m{}\x1b[0m", underline)
                        } else {
                            format!("\x1b[1;34m{}\x1b[0m", underline)
                        }
                    } else {
                        underline
                    };

                    let spaces = " ".repeat(underline_start);
                    if let Some(msg) = &label.message {
                        writeln!(w, " {} {} {}{} {}", padding, pipe, spaces, underline_colored, msg)?;
                    } else {
                        writeln!(w, " {} {} {}{}", padding, pipe, spaces, underline_colored)?;
                    }
                }
            }
        }

        // Notes
        for note in &diag.notes {
            let note_label = if use_color { "\x1b[1;36mnote\x1b[0m" } else { "note" };
            writeln!(w, " = {}: {}", note_label, note)?;
        }

        // Help
        for help in &diag.help {
            let help_label = if use_color { "\x1b[1;32mhelp\x1b[0m" } else { "help" };
            writeln!(w, " = {}: {}", help_label, help)?;
        }

        writeln!(w)?;
        Ok(())
    }
}
