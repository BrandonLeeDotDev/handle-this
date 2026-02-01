//! Require pattern: `require COND else "msg", rest...`
//!
//! Precondition checks that return early with error if condition fails.

use proc_macro2::{TokenStream, TokenTree};
use quote::quote;
use syn::parse::{Parse, ParseStream};
use syn::ext::IdentExt;
use syn::{Result, Error, Ident, braced, token, Token, LitStr};

/// Parsed require input
struct RequireInput {
    /// The condition expression
    condition: TokenStream,
    /// The error message (literal or expression)
    message: MessageKind,
    /// Optional context expression
    context: Option<TokenStream>,
    /// The rest of the tokens to pass to handle!
    rest: TokenStream,
}

enum MessageKind {
    Literal(LitStr),
    Expr(TokenStream),
}

impl Parse for RequireInput {
    fn parse(input: ParseStream) -> Result<Self> {
        // Collect condition tokens until we hit `else` or `,` (comma suggests missing else)
        let mut cond_tokens = Vec::new();
        let first_span = input.span();
        let mut found_comma = false;

        while !input.is_empty() {
            // Check for `else` keyword
            if input.peek(Ident::peek_any) {
                let fork = input.fork();
                let ident = Ident::parse_any(&fork)?;
                if ident == "else" {
                    break;
                }
            }

            // Check for comma - likely means else was forgotten
            if input.peek(Token![,]) {
                found_comma = true;
                break;
            }

            let tt: TokenTree = input.parse()?;
            cond_tokens.push(tt);
        }

        if cond_tokens.is_empty() {
            return Err(Error::new(input.span(), "expected condition before 'else'"));
        }

        // If we hit a comma without else, that's the error
        if found_comma {
            return Err(Error::new(
                first_span,
                "missing 'else' in require: `require COND else \"message\", ...`",
            ));
        }

        let condition: TokenStream = cond_tokens.into_iter().collect();

        // Expect `else`
        if !input.peek(Ident::peek_any) {
            return Err(Error::new(first_span, "missing 'else' in require: `require COND else \"message\", ...`"));
        }

        let else_kw = Ident::parse_any(input)?;
        if else_kw != "else" {
            return Err(Error::new(else_kw.span(), format!("expected 'else', found '{}'", else_kw)));
        }

        // Parse message: either a string literal or { expr }
        let message = if input.peek(token::Brace) {
            let content;
            braced!(content in input);
            let expr: TokenStream = content.parse()?;
            MessageKind::Expr(expr)
        } else if input.peek(LitStr) {
            let lit: LitStr = input.parse()?;
            MessageKind::Literal(lit)
        } else {
            return Err(Error::new(input.span(), "expected string literal or { expression } after 'else'"));
        };

        // Check for optional `with context`
        let mut context = None;
        if input.peek(Ident::peek_any) {
            let fork = input.fork();
            let ident = Ident::parse_any(&fork)?;
            if ident == "with" {
                // Consume `with`
                let _ = Ident::parse_any(input)?;
                // Collect context expression until `,`
                let mut ctx_tokens = Vec::new();
                while !input.is_empty() && !input.peek(Token![,]) {
                    let tt: TokenTree = input.parse()?;
                    ctx_tokens.push(tt);
                }
                if ctx_tokens.is_empty() {
                    return Err(Error::new(input.span(), "expected context expression after 'with'"));
                }
                context = Some(ctx_tokens.into_iter().collect());
            }
        }

        // Expect comma separator
        if !input.peek(Token![,]) {
            return Err(Error::new(input.span(), "expected ',' after require clause"));
        }
        let _comma: Token![,] = input.parse()?;

        // Rest of the tokens go to handle!
        let rest: TokenStream = input.parse()?;

        if rest.is_empty() {
            return Err(Error::new(input.span(), "expected code after require clause (e.g., 'try { ... }')"));
        }

        Ok(RequireInput {
            condition,
            message,
            context,
            rest,
        })
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

/// Process require pattern.
pub fn process(input: TokenStream) -> Result<TokenStream> {
    let parsed: RequireInput = syn::parse2(input)?;

    // Check for incompatible direct mode
    if let Some(span) = is_direct_mode(&parsed.rest) {
        return Err(Error::new(
            span,
            "`require` cannot be used with direct mode `try -> T { }`. \
             Direct mode guarantees success, but `require` can fail. \
             Use `try { }` instead of `try -> T { }`",
        ));
    }

    Ok(generate(parsed))
}

/// Generate code for require.
fn generate(input: RequireInput) -> TokenStream {
    let condition = &input.condition;
    let rest = &input.rest;

    let error_expr = match (&input.message, &input.context) {
        (MessageKind::Literal(lit), None) => {
            quote! {
                ::handle_this::Handled::msg(#lit).frame(file!(), line!(), column!())
            }
        }
        (MessageKind::Literal(lit), Some(ctx)) => {
            quote! {
                ::handle_this::Handled::msg(#lit).frame(file!(), line!(), column!()).ctx(#ctx)
            }
        }
        (MessageKind::Expr(expr), None) => {
            quote! {
                ::handle_this::Handled::msg(#expr).frame(file!(), line!(), column!())
            }
        }
        (MessageKind::Expr(expr), Some(ctx)) => {
            quote! {
                ::handle_this::Handled::msg(#expr).frame(file!(), line!(), column!()).ctx(#ctx)
            }
        }
    };

    // Wrap in block for #[allow] since attributes on if expressions aren't stable
    quote! {
        {
            #[allow(unreachable_code)]
            if !(#condition) {
                ::core::result::Result::Err(#error_expr)
            } else {
                ::handle_this::handle!(#rest)
            }
        }
    }
}
