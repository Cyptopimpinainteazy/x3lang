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
                    while i < bytes.len() {
                        let next = source[i..].chars().next().unwrap();
                        if next == '-' && source[i + next.len_utf8()..].starts_with('>') {
                            break;
                        }
                        if next.is_ascii_alphanumeric() || next == '_' || next == '-' {
                            i += next.len_utf8();
                        } else {
                            break;
                        }
                    }
                    let text = &source[start..i];
                    if text.chars().all(|ch| ch.is_ascii_digit()) {
                        let value = text.parse::<u128>().unwrap_or(0);
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
