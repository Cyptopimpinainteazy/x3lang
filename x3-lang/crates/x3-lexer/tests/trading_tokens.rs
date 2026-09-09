#![cfg(test)]

use x3_lang_lexer::Keyword;

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
    use x3_lang_lexer::{Lexer, TokenKind};

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
}
