//! Source file management for the X3 compiler.

use crate::span::{BytePos, LineCol, LineColRange, Span};
use rustc_hash::FxHashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

/// A source file with its contents and metadata.
#[derive(Debug, Clone)]
pub struct SourceFile {
    /// Unique identifier for this file.
    pub id: u32,
    /// The file path (may be synthetic for REPL input).
    pub path: PathBuf,
    /// The source code contents.
    pub source: Arc<str>,
    /// Line start byte positions (for line/col lookup).
    line_starts: Vec<BytePos>,
}

impl SourceFile {
    pub fn new(id: u32, path: PathBuf, source: String) -> Self {
        let line_starts = compute_line_starts(&source);
        SourceFile {
            id,
            path,
            source: Arc::from(source),
            line_starts,
        }
    }

    /// Create a synthetic source file (e.g., for REPL input).
    pub fn synthetic(id: u32, name: &str, source: String) -> Self {
        Self::new(id, PathBuf::from(name), source)
    }

    /// Get the source text for a span.
    pub fn source_slice(&self, span: Span) -> &str {
        &self.source[span.start.as_usize()..span.end.as_usize()]
    }

    /// Convert a byte position to line/column.
    pub fn byte_to_line_col(&self, pos: BytePos) -> LineCol {
        let line = match self.line_starts.binary_search(&pos) {
            Ok(line) => line,
            Err(line) => line.saturating_sub(1),
        };

        let line_start = self.line_starts.get(line).copied().unwrap_or(BytePos::ZERO);
        let col = pos.0 - line_start.0;

        LineCol {
            line: (line + 1) as u32, // 1-indexed
            col: col + 1,            // 1-indexed
        }
    }

    /// Convert a span to line/column range.
    pub fn span_to_line_col(&self, span: Span) -> LineColRange {
        LineColRange {
            start: self.byte_to_line_col(span.start),
            end: self.byte_to_line_col(span.end),
        }
    }

    /// Get the line number for a byte position.
    pub fn line_number(&self, pos: BytePos) -> u32 {
        self.byte_to_line_col(pos).line
    }

    /// Get the content of a specific line (0-indexed).
    pub fn line_content(&self, line: usize) -> Option<&str> {
        let start = self.line_starts.get(line)?.as_usize();
        let end = self
            .line_starts
            .get(line + 1)
            .map(|p| p.as_usize())
            .unwrap_or(self.source.len());

        Some(self.source[start..end].trim_end_matches('\n'))
    }

    /// Get the byte position for the start of a line (0-indexed).
    pub fn line_start(&self, line: usize) -> Option<BytePos> {
        self.line_starts.get(line).copied()
    }

    /// Get the number of lines in the file.
    pub fn line_count(&self) -> usize {
        self.line_starts.len()
    }

    /// Get the file name (without directory).
    pub fn file_name(&self) -> &str {
        self.path.file_name().and_then(|n| n.to_str()).unwrap_or("<unknown>")
    }
}

/// Compute line start positions for a source string.
fn compute_line_starts(source: &str) -> Vec<BytePos> {
    let mut line_starts = vec![BytePos::ZERO];

    for (i, c) in source.char_indices() {
        if c == '\n' {
            line_starts.push(BytePos((i + 1) as u32));
        }
    }

    line_starts
}

/// A map of all source files in a compilation.
#[derive(Debug, Default)]
pub struct SourceMap {
    files: FxHashMap<u32, Arc<SourceFile>>,
    path_to_id: FxHashMap<PathBuf, u32>,
    next_id: u32,
}

impl SourceMap {
    pub fn new() -> Self {
        Self::default()
    }

    /// Add a source file from a path.
    pub fn add_file(&mut self, path: impl AsRef<Path>, source: String) -> Arc<SourceFile> {
        let path = path.as_ref().to_path_buf();

        if let Some(&id) = self.path_to_id.get(&path) {
            return self.files.get(&id).unwrap().clone();
        }

        let id = self.next_id;
        self.next_id += 1;

        let file = Arc::new(SourceFile::new(id, path.clone(), source));
        self.files.insert(id, file.clone());
        self.path_to_id.insert(path, id);

        file
    }

    /// Add a synthetic source file.
    pub fn add_synthetic(&mut self, name: &str, source: String) -> Arc<SourceFile> {
        let id = self.next_id;
        self.next_id += 1;

        let file = Arc::new(SourceFile::synthetic(id, name, source));
        self.files.insert(id, file.clone());

        file
    }

    /// Get a source file by ID.
    pub fn get_file(&self, id: u32) -> Option<&Arc<SourceFile>> {
        self.files.get(&id)
    }

    /// Get a source file by path.
    pub fn get_file_by_path(&self, path: impl AsRef<Path>) -> Option<&Arc<SourceFile>> {
        let id = self.path_to_id.get(path.as_ref())?;
        self.files.get(id)
    }

    /// Get the source text for a span.
    pub fn source_slice(&self, span: Span) -> Option<&str> {
        let file = self.get_file(span.file_id)?;
        Some(file.source_slice(span))
    }

    /// Get line/column information for a span.
    pub fn span_to_line_col(&self, span: Span) -> Option<(Arc<SourceFile>, LineColRange)> {
        let file = self.get_file(span.file_id)?;
        let range = file.span_to_line_col(span);
        Some((file.clone(), range))
    }

    /// Get all source files.
    pub fn files(&self) -> impl Iterator<Item = &Arc<SourceFile>> {
        self.files.values()
    }

    /// Get the number of files.
    pub fn file_count(&self) -> usize {
        self.files.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_line_col_conversion() {
        let source = "fn main() {\n    let x = 42;\n}\n";
        let file = SourceFile::new(0, PathBuf::from("test.x3"), source.to_string());

        // First line, first char
        let lc = file.byte_to_line_col(BytePos(0));
        assert_eq!(lc.line, 1);
        assert_eq!(lc.col, 1);

        // First line, "main"
        let lc = file.byte_to_line_col(BytePos(3));
        assert_eq!(lc.line, 1);
        assert_eq!(lc.col, 4);

        // Second line
        let lc = file.byte_to_line_col(BytePos(12));
        assert_eq!(lc.line, 2);
        assert_eq!(lc.col, 1);
    }

    #[test]
    fn test_line_content() {
        let source = "line 1\nline 2\nline 3";
        let file = SourceFile::new(0, PathBuf::from("test.x3"), source.to_string());

        assert_eq!(file.line_content(0), Some("line 1"));
        assert_eq!(file.line_content(1), Some("line 2"));
        assert_eq!(file.line_content(2), Some("line 3"));
        assert_eq!(file.line_content(3), None);
    }
}
