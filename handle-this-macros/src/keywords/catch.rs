//! Catch keyword - error recovery.
//!
//! Syntax variants:
//! - `catch { recovery }` - catch-all, consume binding
//! - `catch e { recovery }` - catch-all with binding
//! - `catch Type(e) { recovery }` - typed catch
//! - `catch Type(e) when guard { recovery }` - typed with guard
//! - `catch Type(e) match expr { arms }` - typed with match
//! - `catch any Type(e) { ... }` - search cause chain
//! - `catch all Type |errors| { ... }` - collect all from chain

use proc_macro2::TokenStream;
use syn::parse::ParseStream;
use syn::{Ident, Result};

use super::{ChainVariant, Guard, parse_keyword};
use super::clause::{parse_clause, ClauseConfig};
use super::parsing;

/// A parsed catch clause.
#[derive(Debug, Clone)]
pub struct CatchClause {
    /// Span of the `catch` keyword (for error reporting)
    pub catch_span: proc_macro2::Span,
    /// Chain variant: Root, Any, or All
    pub variant: ChainVariant,
    /// Type path (None for catch-all)
    pub type_path: Option<TokenStream>,
    /// Error binding identifier
    pub binding: Ident,
    /// Optional guard (when or match)
    pub guard: Option<Guard>,
    /// Recovery body
    pub body: TokenStream,
}

/// Parse a catch clause.
pub fn parse(input: ParseStream) -> Result<CatchClause> {
    let catch_kw = parse_keyword(input, "catch")?;
    let catch_span = catch_kw.span();

    let clause = parse_clause(input, catch_span, ClauseConfig::catch())?;

    Ok(CatchClause {
        catch_span,
        variant: clause.variant,
        type_path: clause.type_path,
        binding: clause.binding.unwrap_or_else(parsing::underscore_ident),
        guard: clause.guard,
        body: clause.body,
    })
}
