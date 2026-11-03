//! Symbol interning for efficient string handling in the X3 compiler.
//!
//! Production builds use the `internment` crate; the offline build falls back
//! to a small in-process interner so the workspace can still compile without
//! network access. Both paths preserve the same `Symbol` public API.

#[cfg(feature = "intern")]
use internment::Intern;

use rustc_hash::FxHashMap;
use serde::{Deserialize, Serialize};
use std::fmt;
use std::sync::atomic::{AtomicU32, Ordering};

#[cfg(not(feature = "intern"))]
#[derive(Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct Intern<T>(T);

#[cfg(not(feature = "intern"))]
impl<T> Intern<T> {
    pub fn new(value: T) -> Self {
        Intern(value)
    }
    pub fn as_str(&self) -> &str
    where
        T: AsRef<str>,
    {
        self.0.as_ref()
    }
    pub fn len(&self) -> usize
    where
        T: AsRef<str>,
    {
        self.0.as_ref().len()
    }
    pub fn is_empty(&self) -> bool
    where
        T: AsRef<str>,
    {
        self.0.as_ref().is_empty()
    }
}

/// An interned string symbol.
///
/// Symbols are cheap to copy, compare, and hash. They are used throughout
/// the compiler for identifiers, keywords, and other frequently-used strings.
#[derive(Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct Symbol(Intern<String>);

#[cfg(feature = "intern")]
impl Copy for Symbol {}

impl Symbol {
    /// Create a new symbol from a string.
    #[inline]
    pub fn new(s: &str) -> Self {
        Symbol(Intern::new(s.to_string()))
    }

    /// Create a new symbol from an owned string.
    #[inline]
    pub fn from_string(s: String) -> Self {
        Symbol(Intern::new(s))
    }

    /// Get the string value of this symbol.
    #[inline]
    pub fn as_str(&self) -> &str {
        self.0.as_str()
    }

    /// Get the length of the symbol's string.
    #[inline]
    pub fn len(&self) -> usize {
        self.0.len()
    }

    /// Check if the symbol is empty.
    #[inline]
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }
}

impl fmt::Debug for Symbol {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Symbol({:?})", self.as_str())
    }
}

impl fmt::Display for Symbol {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.as_str())
    }
}

impl From<&str> for Symbol {
    fn from(s: &str) -> Self {
        Symbol::new(s)
    }
}

impl From<String> for Symbol {
    fn from(s: String) -> Self {
        Symbol::from_string(s)
    }
}

impl AsRef<str> for Symbol {
    fn as_ref(&self) -> &str {
        self.as_str()
    }
}

impl PartialEq<str> for Symbol {
    fn eq(&self, other: &str) -> bool {
        self.as_str() == other
    }
}

impl PartialEq<&str> for Symbol {
    fn eq(&self, other: &&str) -> bool {
        self.as_str() == *other
    }
}

/// A symbol interner that provides numeric IDs for symbols.
///
/// This is used when we need stable numeric identifiers for symbols,
/// such as in serialized data structures.
#[derive(Debug, Default)]
pub struct SymbolInterner {
    symbol_to_id: FxHashMap<Symbol, u32>,
    id_to_symbol: Vec<Symbol>,
    next_id: AtomicU32,
}

impl SymbolInterner {
    pub fn new() -> Self {
        Self::default()
    }

    /// Intern a symbol and get its numeric ID.
    pub fn intern(&mut self, symbol: Symbol) -> u32 {
        if let Some(&id) = self.symbol_to_id.get(&symbol) {
            return id;
        }

        let id = self.next_id.fetch_add(1, Ordering::SeqCst);
        self.symbol_to_id.insert(symbol.clone(), id);
        self.id_to_symbol.push(symbol);
        id
    }

    /// Intern a string and get its numeric ID.
    pub fn intern_str(&mut self, s: &str) -> u32 {
        self.intern(Symbol::new(s))
    }

    /// Get the symbol for a numeric ID.
    pub fn get(&self, id: u32) -> Option<Symbol> {
        self.id_to_symbol.get(id as usize).cloned()
    }

    /// Get the ID for a symbol if it exists.
    pub fn get_id(&self, symbol: Symbol) -> Option<u32> {
        self.symbol_to_id.get(&symbol).copied()
    }

    /// Get the number of interned symbols.
    pub fn len(&self) -> usize {
        self.id_to_symbol.len()
    }

    pub fn is_empty(&self) -> bool {
        self.id_to_symbol.is_empty()
    }
}

/// Pre-defined symbols for keywords and common identifiers.
pub mod kw {
    use super::Symbol;
    use std::sync::LazyLock;

    macro_rules! define_keywords {
        ($($name:ident => $str:literal),* $(,)?) => {
            $(
                pub static $name: LazyLock<Symbol> = LazyLock::new(|| Symbol::new($str));
            )*

            /// Check if a symbol is a keyword.
            pub fn is_keyword(sym: Symbol) -> bool {
                $(
                    if sym == *$name { return true; }
                )*
                false
            }

            /// Get all keywords.
            pub fn all_keywords() -> Vec<Symbol> {
                vec![$($name.clone()),*]
            }
        };
    }

    define_keywords! {
        // Basic keywords
        FN => "fn",
        LET => "let",
        MUT => "mut",
        CONST => "const",
        STATIC => "static",
        IF => "if",
        ELSE => "else",
        MATCH => "match",
        FOR => "for",
        WHILE => "while",
        LOOP => "loop",
        BREAK => "break",
        CONTINUE => "continue",
        RETURN => "return",
        YIELD => "yield",

        // Type keywords
        STRUCT => "struct",
        ENUM => "enum",
        TRAIT => "trait",
        IMPL => "impl",
        TYPE => "type",
        WHERE => "where",

        // Visibility
        PUB => "pub",
        PRIV => "priv",

        // Module system
        MOD => "mod",
        USE => "use",
        AS => "as",
        SELF_LOWER => "self",
        SELF_UPPER => "Self",
        SUPER => "super",
        CRATE => "crate",

        // X3-specific: Agent keywords
        AGENT => "agent",
        CONTEXT => "context",
        STATE => "state",
        STRATEGY => "strategy",
        SPAWN => "spawn",

        // X3-specific: Atomic execution
        ATOMIC => "atomic",
        BUNDLE => "bundle",
        COMMIT => "commit",
        ROLLBACK => "rollback",

        // X3-specific: MEV/Finance operations
        FLASHLOAN => "flashloan",
        ROUTE => "route",
        SIM => "sim",

        // X3-specific: Chain adapters
        EVM => "evm",
        SVM => "svm",
        BRIDGE => "bridge",

        // X3-specific: Events and effects
        EMIT => "emit",
        LOG => "log",
        ASSERT => "assert",
        REQUIRE => "require",

        // Boolean literals
        TRUE => "true",
        FALSE => "false",

        // Special
        UNDERSCORE => "_",

        // Async
        ASYNC => "async",
        AWAIT => "await",

        // Error handling
        TRY => "try",
        CATCH => "catch",
        THROW => "throw",

        // Memory
        BOX => "box",
        REF => "ref",
        MOVE => "move",

        // Unsafe
        UNSAFE => "unsafe",

        // External
        EXTERN => "extern",
    }
}

/// Pre-defined symbols for primitive types.
pub mod primitives {
    use super::Symbol;
    use std::sync::LazyLock;

    macro_rules! define_primitives {
        ($($name:ident => $str:literal),* $(,)?) => {
            $(
                pub static $name: LazyLock<Symbol> = LazyLock::new(|| Symbol::new($str));
            )*

            /// Check if a symbol is a primitive type.
            pub fn is_primitive(sym: Symbol) -> bool {
                $(
                    if sym == *$name { return true; }
                )*
                false
            }
        };
    }

    define_primitives! {
        // Integer types
        U8 => "u8",
        U16 => "u16",
        U32 => "u32",
        U64 => "u64",
        U128 => "u128",
        U256 => "U256",
        USIZE => "usize",
        I8 => "i8",
        I16 => "i16",
        I32 => "i32",
        I64 => "i64",
        I128 => "i128",
        I256 => "I256",
        ISIZE => "isize",

        // Float types
        F32 => "f32",
        F64 => "f64",

        // Other primitives
        BOOL => "bool",
        CHAR => "char",
        STR => "str",

        // Special types
        UNIT => "()",
        NEVER => "!",

        // X3-specific types
        ADDRESS => "Address",
        BYTES => "Bytes",
        BYTES32 => "Bytes32",
        HASH => "Hash",
        SIGNATURE => "Signature",
        PUBKEY => "PubKey",
        PRIVKEY => "PrivKey",
    }
}
