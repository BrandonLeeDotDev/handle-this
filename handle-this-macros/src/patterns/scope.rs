//! Scope pattern: `scope "name", rest...`
//!
//! Named context that wraps errors with additional frame information.
//!
//! Supports:
//! - `scope "name", try { ... }` - just scope name
//! - `scope "name", { key: value }, try { ... }` - scope with structured data

use proc_macro2::{TokenStream, TokenTree};
use quote::quote;
use syn::parse::{Parse, ParseStream};
use syn::{Result, Error, Expr, Ident, LitStr, Token, braced};

/// A key-value pair for structured scope context.
#[derive(Clone)]
struct KvPair {
    key: Ident,
    value: Expr,
}

/// Parsed scope input
struct ScopeInput {
    /// The scope name (context message)
    name: LitStr,
    /// Optional key-value pairs (inside braces)
    kv_pairs: Vec<KvPair>,
    /// The rest of the tokens to pass to handle!
    rest: TokenStream,
}

/// Parse key-value pairs from inside braces: { key: value, key2: value2 }
fn parse_kv_braced(input: ParseStream) -> Result<Vec<KvPair>> {
    let content;
    braced!(content in input);

    let mut pairs = Vec::new();
    while !content.is_empty() {
        let key: Ident = content.parse()?;
        content.parse::<Token![:]>()?;
        let value: Expr = content.parse()?;
        pairs.push(KvPair { key, value });

        if content.peek(Token![,]) {
            content.parse::<Token![,]>()?;
        } else {
            break;
        }
    }
    Ok(pairs)
}

impl Parse for ScopeInput {
    fn parse(input: ParseStream) -> Result<Self> {
        // Expect string literal for scope name
        let name: LitStr = input.parse()?;

        // Expect comma separator
        if !input.peek(Token![,]) {
            return Err(Error::new(name.span(), "expected ',' after scope name: `scope \"name\", try { ... }`"));
        }
        input.parse::<Token![,]>()?;

        // Check for optional { key: value } block
        let kv_pairs = if input.peek(syn::token::Brace) {
            let pairs = parse_kv_braced(input)?;
            // Expect comma after braces
            if input.peek(Token![,]) {
                input.parse::<Token![,]>()?;
            }
            pairs
        } else {
            Vec::new()
        };

        // Rest of the tokens go to handle!
        let rest: TokenStream = input.parse()?;

        if rest.is_empty() {
            return Err(Error::new(input.span(), "expected code after scope (e.g., 'try { ... }')"));
        }

        Ok(ScopeInput { name, kv_pairs, rest })
    }
}

/// Check if a token stream starts with `try ->` (direct mode).
/// Returns the span of `try` if found, for error reporting.
fn is_direct_mode(rest: &TokenStream) -> Option<proc_macro2::Span> {
    let mut iter = rest.clone().into_iter();

    // Look for `try` keyword
    let try_span = match iter.next() {
        Some(TokenTree::Ident(id)) if id.to_string() == "try" => id.span(),
        _ => return None,
    };

    // Look for `->` (Punct '-' followed by Punct '>')
    match iter.next() {
        Some(TokenTree::Punct(p)) if p.as_char() == '-' => {}
        _ => return None,
    }
    match iter.next() {
        Some(TokenTree::Punct(p)) if p.as_char() == '>' => {}
        _ => return None,
    }

    Some(try_span)
}

/// Process scope pattern.
pub fn process(input: TokenStream) -> Result<TokenStream> {
    let parsed: ScopeInput = syn::parse2(input)?;

    // Check for incompatible direct mode
    if let Some(span) = is_direct_mode(&parsed.rest) {
        return Err(Error::new(
            span,
            "`scope` cannot be used with direct mode `try -> T { }`. \
             Direct mode guarantees success, but `scope` wraps errors. \
             Use `try { }` instead of `try -> T { }`",
        ));
    }

    Ok(generate(parsed))
}

/// Generate code for scope.
fn generate(input: ScopeInput) -> TokenStream {
    let name = &input.name;
    let rest = &input.rest;

    // Build kv chain
    let kv_chain: TokenStream = input.kv_pairs.iter().map(|kv| {
        let key = &kv.key;
        let value = &kv.value;
        quote! { .kv(stringify!(#key), #value) }
    }).collect();

    // Scope wraps the inner result, adding context on error
    quote! {
        {
            let __scope_result = ::handle_this::handle!(#rest);
            match __scope_result {
                ::core::result::Result::Ok(__v) => ::core::result::Result::Ok(__v),
                ::core::result::Result::Err(__e) => ::core::result::Result::Err(
                    __e.frame(file!(), line!(), column!()).ctx(#name) #kv_chain
                ),
            }
        }
    }
}
