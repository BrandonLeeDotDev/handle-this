//! Try catch keyword - result-returning error handler.
//!
//! Unlike regular `catch` which always returns `Ok(value)`, `try catch`
//! allows the handler to return either `Ok` or `Err`.
//!
//! Syntax variants:
//! - `try catch { Ok(value) }` - catch-all, no binding
//! - `try catch e { if recoverable { Ok(0) } else { Err(e) } }` - catch-all with binding
//! - `try catch Type(e) { Ok/Err }` - typed catch
//! - `try catch Type(e) when guard { Ok/Err }` - typed with guard
//! - `try catch Type(e) match expr { arms }` - typed with match
//! - `try catch any Type(e) { Ok/Err }` - search cause chain
//! - `try catch all Type |errors| { Ok/Err }` - collect all from chain

use proc_macro2::TokenStream;
use syn::parse::ParseStream;
use syn::{Ident, Result};

use super::{ChainVariant, Guard, parse_keyword};
use super::clause::{parse_clause, ClauseConfig};
use super::parsing;

/// A parsed try catch clause.
#[derive(Debug, Clone)]
pub struct TryCatchClause {
    /// Chain variant: Root, Any, or All
    pub variant: ChainVariant,
    /// Type path (None for catch-all)
    pub type_path: Option<TokenStream>,
    /// Error binding identifier
    pub binding: Ident,
    /// Optional guard (when or match)
    pub guard: Option<Guard>,
    /// Handler body that returns Result
    pub body: TokenStream,
}

/// Parse a try catch clause.
/// Input starts after `try` - expects `catch ...`
pub fn parse(input: ParseStream) -> Result<TryCatchClause> {
    let catch_kw = parse_keyword(input, "catch")?;

    let clause = parse_clause(input, catch_kw.span(), ClauseConfig::try_catch())?;

    Ok(TryCatchClause {
        variant: clause.variant,
        type_path: clause.type_path,
        binding: clause.binding.unwrap_or_else(parsing::underscore_ident),
        guard: clause.guard,
        body: clause.body,
    })
}
