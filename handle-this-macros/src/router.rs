//! Pattern router - identifies pattern markers and routes to appropriate handler.

use proc_macro2::{TokenStream, TokenTree, Ident, Span};
use quote::quote;
use syn::{Error, Result};

use crate::patterns::r#try;

/// Route input to the appropriate pattern handler based on marker token.
pub fn route(input: TokenStream) -> Result<TokenStream> {
    let mut iter = input.into_iter().peekable();

    // First token should be the pattern marker
    let marker = match iter.next() {
        Some(TokenTree::Ident(id)) => id,
        Some(other) => {
            return Err(Error::new_spanned(other, "expected pattern marker"));
        }
        None => {
            return Err(Error::new(
                proc_macro2::Span::call_site(),
                "empty input to __handle_proc",
            ));
        }
    };

    let rest: TokenStream = iter.collect();

    match marker.to_string().as_str() {
        "SYNC" => r#try::sync::process(rest),
        "ASYNC" => r#try::async_impl::process(rest),
        "FOR" => r#try::iter::process_for(rest),
        "ANY" => r#try::iter::process_any(rest),
        "ALL" => r#try::iter::process_all(rest),
        "WHILE" => r#try::retry::process(rest),
        "REQUIRE" => crate::patterns::require::process(rest),
        "SCOPE" => crate::patterns::scope::process(rest),
        "WHEN" => r#try::cond::process(rest),
        "THEN" => crate::patterns::then_chain::process(rest),
        // Unified error handler with proper spans
        "ERROR" => {
            let first = rest.into_iter().next();
            let (span, token_str) = first
                .map(|t| (t.span(), t.to_string()))
                .unwrap_or_else(|| (proc_macro2::Span::call_site(), "?".to_string()));

            if token_str == "try" {
                Err(Error::new(span, "`try` requires a body: `try { ... }`"))
            } else {
                Err(Error::new(span, format!("expected `try`, `require`, or `scope`, found `{}`", token_str)))
            }
        }
        "ERROR_EMPTY" => {
            Err(Error::new(proc_macro2::Span::call_site(), "empty handle! block"))
        }
        other => Err(Error::new(
            marker.span(),
            format!("unknown pattern marker: {}", other),
        )),
    }
}

/// Check if token stream contains `, then` after a closing brace.
fn contains_then_after_brace(tokens: &[TokenTree]) -> bool {
    let mut saw_brace = false;
    let mut saw_comma = false;

    for token in tokens {
        match token {
            TokenTree::Group(g) if g.delimiter() == proc_macro2::Delimiter::Brace => {
                saw_brace = true;
                saw_comma = false;
            }
            TokenTree::Punct(p) if p.as_char() == ',' && saw_brace => {
                saw_comma = true;
            }
            TokenTree::Ident(id) if id == "then" && saw_comma => {
                return true;
            }
            _ => {
                if saw_brace && !saw_comma {
                    // Check for `with` after brace - reset comma detection
                    if let TokenTree::Ident(id) = token {
                        if id == "with" {
                            // Continue looking
                            continue;
                        }
                    }
                }
            }
        }
    }
    false
}

/// Route basic try with `with` clause - detect if `then` follows.
/// Input: BASIC { body } with ...
pub fn route_then_or_sync(input: TokenStream) -> Result<TokenStream> {
    let tokens: Vec<TokenTree> = input.clone().into_iter().collect();

    if contains_then_after_brace(&tokens) {
        // Has `then` - route to then_chain
        crate::patterns::then_chain::process(input)
    } else {
        // No `then` - route to sync (strip BASIC marker)
        let mut iter = tokens.into_iter();
        let _marker = iter.next(); // skip BASIC
        let rest: TokenStream = iter.collect();
        r#try::sync::process(rest)
    }
}

/// Route iteration patterns - detect if `then` follows after body.
/// Input: FOR/ANY/ALL/WHILE ...
pub fn route_then_or_iter(input: TokenStream) -> Result<TokenStream> {
    let tokens: Vec<TokenTree> = input.clone().into_iter().collect();

    if contains_then_after_brace(&tokens) {
        // Has `then` - route to then_chain with marker prefix
        let mut iter = input.into_iter();
        let marker = iter.next().unwrap(); // FOR/ANY/ALL/WHILE
        let rest: TokenStream = iter.collect();

        // Prepend THEN marker for then_chain
        let then_marker = Ident::new("THEN", Span::call_site());
        let combined = quote! { #then_marker #marker #rest };
        crate::patterns::then_chain::process(combined)
    } else {
        // No `then` - route to regular iter/retry
        let mut iter = input.into_iter();
        let marker = match iter.next() {
            Some(TokenTree::Ident(id)) => id,
            _ => return Err(Error::new(Span::call_site(), "expected pattern marker")),
        };
        let rest: TokenStream = iter.collect();

        match marker.to_string().as_str() {
            "FOR" => r#try::iter::process_for(rest),
            "ANY" => r#try::iter::process_any(rest),
            "ALL" => r#try::iter::process_all(rest),
            "WHILE" => r#try::retry::process(rest),
            other => Err(Error::new(marker.span(), format!("unknown iter marker: {}", other))),
        }
    }
}
