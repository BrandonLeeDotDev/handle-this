//! With context keyword - adds context and key-value pairs to errors.
//!
//! Syntax:
//! - `with "context message"`
//! - `with { key: value }`
//! - `with "context", { key: value, key2: value2 }`

use proc_macro2::TokenStream;
use quote::quote;
use syn::parse::ParseStream;
use syn::{braced, Expr, Ident, Result, Token};

use super::{parse_keyword, GenContext};

/// A key-value pair for structured error context.
#[derive(Debug, Clone)]
pub struct KvPair {
    pub key: TokenStream,
    pub value: TokenStream,
}

/// Parsed with clause.
#[derive(Debug, Clone, Default)]
pub struct WithClause {
    pub context: Option<Expr>,
    pub kv_pairs: Vec<KvPair>,
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
        pairs.push(KvPair {
            key: quote! { stringify!(#key) },
            value: quote! { #value },
        });

        if content.peek(Token![,]) {
            content.parse::<Token![,]>()?;
        } else {
            break;
        }
    }
    Ok(pairs)
}

/// Parse a with clause.
///
/// Supports:
/// - `with "context"`
/// - `with { key: value }`
/// - `with "context", { key: value }`
pub fn parse(input: ParseStream) -> Result<WithClause> {
    parse_keyword(input, "with")?;

    let mut clause = WithClause::default();

    // First item could be context string or braced kv pairs
    if input.peek(syn::LitStr) {
        let ctx: Expr = input.parse()?;
        clause.context = Some(ctx);

        // Check for comma and optional braced kv pairs
        if input.peek(Token![,]) {
            input.parse::<Token![,]>()?;
            // Check for braced kv pairs after comma
            if input.peek(syn::token::Brace) {
                clause.kv_pairs = parse_kv_braced(input)?;
            }
        }
    } else if input.peek(syn::token::Brace) {
        // Just braced kv pairs, no context message
        clause.kv_pairs = parse_kv_braced(input)?;
    }

    Ok(clause)
}

/// Apply context to a GenContext.
pub fn apply_to_context(with_clause: &WithClause, ctx: &mut GenContext) {
    if let Some(ref context) = with_clause.context {
        ctx.ctx_expr = Some(quote! { #context });
    }
    for kv in &with_clause.kv_pairs {
        let key = &kv.key;
        let value = &kv.value;
        ctx.kv_pairs.push((quote! { #key }, quote! { #value }));
    }
}

/// Generate context/kv method chain for error wrapping.
pub fn gen_ctx_chain(ctx: &GenContext) -> TokenStream {
    let mut chain = TokenStream::new();

    if let Some(ref ctx_expr) = ctx.ctx_expr {
        chain.extend(quote! { .ctx(#ctx_expr) });
    }

    for (key, value) in &ctx.kv_pairs {
        chain.extend(quote! { .kv(#key, #value) });
    }

    chain
}
