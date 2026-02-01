//! Throw keyword - error transformation.
//!
//! Syntax variants:
//! - `throw { new_error }` - catch-all transform (no binding)
//! - `throw e { new_error }` - catch-all with binding
//! - `throw _ { new_error }` - catch-all with explicit discard
//! - `throw Type(e) { new_error }` - typed transform
//! - `throw Type(e) when guard { new_error }` - typed with guard
//! - `throw Type(e) match expr { arms }` - typed with match
//! - `throw any Type(e) { ... }` - search cause chain
//! - `throw all Type |errors| { ... }` - collect all from chain

use proc_macro2::TokenStream;
use syn::parse::ParseStream;
use syn::{Ident, Result};

use super::{ChainVariant, Guard, parse_keyword};
use super::clause::{parse_clause, ClauseConfig};

/// A parsed throw clause.
#[derive(Debug, Clone)]
pub struct ThrowClause {
    /// Chain variant: Root, Any, or All
    pub variant: ChainVariant,
    /// Type path (None for catch-all)
    pub type_path: Option<TokenStream>,
    /// Error binding identifier (optional for no-binding throw)
    pub binding: Option<Ident>,
    /// Optional guard (when or match)
    pub guard: Option<Guard>,
    /// Transform expression (the new error to throw)
    pub throw_expr: TokenStream,
}

/// Parse a throw clause.
pub fn parse(input: ParseStream) -> Result<ThrowClause> {
    let throw_kw = parse_keyword(input, "throw")?;

    let clause = parse_clause(input, throw_kw.span(), ClauseConfig::throw())?;

    Ok(ThrowClause {
        variant: clause.variant,
        type_path: clause.type_path,
        binding: clause.binding,
        guard: clause.guard,
        throw_expr: clause.body,
    })
}
