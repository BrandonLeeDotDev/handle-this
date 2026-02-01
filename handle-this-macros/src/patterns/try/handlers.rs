//! Shared handler parsing for try patterns.
//!
//! Extracts the common handler fields and parsing logic used by try_for, try_while, try_all.

use proc_macro2::{Span, TokenStream};
use syn::parse::ParseStream;
use syn::Result;

use crate::keywords::{self, peek_keyword, Guard};
use crate::keywords::catch::CatchClause;
use crate::keywords::throw::ThrowClause;
use crate::keywords::inspect::InspectClause;
use crate::keywords::with_ctx::WithClause;
use crate::nested::contains_control_flow;

// Re-export Handler from common for use by iter.rs, retry.rs, etc.
pub use super::common::Handler;

/// Common handler fields for try patterns.
#[derive(Default)]
pub struct Handlers {
    /// All handlers in declaration order
    pub handlers: Vec<Handler>,
    /// Catches (for backwards compatibility and quick access)
    pub catches: Vec<CatchClause>,
    /// Throws (for backwards compatibility and quick access)
    pub throws: Vec<ThrowClause>,
    /// Inspects (for backwards compatibility and quick access)
    pub inspects: Vec<InspectClause>,
    /// Finally block
    pub finally: Option<TokenStream>,
    /// With clause for context
    pub with_clause: Option<WithClause>,
}

impl Handlers {
    /// Check if any handler contains control flow (break/continue/return).
    /// Used to determine whether to use closure-based or direct-mode code generation.
    pub fn has_control_flow(&self) -> bool {
        for handler in &self.handlers {
            match handler {
                Handler::Catch(catch) => {
                    if contains_control_flow(&catch.body) {
                        return true;
                    }
                    if let Some(Guard::Match { arms, .. }) = &catch.guard {
                        if contains_control_flow(arms) {
                            return true;
                        }
                    }
                }
                Handler::Throw(throw) => {
                    if contains_control_flow(&throw.throw_expr) {
                        return true;
                    }
                    if let Some(Guard::Match { arms, .. }) = &throw.guard {
                        if contains_control_flow(arms) {
                            return true;
                        }
                    }
                }
                Handler::Inspect(inspect) => {
                    if contains_control_flow(&inspect.body) {
                        return true;
                    }
                    if let Some(Guard::Match { arms, .. }) = &inspect.guard {
                        if contains_control_flow(arms) {
                            return true;
                        }
                    }
                }
                Handler::TryCatch(try_catch) => {
                    if contains_control_flow(&try_catch.body) {
                        return true;
                    }
                    if let Some(Guard::Match { arms, .. }) = &try_catch.guard {
                        if contains_control_flow(arms) {
                            return true;
                        }
                    }
                }
            }
        }
        false
    }

    /// Check if there's an unconditional catch-all handler.
    pub fn has_catch_all(&self) -> bool {
        // Use the is_untyped_catchall() method from Handler
        self.handlers.iter().any(|h| h.is_untyped_catchall())
    }

}

/// Parse optional handlers: catch, throw, inspect, finally, with.
///
/// Consumes tokens until no more handler keywords are found.
/// Collects multiple handlers and tracks declaration order.
///
/// Supports `else` suffix for typed handlers:
/// - `catch Type(e) { } else { }` - typed catch followed by catch-all
/// - `throw Type(e) { } else { }` - typed throw followed by catch-all throw
pub fn parse(input: ParseStream) -> Result<Handlers> {
    let mut handlers = Handlers::default();

    while !input.is_empty() {
        if peek_keyword(input, "catch") {
            let clause = keywords::catch::parse(input)?;
            handlers.handlers.push(Handler::Catch(clause.clone()));
            handlers.catches.push(clause.clone());

            // Check for `catch Type {} else {}` - creates catch-all after typed catch
            if clause.type_path.is_some() && input.peek(syn::Token![else]) {
                input.parse::<syn::Token![else]>()?;
                let else_body = keywords::parsing::parse_braced_body(input)?;
                let else_clause = CatchClause {
                    catch_span: clause.catch_span,
                    variant: keywords::ChainVariant::Root,
                    type_path: None,
                    binding: keywords::parsing::underscore_ident(),
                    guard: None,
                    body: else_body,
                };
                handlers.handlers.push(Handler::Catch(else_clause.clone()));
                handlers.catches.push(else_clause);
            }
        } else if peek_keyword(input, "throw") {
            let clause = keywords::throw::parse(input)?;
            handlers.handlers.push(Handler::Throw(clause.clone()));
            handlers.throws.push(clause.clone());

            // Check for `throw Type {} else {}` - creates catch-all CATCH after typed throw
            if clause.type_path.is_some() && input.peek(syn::Token![else]) {
                input.parse::<syn::Token![else]>()?;
                let else_body = keywords::parsing::parse_braced_body(input)?;
                // Note: can't use contains_question_mark here - iter/while handlers
                // have different infallibility requirements than sync mode
                let else_clause = CatchClause {
                    catch_span: Span::call_site(),
                    variant: keywords::ChainVariant::Root,
                    type_path: None,
                    binding: keywords::parsing::underscore_ident(),
                    guard: None,
                    body: else_body,
                };
                handlers.handlers.push(Handler::Catch(else_clause.clone()));
                handlers.catches.push(else_clause);
            }
        } else if peek_keyword(input, "inspect") {
            let clause = keywords::inspect::parse(input)?;
            handlers.handlers.push(Handler::Inspect(clause.clone()));
            handlers.inspects.push(clause);
        } else if peek_keyword(input, "finally") {
            let finally_span = input.span();
            if handlers.finally.is_some() {
                return Err(syn::Error::new(
                    finally_span,
                    "multiple `finally` blocks are not allowed; combine into a single block",
                ));
            }
            handlers.finally = Some(keywords::finally::parse(input)?);
        } else if peek_keyword(input, "with") {
            if handlers.with_clause.is_some() {
                return Err(syn::Error::new(
                    input.span(),
                    "multiple `with` clauses are not allowed; combine context into a single `with { ... }`",
                ));
            }
            handlers.with_clause = Some(keywords::with_ctx::parse(input)?);
        } else {
            break;
        }
    }

    Ok(handlers)
}
