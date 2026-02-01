//! Shared parsing utilities for keyword modules.
//!
//! Provides common parsing functions used across catch, throw, inspect, and try_catch.

use proc_macro2::{TokenStream, TokenTree};
use quote::quote;
use syn::parse::ParseStream;
use syn::{Ident, Result, braced, token};

use super::{ChainVariant, Guard, peek_keyword, parse_keyword};

/// Parse chain variant (any/all) if present.
pub fn parse_chain_variant(input: ParseStream) -> Result<ChainVariant> {
    if peek_keyword(input, "any") {
        input.parse::<Ident>()?;
        Ok(ChainVariant::Any)
    } else if peek_keyword(input, "all") {
        input.parse::<Ident>()?;
        Ok(ChainVariant::All)
    } else {
        Ok(ChainVariant::Root)
    }
}

/// Parse type path (handles paths like `std::io::Error`).
pub fn parse_type_path(input: ParseStream) -> Result<TokenStream> {
    let mut tokens = Vec::new();

    // First segment
    let ident: Ident = input.parse()?;
    tokens.push(quote! { #ident });

    // Additional path segments
    while input.peek(syn::Token![::]) {
        input.parse::<syn::Token![::]>()?;
        let seg: Ident = input.parse()?;
        tokens.push(quote! { :: #seg });
    }

    Ok(tokens.into_iter().collect())
}

/// Reserved internal binding names that would conflict with generated code.
const RESERVED_BINDINGS: &[&str] = &[
    "__err", "__signal", "__signal_value", "__new_err", "__result", "__ok_value",
];

/// Check if a binding name is reserved for internal use.
pub fn is_reserved_binding(name: &str) -> bool {
    RESERVED_BINDINGS.contains(&name) || name.starts_with("__handle_")
}

/// Parse binding: `(e)` or `(_)` for Root/Any, `|errors|` for All.
pub fn parse_binding(input: ParseStream, variant: ChainVariant) -> Result<Ident> {
    let binding = match variant {
        ChainVariant::All => {
            // |errors| - but check for common mistake of (e)
            if input.peek(syn::token::Paren) {
                return Err(syn::Error::new(
                    input.span(),
                    "`all` uses `|binding|` not `(binding)`: `catch all Type |errors| { ... }`",
                ));
            }
            input.parse::<syn::Token![|]>()?;
            let binding: Ident = input.parse()?;
            input.parse::<syn::Token![|]>()?;
            binding
        }
        _ => {
            // (e) or (_)
            let content;
            syn::parenthesized!(content in input);
            // Check for underscore keyword
            if content.peek(syn::Token![_]) {
                content.parse::<syn::Token![_]>()?;
                Ident::new("_", proc_macro2::Span::call_site())
            } else {
                content.parse()?
            }
        }
    };

    // Check for reserved internal names
    let name = binding.to_string();
    if is_reserved_binding(&name) {
        return Err(syn::Error::new(
            binding.span(),
            format!("`{}` is reserved for internal use; choose a different binding name", name),
        ));
    }

    Ok(binding)
}

/// Parse optional guard (when or match).
pub fn parse_optional_guard(input: ParseStream) -> Result<Option<Guard>> {
    if peek_keyword(input, "when") {
        let condition = parse_when_condition(input)?;
        Ok(Some(Guard::When(condition)))
    } else if peek_keyword(input, "match") {
        let (expr, arms) = parse_match_clause(input)?;
        Ok(Some(Guard::Match { expr, arms }))
    } else {
        Ok(None)
    }
}

/// Parse a `when condition` guard.
/// Collects tokens until we hit a `{` (body) or `match` keyword.
fn parse_when_condition(input: ParseStream) -> Result<TokenStream> {
    parse_keyword(input, "when")?;

    let mut tokens = Vec::new();

    while !input.is_empty() {
        // Stop at brace (body start)
        if input.peek(token::Brace) {
            break;
        }
        // Stop at match keyword
        if peek_keyword(input, "match") {
            break;
        }
        let tt: TokenTree = input.parse()?;
        tokens.push(tt);
    }

    if tokens.is_empty() {
        return Err(syn::Error::new(input.span(), "expected condition after `when`"));
    }

    Ok(tokens.into_iter().collect())
}

/// Parse a `match expr { arms }` clause.
/// Returns (expr, arms).
fn parse_match_clause(input: ParseStream) -> Result<(TokenStream, TokenStream)> {
    let match_span = input.span();
    parse_keyword(input, "match")?;

    // Collect expression tokens until `{`
    let mut expr_tokens = Vec::new();
    while !input.is_empty() && !input.peek(token::Brace) {
        let tt: TokenTree = input.parse()?;
        expr_tokens.push(tt);
    }

    if expr_tokens.is_empty() {
        return Err(syn::Error::new(
            match_span,
            "expected expression after `match`: `catch Type(e) match e { ... }`",
        ));
    }

    let expr: TokenStream = expr_tokens.into_iter().collect();

    // Parse arms
    let arms_content;
    braced!(arms_content in input);
    let arms: TokenStream = arms_content.parse()?;

    if arms.is_empty() {
        return Err(syn::Error::new(
            match_span,
            "match arms cannot be empty",
        ));
    }

    Ok((expr, arms))
}

/// Parse a braced body `{ ... }`.
pub fn parse_braced_body(input: ParseStream) -> Result<TokenStream> {
    if !input.peek(token::Brace) {
        return Err(syn::Error::new(
            input.span(),
            "expected `{ }` body for handler",
        ));
    }
    let content;
    braced!(content in input);
    content.parse()
}

/// Check if looking at a braced block.
pub fn peek_brace(input: ParseStream) -> bool {
    input.peek(token::Brace)
}

/// Create an underscore identifier.
pub fn underscore_ident() -> Ident {
    Ident::new("_", proc_macro2::Span::call_site())
}
