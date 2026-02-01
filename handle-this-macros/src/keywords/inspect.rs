//! Inspect keyword - side effects without recovery.
//!
//! Syntax variants:
//! - `inspect e { side_effect }` - catch-all with binding
//! - `inspect Type(e) { side_effect }` - typed inspect
//! - `inspect Type(e) when guard { side_effect }` - typed with guard
//! - `inspect Type(e) match expr { arms }` - typed with match
//! - `inspect any Type(e) { ... }` - search cause chain
//! - `inspect all Type |errors| { ... }` - collect all from chain

use proc_macro2::TokenStream;
use syn::parse::ParseStream;
use syn::{Ident, Result};

use super::{ChainVariant, Guard, parse_keyword};
use super::clause::{parse_clause, ClauseConfig};
use super::parsing;

/// A parsed inspect clause.
#[derive(Debug, Clone)]
pub struct InspectClause {
    /// Span of the `inspect` keyword (for error reporting)
    pub inspect_span: proc_macro2::Span,
    /// Chain variant: Root, Any, or All
    pub variant: ChainVariant,
    /// Type path (None for catch-all)
    pub type_path: Option<TokenStream>,
    /// Error binding identifier
    pub binding: Ident,
    /// Optional guard (when or match)
    pub guard: Option<Guard>,
    /// Side effect body
    pub body: TokenStream,
}

/// Parse an inspect clause.
pub fn parse(input: ParseStream) -> Result<InspectClause> {
    let inspect_kw = parse_keyword(input, "inspect")?;
    let inspect_span = inspect_kw.span();

    let clause = parse_clause(input, inspect_span, ClauseConfig::inspect())?;

    Ok(InspectClause {
        inspect_span,
        variant: clause.variant,
        type_path: clause.type_path,
        binding: clause.binding.unwrap_or_else(parsing::underscore_ident),
        guard: clause.guard,
        body: clause.body,
    })
}
