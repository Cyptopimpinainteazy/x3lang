#![cfg(test)]

use x3_lang_lexer::Keyword;

#[test]
fn numeric_minus_has_its_own_token_and_exact_span() {
    use x3_lang_lexer::{Lexer, Literal, TokenKind, UnOp};
    let tokens = Lexer::lex_all("10-2");
    assert!(matches!(
        tokens[0].kind,
        TokenKind::Literal(Literal::Int { value: 10, .. })
    ));
    assert!(matches!(tokens[1].kind, TokenKind::UnOp(UnOp::Neg)));
    assert!(matches!(
        tokens[2].kind,
        TokenKind::Literal(Literal::Int { value: 2, .. })
    ));
    assert_eq!(
        tokens[1].span,
        x3_lang_common::Span::new(x3_lang_common::BytePos::new(2), x3_lang_common::BytePos::new(3), 0)
    );
}

#[test]
fn hyphenated_names_and_arrows_keep_their_token_boundaries() {
    use x3_lang_lexer::{Lexer, TokenKind};
    let tokens = Lexer::lex_all("chain-name->other");
    assert!(matches!(&tokens[0].kind, TokenKind::Ident(name) if name.as_str() == "chain-name"));
    assert!(matches!(&tokens[2].kind, TokenKind::Ident(name) if name.as_str() == "other"));
}

#[test]
fn trading_keywords_map_both_directions() {
    let pairs: &[(&str, Keyword)] = &[
        ("asset", Keyword::Asset),
        ("atomic", Keyword::Atomic),
        ("trade", Keyword::Trade),
        ("using", Keyword::Using),
        ("borrow", Keyword::Borrow),
        ("from", Keyword::From),
        ("as", Keyword::As),
        ("swap", Keyword::Swap),
        ("via", Keyword::Via),
        ("min_out", Keyword::MinOut),
        ("repay", Keyword::Repay),
        ("net_profit", Keyword::NetProfit),
        ("all_debts_repaid", Keyword::AllDebtsRepaid),
        ("receipt", Keyword::Receipt),
        ("bps", Keyword::Bps),
        ("private_submission", Keyword::PrivateSubmission),
    ];

    for (text, keyword) in pairs {
        assert_eq!(Keyword::from_str(text), Some(*keyword), "text {text}");
        assert_eq!(keyword.as_str(), *text, "keyword {keyword:?}");
    }
}

#[cfg(feature = "logos")]
mod lexing {
    use super::*;
    use x3_lang_lexer::{Lexer, Literal, TokenKind};

    #[test]
    fn lex_all_recognizes_every_trading_keyword() {
        let source = concat!(
            "asset atomic trade using borrow from as swap via ",
            "min_out repay net_profit all_debts_repaid receipt bps private_submission"
        );
        let tokens = Lexer::lex_all(source);
        let expected = [
            Keyword::Asset,
            Keyword::Atomic,
            Keyword::Trade,
            Keyword::Using,
            Keyword::Borrow,
            Keyword::From,
            Keyword::As,
            Keyword::Swap,
            Keyword::Via,
            Keyword::MinOut,
            Keyword::Repay,
            Keyword::NetProfit,
            Keyword::AllDebtsRepaid,
            Keyword::Receipt,
            Keyword::Bps,
            Keyword::PrivateSubmission,
        ];
        let kinds: Vec<TokenKind> = tokens.iter().map(|t| t.kind.clone()).collect();
        for (index, keyword) in expected.iter().enumerate() {
            assert_eq!(
                kinds.get(index),
                Some(&TokenKind::Keyword(*keyword)),
                "token index {index}"
            );
        }
        assert!(matches!(tokens.last().map(|t| &t.kind), Some(TokenKind::Eof)));
    }

    #[test]
    fn atomic_and_swap_tokenize_as_separate_preserved_keywords() {
        let tokens = Lexer::lex_all("atomic swap");
        let kinds: Vec<TokenKind> = tokens.iter().map(|t| t.kind.clone()).collect();
        assert_eq!(
            kinds,
            vec![
                TokenKind::Keyword(Keyword::Atomic),
                TokenKind::Keyword(Keyword::Swap),
                TokenKind::Eof
            ]
        );
    }

    #[test]
    fn decimal_literals_used_by_trading_syntax_lex_as_numbers() {
        let tokens = Lexer::lex_all("1_000_000 0.02 0xA0b8");
        let kinds: Vec<TokenKind> = tokens.iter().map(|t| t.kind.clone()).collect();
        assert!(matches!(
            &kinds[0],
            TokenKind::Literal(Literal::Int { value: 1_000_000, .. })
        ));
        assert!(matches!(
            &kinds[1],
            TokenKind::Literal(Literal::Float { value, .. }) if value.as_str() == "0.02"
        ));
        assert!(matches!(
            &kinds[2],
            TokenKind::Ident(name) if name.as_str() == "0xA0b8"
        ));
    }

    #[test]
    fn malformed_decimal_separators_are_not_silently_numbers() {
        for source in ["1_", "1__2", "1_a"] {
            let tokens = Lexer::lex_all(source);
            let kinds: Vec<TokenKind> = tokens.iter().map(|t| t.kind.clone()).collect();
            assert!(
                !kinds
                    .iter()
                    .any(|k| matches!(k, TokenKind::Literal(Literal::Int { .. }))),
                "source {source} must not lex as an integer literal: {kinds:?}"
            );
        }
    }
}
#[test]
fn unicode_escapes_decode_scalar_values_and_refuse_malformed_input() {
    use x3_lang_lexer::{Lexer, Literal, TokenKind};
    for (source, expected) in [
        (r#""\u{41}\u{1F4A5}\u{0}""#, "A💥\0"),
        (r#""\u{10ffff}""#, "\u{10ffff}"),
    ] {
        let tokens = Lexer::lex_all(source);
        assert!(
            matches!(&tokens[0].kind, TokenKind::Literal(Literal::String(value)) if value.as_str() == expected),
            "{source}"
        );
        assert_eq!(tokens[0].span.end.as_usize(), source.len());
    }
    for source in [
        r#""\u{}""#,
        r#""\u{D800}""#,
        r#""\u{110000}""#,
        r#""\u{0000000}""#,
        r#""\u{xyz}""#,
        r#""\u41""#,
        r#""\u{41""#,
        "\"\\u{💥",
    ] {
        assert!(
            matches!(Lexer::lex_all(source)[0].kind, TokenKind::Unknown('"')),
            "{source}"
        );
    }
}
