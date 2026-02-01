//! Try when pattern: `try when CONDITION { body } else { fallback } handlers...`
//!
//! Conditional execution that picks between branches based on condition.

use proc_macro2::{TokenStream, TokenTree};
use quote::quote;
use syn::parse::{Parse, ParseStream};
use syn::ext::IdentExt;
use syn::{Result, Error, Ident, braced};

use crate::keywords::{self, peek_keyword};
use crate::keywords::catch::CatchClause;
use crate::keywords::throw::ThrowClause;
use crate::keywords::inspect::InspectClause;
use crate::nested;

/// Parsed try when input
struct TryWhenInput {
    /// The condition expression
    condition: TokenStream,
    /// The body to execute if condition is true
    body: TokenStream,
    /// Else-when branches: (condition, body)
    else_whens: Vec<(TokenStream, TokenStream)>,
    /// The else body (optional if handlers provided)
    else_body: Option<TokenStream>,
    /// Optional catch clauses
    catches: Vec<CatchClause>,
    /// Optional throw clauses
    throws: Vec<ThrowClause>,
    /// Optional inspect clauses
    inspects: Vec<InspectClause>,
    /// Optional finally body
    finally: Option<TokenStream>,
}

impl Parse for TryWhenInput {
    fn parse(input: ParseStream) -> Result<Self> {
        // Collect condition tokens until `{`
        let mut cond_tokens = Vec::new();

        while !input.is_empty() && !input.peek(syn::token::Brace) {
            let tt: TokenTree = input.parse()?;
            cond_tokens.push(tt);
        }

        if cond_tokens.is_empty() {
            return Err(Error::new(input.span(), "expected condition after 'when'"));
        }

        let condition: TokenStream = cond_tokens.into_iter().collect();

        // Parse body in braces
        let body_content;
        braced!(body_content in input);
        let body: TokenStream = body_content.parse()?;

        // Parse else-when chains and else body
        let mut else_whens = Vec::new();
        let mut else_body = None;

        while !input.is_empty() {
            // Check for `else` keyword
            if input.peek(Ident::peek_any) {
                let fork = input.fork();
                let ident = Ident::parse_any(&fork)?;
                if ident == "else" {
                    // Consume `else`
                    let _ = Ident::parse_any(input)?;

                    // Check if `else when` or just `else`
                    if input.peek(Ident::peek_any) {
                        let fork2 = input.fork();
                        let next_ident = Ident::parse_any(&fork2)?;
                        if next_ident == "when" {
                            // `else when CONDITION { body }`
                            let _ = Ident::parse_any(input)?; // consume `when`

                            // Collect condition tokens until `{`
                            let mut ew_cond_tokens = Vec::new();
                            while !input.is_empty() && !input.peek(syn::token::Brace) {
                                let tt: TokenTree = input.parse()?;
                                ew_cond_tokens.push(tt);
                            }

                            if ew_cond_tokens.is_empty() {
                                return Err(Error::new(input.span(), "expected condition after 'else when'"));
                            }

                            let ew_condition: TokenStream = ew_cond_tokens.into_iter().collect();

                            let ew_body_content;
                            braced!(ew_body_content in input);
                            let ew_body: TokenStream = ew_body_content.parse()?;

                            else_whens.push((ew_condition, ew_body));
                            continue;
                        }
                    }

                    // Just `else { body }`
                    if input.peek(syn::token::Brace) {
                        let else_content;
                        braced!(else_content in input);
                        else_body = Some(else_content.parse()?);
                    } else {
                        return Err(Error::new(input.span(), "expected '{' after 'else'"));
                    }
                    break;
                }
                // Not `else`, break to parse handlers
                break;
            } else {
                break;
            }
        }

        // Parse optional handlers
        let mut catches = Vec::new();
        let mut throws = Vec::new();
        let mut inspects = Vec::new();
        let mut finally = None;

        while !input.is_empty() {
            if peek_keyword(input, "catch") {
                catches.push(keywords::catch::parse(input)?);
            } else if peek_keyword(input, "throw") {
                throws.push(keywords::throw::parse(input)?);
            } else if peek_keyword(input, "inspect") {
                inspects.push(keywords::inspect::parse(input)?);
            } else if peek_keyword(input, "finally") {
                finally = Some(keywords::finally::parse(input)?);
            } else if input.peek(Ident::peek_any) {
                let ident: Ident = input.parse()?;
                return Err(Error::new(ident.span(), format!("unexpected keyword: {}", ident)));
            } else {
                let tt: TokenTree = input.parse()?;
                return Err(Error::new_spanned(tt, "unexpected token"));
            }
        }

        Ok(TryWhenInput {
            condition,
            body,
            else_whens,
            else_body,
            catches,
            throws,
            inspects,
            finally,
        })
    }
}

/// Process try when pattern.
pub fn process(input: TokenStream) -> Result<TokenStream> {
    let parsed: TryWhenInput = syn::parse2(input)?;
    Ok(generate(parsed))
}

/// Generate code for try when.
fn generate(input: TryWhenInput) -> TokenStream {
    let condition = &input.condition;
    let body = nested::transform_nested(input.body.clone());
    let else_whens = &input.else_whens;
    let else_body = input.else_body.as_ref().map(|b| nested::transform_nested(b.clone()));

    // Build the if-else chain
    let mut if_chain = quote! {
        if #condition {
            ::handle_this::__try_block!(#body)
        }
    };

    for (ew_cond, ew_body) in else_whens {
        let transformed_body = nested::transform_nested(ew_body.clone());
        if_chain = quote! {
            #if_chain else if #ew_cond {
                ::handle_this::__try_block!(#transformed_body)
            }
        };
    }

    // Add else branch
    if let Some(eb) = else_body {
        if_chain = quote! {
            #if_chain else {
                ::handle_this::__try_block!(#eb)
            }
        };
    } else {
        // No else body - return unit as success
        if_chain = quote! {
            #if_chain else {
                ::core::result::Result::Ok(())
            }
        };
    }

    // Check if we have any handlers
    let has_handlers = !input.catches.is_empty()
        || !input.throws.is_empty()
        || !input.inspects.is_empty();

    let code = if has_handlers {
        // Use or_else pattern for type unification (like sync_try)
        let handler_checks = generate_handler_checks(&input);

        quote! {
            #if_chain.or_else(|__raw_err| -> ::core::result::Result<_, ::handle_this::Handled> {
                #[allow(unreachable_code)]
                {
                    // __err must be mutable because throw can transform it
                    let mut __err = ::handle_this::__wrap_frame(__raw_err, file!(), line!(), column!());
                    #handler_checks
                }
            })
        }
    } else {
        // No handlers - just wrap error with frame
        quote! {
            #if_chain
                .map_err(|__e| ::handle_this::__wrap_frame(__e, file!(), line!(), column!()))
        }
    };

    // Wrap with finally if present
    let code = if let Some(ref finally_body) = input.finally {
        let finally_transformed = nested::transform_nested(finally_body.clone());
        keywords::finally::wrap(code, &finally_transformed)
    } else {
        code
    };

    quote! { { #code } }
}

/// Generate handler checks using the same pattern as sync.
fn generate_handler_checks(input: &TryWhenInput) -> TokenStream {
    use super::sync::{generate_catch_checks, generate_throw_checks, generate_inspect_checks};

    let mut checks = Vec::new();
    checks.extend(generate_catch_checks(&input.catches));
    checks.extend(generate_throw_checks(&input.throws));
    checks.extend(generate_inspect_checks(&input.inspects));

    // Fallback: propagate error
    let fallback = quote! {
        ::core::result::Result::Err(__err)
    };

    quote! {
        #(#checks)*
        #fallback
    }
}
