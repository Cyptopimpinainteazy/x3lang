use crate::token::{Token, TokenKind};

#[derive(Clone, Debug)]
pub struct Cursor {
    tokens: Vec<Token>,
    pos: usize,
}

impl Cursor {
    pub fn new(tokens: Vec<Token>) -> Self {
        Self { tokens, pos: 0 }
    }

    pub fn peek(&self) -> &Token {
        self.tokens
            .get(self.pos)
            .unwrap_or_else(|| self.tokens.last().expect("lexer cursor requires at least an EOF token"))
    }

    pub fn bump(&mut self) -> Token {
        let token = self.peek().clone();
        if !matches!(token.kind, TokenKind::Eof) {
            self.pos += 1;
        }
        token
    }

    pub fn is_eof(&self) -> bool {
        matches!(self.peek().kind, TokenKind::Eof)
    }
}
