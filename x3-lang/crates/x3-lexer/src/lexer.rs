use crate::token::{BinOp, Delimiter, IntBase, Keyword, Literal, Token, TokenKind, UnOp};
use x3_lang_common::{BytePos, Span, Symbol};

pub struct Lexer<'a> {
    tokens: Vec<Token>,
    pos: usize,
    _source: &'a str,
}

impl<'a> Lexer<'a> {
    pub fn new(source: &'a str, file_id: u32) -> Self {
        Self {
            tokens: Self::lex_all_with_file(source, file_id),
            pos: 0,
            _source: source,
        }
    }

    pub fn lex_all(source: &str) -> Vec<Token> {
        Self::lex_all_with_file(source, 0)
    }

    pub fn lex_all_with_file(source: &str, file_id: u32) -> Vec<Token> {
        let mut tokens = Vec::new();
        let bytes = source.as_bytes();
        let mut i = 0usize;
        while i < bytes.len() {
            let ch = source[i..].chars().next().unwrap();
            let start = i;
            i += ch.len_utf8();
            match ch {
                c if c.is_whitespace() => {
                    if c == '\n' {
                        tokens.push(token(TokenKind::Newline, start, i, file_id));
                    }
                }
                c if c.is_ascii_alphabetic() || c == '_' || c.is_ascii_digit() => {
                    let starts_with_digit = c.is_ascii_digit();
                    while i < bytes.len() {
                        let next = source[i..].chars().next().unwrap();
                        if next == '-' && source[i + next.len_utf8()..].starts_with('>') {
                            break;
                        }
                        if next.is_ascii_alphanumeric() || next == '_' || next == '-' {
                            i += next.len_utf8();
                        } else if starts_with_digit && next == '.' {
                            // Decimal literals (0.02, 1_000.5) continue through
                            // exactly one dot followed by at least one digit.
                            let after_dot = source[i + next.len_utf8()..].chars().next();
                            if after_dot.is_some_and(|ch| ch.is_ascii_digit()) {
                                i += next.len_utf8();
                            } else {
                                break;
                            }
                        } else {
                            break;
                        }
                    }
                    let text = &source[start..i];
                    if starts_with_digit {
                        if let Some(value) = parse_decimal_integer(text) {
                            tokens.push(token(
                                TokenKind::Literal(Literal::Int {
                                    value,
                                    suffix: None,
                                    base: IntBase::Decimal,
                                }),
                                start,
                                i,
                                file_id,
                            ));
                        } else if let Some(raw) = decimal_float_text(text) {
                            tokens.push(token(
                                TokenKind::Literal(Literal::Float {
                                    value: Symbol::new(raw),
                                    suffix: None,
                                }),
                                start,
                                i,
                                file_id,
                            ));
                        } else {
                            let kind = Keyword::from_str(text)
                                .map(TokenKind::Keyword)
                                .unwrap_or_else(|| TokenKind::Ident(Symbol::new(text)));
                            tokens.push(token(kind, start, i, file_id));
                        }
                    } else {
                        let kind = Keyword::from_str(text)
                            .map(TokenKind::Keyword)
                            .unwrap_or_else(|| TokenKind::Ident(Symbol::new(text)));
                        tokens.push(token(kind, start, i, file_id));
                    }
                }
                '"' => {
                    while i < bytes.len() {
                        let next = source[i..].chars().next().unwrap();
                        i += next.len_utf8();
                        if next == '"' {
                            break;
                        }
                    }
                    let end = i.saturating_sub(1);
                    tokens.push(token(
                        TokenKind::Literal(Literal::String(Symbol::new(&source[start + 1..end]))),
                        start,
                        i,
                        file_id,
                    ));
                }
                '(' => tokens.push(token(TokenKind::Delimiter(Delimiter::OpenParen), start, i, file_id)),
                ')' => tokens.push(token(TokenKind::Delimiter(Delimiter::CloseParen), start, i, file_id)),
                '[' => tokens.push(token(TokenKind::Delimiter(Delimiter::OpenBracket), start, i, file_id)),
                ']' => tokens.push(token(TokenKind::Delimiter(Delimiter::CloseBracket), start, i, file_id)),
                '{' => tokens.push(token(TokenKind::Delimiter(Delimiter::OpenBrace), start, i, file_id)),
                '}' => tokens.push(token(TokenKind::Delimiter(Delimiter::CloseBrace), start, i, file_id)),
                ',' => tokens.push(token(TokenKind::Comma, start, i, file_id)),
                ';' => tokens.push(token(TokenKind::Semi, start, i, file_id)),
                ':' => tokens.push(token(TokenKind::Colon, start, i, file_id)),
                '@' => tokens.push(token(TokenKind::At, start, i, file_id)),
                '.' => tokens.push(token(TokenKind::Dot, start, i, file_id)),
                '=' if source[i..].starts_with('>') => {
                    i += 1;
                    tokens.push(token(TokenKind::FatArrow, start, i, file_id));
                }
                '=' if source[i..].starts_with('=') => {
                    i += 1;
                    tokens.push(token(TokenKind::BinOp(BinOp::EqEq), start, i, file_id));
                }
                '=' => tokens.push(token(TokenKind::Eq, start, i, file_id)),
                '!' if source[i..].starts_with('=') => {
                    i += 1;
                    tokens.push(token(TokenKind::BinOp(BinOp::Ne), start, i, file_id));
                }
                '<' if source[i..].starts_with('=') => {
                    i += 1;
                    tokens.push(token(TokenKind::BinOp(BinOp::Le), start, i, file_id));
                }
                '>' if source[i..].starts_with('=') => {
                    i += 1;
                    tokens.push(token(TokenKind::BinOp(BinOp::Ge), start, i, file_id));
                }
                '&' if source[i..].starts_with('&') => {
                    i += 1;
                    tokens.push(token(TokenKind::BinOp(BinOp::AndAnd), start, i, file_id));
                }
                '|' if source[i..].starts_with('|') => {
                    i += 1;
                    tokens.push(token(TokenKind::BinOp(BinOp::OrOr), start, i, file_id));
                }
                '-' if source[i..].starts_with('>') => {
                    i += 1;
                    tokens.push(token(TokenKind::Arrow, start, i, file_id));
                }
                '+' => tokens.push(token(TokenKind::BinOp(BinOp::Plus), start, i, file_id)),
                '*' => tokens.push(token(TokenKind::BinOp(BinOp::Star), start, i, file_id)),
                // Comments. These arms must precede the generic `'/` arm
                // below, or `//` would lex as two division operators and
                // `/*` as division followed by multiply.
                '/' if source[i..].starts_with('/') => {
                    // Line comment: `//` to end of line. The terminating
                    // newline is deliberately left unconsumed so the Newline
                    // token that ends the logical line is still produced —
                    // this grammar is newline-sensitive, and swallowing it
                    // would let a trailing comment merge two statements.
                    i += 1;
                    while i < bytes.len() {
                        let next = source[i..].chars().next().unwrap();
                        if next == '\n' {
                            break;
                        }
                        i += next.len_utf8();
                    }
                }
                '/' if source[i..].starts_with('*') => {
                    // Block comment: `/* ... */`. Newlines inside the comment
                    // still produce Newline tokens, so wrapping a token in a
                    // block comment cannot silently merge two logical lines
                    // the way a plain whitespace skip would.
                    i += 1;
                    let mut terminated = false;
                    while i < bytes.len() {
                        let next = source[i..].chars().next().unwrap();
                        if next == '*' && source[i + 1..].starts_with('/') {
                            i += 2;
                            terminated = true;
                            break;
                        }
                        if next == '\n' {
                            tokens.push(token(TokenKind::Newline, i, i + 1, file_id));
                        }
                        i += next.len_utf8();
                    }
                    if !terminated {
                        // Fail closed. Silently treating an unterminated block
                        // comment as whitespace to end of file would discard
                        // the rest of the program and still report success.
                        tokens.push(token(TokenKind::Unknown('/'), start, start + 1, file_id));
                    }
                }
                '/' => tokens.push(token(TokenKind::BinOp(BinOp::Slash), start, i, file_id)),
                '%' => tokens.push(token(TokenKind::BinOp(BinOp::Percent), start, i, file_id)),
                '<' => tokens.push(token(TokenKind::BinOp(BinOp::Lt), start, i, file_id)),
                '>' => tokens.push(token(TokenKind::BinOp(BinOp::Gt), start, i, file_id)),
                '!' => tokens.push(token(TokenKind::UnOp(UnOp::Not), start, i, file_id)),
                '-' => tokens.push(token(TokenKind::UnOp(UnOp::Neg), start, i, file_id)),
                _ => tokens.push(token(TokenKind::Unknown(ch), start, i, file_id)),
            }
        }
        tokens.push(token(TokenKind::Eof, source.len(), source.len(), file_id));
        tokens
    }
}

impl Iterator for Lexer<'_> {
    type Item = Token;

    fn next(&mut self) -> Option<Self::Item> {
        let token = self.tokens.get(self.pos).cloned();
        if token.is_some() {
            self.pos += 1;
        }
        token
    }
}

fn token(kind: TokenKind, start: usize, end: usize, file_id: u32) -> Token {
    Token::new(kind, Span::new(BytePos(start as u32), BytePos(end as u32), file_id))
}

/// Parse a decimal integer that may contain `_` separators
/// (for example `1_000_000`). Non-decimal or overflowing text returns `None`
/// so the lexer falls back to identifier classification.
fn parse_decimal_integer(text: &str) -> Option<u128> {
    if !well_formed_number_part(text) {
        return None;
    }
    let digits: String = text.chars().filter(|ch| *ch != '_').collect();
    digits.parse().ok()
}

/// Validate a decimal floating-point literal and return its source text.
/// Accepts a single `.` with digits on both sides and `_` separators, while
/// rejecting hex-like identifiers (`0xA0b8`) and suffixed words.
fn decimal_float_text(text: &str) -> Option<&str> {
    let mut parts = text.split('.');
    let int_part = parts.next()?;
    let frac_part = parts.next()?;
    if parts.next().is_some() {
        return None;
    }
    let valid_side = |side: &str| well_formed_number_part(side) && side.chars().any(|ch| ch.is_ascii_digit());
    if valid_side(int_part) && valid_side(frac_part) {
        Some(text)
    } else {
        None
    }
}

/// A decimal number part is well formed when it starts and ends with a digit
/// and every `_` separator sits between two digits. This rejects `1_`, `_1`,
/// `1__2`, and `1_a` instead of silently filtering separators out.
fn well_formed_number_part(part: &str) -> bool {
    let chars: Vec<char> = part.chars().collect();
    if chars.is_empty() || !chars[0].is_ascii_digit() || !chars[chars.len() - 1].is_ascii_digit() {
        return false;
    }
    for (index, ch) in chars.iter().enumerate() {
        match ch {
            c if c.is_ascii_digit() => {}
            '_' => {
                if index == 0
                    || index + 1 >= chars.len()
                    || !chars[index - 1].is_ascii_digit()
                    || !chars[index + 1].is_ascii_digit()
                {
                    return false;
                }
            }
            _ => return false,
        }
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    fn kinds(source: &str) -> Vec<TokenKind> {
        Lexer::lex_all(source).into_iter().map(|token| token.kind).collect()
    }

    fn newline_count(tokens: &[TokenKind]) -> usize {
        tokens.iter().filter(|kind| matches!(kind, TokenKind::Newline)).count()
    }

    fn has_unknown(tokens: &[TokenKind]) -> bool {
        tokens.iter().any(|kind| matches!(kind, TokenKind::Unknown(_)))
    }

    fn has_intent_keyword(tokens: &[TokenKind]) -> bool {
        tokens
            .iter()
            .any(|kind| matches!(kind, TokenKind::Keyword(Keyword::Intent)))
    }

    #[test]
    fn line_comment_at_start_of_file_is_skipped() {
        // Regression: `//` used to lex as two `Slash` operators, so a file
        // beginning with a comment failed the parser with "expected top-level
        // item". Six of the shipped `examples/*.x3` files were unparseable
        // for exactly this reason.
        let tokens = kinds("// leading comment\nintent a {\n}\n");
        assert!(
            !has_unknown(&tokens),
            "comment characters leaked into tokens: {tokens:?}"
        );
        assert!(
            has_intent_keyword(&tokens),
            "the declaration after the comment must survive"
        );
    }

    #[test]
    fn line_comment_inside_a_block_is_skipped() {
        let tokens = kinds("intent a {\n// inner\n}\n");
        assert!(
            !has_unknown(&tokens),
            "comment characters leaked into tokens: {tokens:?}"
        );
        assert!(has_intent_keyword(&tokens));
    }

    #[test]
    fn line_comment_does_not_swallow_its_terminating_newline() {
        // The grammar is newline-sensitive. If a line comment consumed its own
        // newline, a trailing comment could merge two statements into one.
        let tokens = kinds("let x = 1 // trailing\nlet y = 2\n");
        assert_eq!(newline_count(&tokens), 2, "both logical line endings must survive");
    }

    #[test]
    fn block_comment_is_skipped() {
        let tokens = kinds("/* block */\nintent a {\n}\n");
        assert!(!has_unknown(&tokens), "block comment characters leaked: {tokens:?}");
        assert!(has_intent_keyword(&tokens));
    }

    #[test]
    fn multiline_block_comment_preserves_line_structure() {
        // Four line endings: after `{`, inside the comment, after `*/`, and
        // after the closing `}`.
        let tokens = kinds("intent a {\n/* x\ny */\n}\n");
        assert!(!has_unknown(&tokens), "block comment characters leaked: {tokens:?}");
        assert_eq!(
            newline_count(&tokens),
            4,
            "newlines inside a block comment must still be observed"
        );
    }

    #[test]
    fn unterminated_block_comment_fails_closed() {
        // Treating an unterminated block comment as whitespace to EOF would
        // discard the rest of the program and still report success.
        let tokens = kinds("/* never closed\nintent a {\n}\n");
        assert!(
            has_unknown(&tokens),
            "an unterminated block comment must produce an error token, got {tokens:?}"
        );
    }

    #[test]
    fn division_is_still_lexed_as_slash() {
        let tokens = kinds("let x = 6 / 2\n");
        assert!(
            tokens.iter().any(|kind| matches!(kind, TokenKind::BinOp(BinOp::Slash))),
            "single `/` must remain division"
        );
        assert!(!has_unknown(&tokens));
    }

    #[test]
    fn comment_markers_inside_comments_do_not_confuse_the_lexer() {
        let line_with_block = kinds("// a /* b */\nintent a {\n}\n");
        assert!(
            !has_unknown(&line_with_block),
            "line comment must swallow `/*`: {line_with_block:?}"
        );
        assert!(has_intent_keyword(&line_with_block));

        let block_with_line = kinds("/* // */\nintent a {\n}\n");
        assert!(
            !has_unknown(&block_with_line),
            "block comment must swallow `//`: {block_with_line:?}"
        );
        assert!(has_intent_keyword(&block_with_line));
    }
}
