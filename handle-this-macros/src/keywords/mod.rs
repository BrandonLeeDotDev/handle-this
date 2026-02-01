//! Keyword modules - each handles parsing and code generation for one keyword.
//!
//! These modules are shared across all pattern handlers.
//!
//! ## Architecture
//!
//! - `parsing` - Shared parsing utilities (type paths, bindings, guards, chain variants)
//! - `codegen` - Code generation engine (typed handlers, guard wrapping)
//! - Each keyword module uses these shared utilities for its specific semantics

pub mod parsing;
pub mod clause;

pub mod catch;
pub mod throw;
pub mod inspect;
pub mod finally;
pub mod with_ctx;
pub mod try_catch;

use proc_macro2::TokenStream;
use syn::Ident;

/// Chain variant for catch/throw/inspect.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChainVariant {
    /// Root error only (default)
    Root,
    /// Search cause chain for first match
    Any,
    /// Collect all matches from cause chain
    All,
}

/// Guard type - either `when` condition or `match` expression.
#[derive(Debug, Clone)]
pub enum Guard {
    /// `when condition`
    When(TokenStream),
    /// `match expr { arms }`
    Match {
        expr: TokenStream,
        arms: TokenStream,
    },
}

/// Context for code generation - shared state across keyword generators.
#[derive(Debug, Clone, Default)]
pub struct GenContext {
    /// Whether generating async code
    pub is_async: bool,
    /// Context expression from `with "context"`
    pub ctx_expr: Option<TokenStream>,
    /// Key-value pairs from `with key: value`
    pub kv_pairs: Vec<(TokenStream, TokenStream)>,
}

impl GenContext {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn async_mode(mut self) -> Self {
        self.is_async = true;
        self
    }
}

/// Helper to check if an identifier is a binding (for catch-all detection).
/// Returns true for lowercase identifiers (e.g., `e`) or underscore-prefixed (e.g., `_e`).
pub fn is_lowercase_ident(ident: &Ident) -> bool {
    let s = ident.to_string();
    s.chars().next().map(|c| c.is_lowercase() || c == '_').unwrap_or(false)
}

/// Helper to peek for a keyword without consuming.
/// Handles both regular identifiers and Rust reserved keywords like `match`, `else`, `if`.
pub fn peek_keyword(input: syn::parse::ParseStream, keyword: &str) -> bool {
    // Special handling for Rust reserved keywords
    match keyword {
        "match" => input.peek(syn::Token![match]),
        "else" => input.peek(syn::Token![else]),
        "if" => input.peek(syn::Token![if]),
        _ => input.peek(Ident) && input.fork().parse::<Ident>().map(|id| id == keyword).unwrap_or(false),
    }
}

/// Parse a keyword, returning error if not found.
/// Handles both regular identifiers and Rust reserved keywords like `match` and `else`.
pub fn parse_keyword(input: syn::parse::ParseStream, keyword: &str) -> syn::Result<Ident> {
    // Special handling for Rust reserved keywords
    match keyword {
        "match" => {
            let token: syn::Token![match] = input.parse()?;
            Ok(Ident::new("match", token.span))
        }
        "else" => {
            let token: syn::Token![else] = input.parse()?;
            Ok(Ident::new("else", token.span))
        }
        _ => {
            let ident: Ident = input.parse()?;
            if ident == keyword {
                Ok(ident)
            } else {
                Err(syn::Error::new(ident.span(), format!("expected `{}`", keyword)))
            }
        }
    }
}
