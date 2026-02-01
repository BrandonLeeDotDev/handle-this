//! Try while pattern: `try while condition { body } [handlers...]`
//!
//! Retry loop - keeps trying while condition is true.
//!
//! # Signal Mode
//!
//! When handlers contain control flow (`continue`, `break`), this module uses
//! **signal mode** instead of closure mode. Signal mode:
//!
//! 1. Transforms `continue`/`break` in handlers to `LoopSignal::Continue/Break`
//! 2. Returns `Result<LoopSignal<T>, Handled>` from the closure
//! 3. Translates signals to actual control flow at the expansion site
//!
//! This allows typed catches with control flow to **propagate unmatched errors**
//! instead of hitting `unreachable!()`.

use proc_macro2::{TokenStream, TokenTree};
use quote::quote;
use syn::parse::{Parse, ParseStream};
use syn::{Result, braced, token};

use crate::keywords::{self, GenContext};
use crate::nested::transform_nested;
use super::error_handler;
use super::handlers::{self, Handlers};
use super::signal::signal_type;
use super::signal_handler;

/// Parsed try while input.
struct TryWhileInput {
    condition: TokenStream,
    body: TokenStream,
    handlers: Handlers,
}

impl Parse for TryWhileInput {
    fn parse(input: ParseStream) -> Result<Self> {
        // Collect condition tokens until `{`
        let mut cond_tokens = Vec::new();
        while !input.is_empty() && !input.peek(token::Brace) {
            let tt: TokenTree = input.parse()?;
            cond_tokens.push(tt);
        }

        if cond_tokens.is_empty() {
            return Err(syn::Error::new(input.span(), "expected condition before `{`"));
        }

        let condition: TokenStream = cond_tokens.into_iter().collect();

        // Parse body
        let content;
        braced!(content in input);
        let body: TokenStream = content.parse()?;

        // Parse optional handlers
        let handlers = handlers::parse(input)?;

        Ok(TryWhileInput {
            condition,
            body,
            handlers,
        })
    }
}

/// Process try while pattern.
pub fn process(input: TokenStream) -> Result<TokenStream> {
    let parsed: TryWhileInput = syn::parse2(input)?;
    Ok(generate(parsed))
}

/// Generate code for try while (retry loop).
fn generate(input: TryWhileInput) -> TokenStream {
    let mut ctx = GenContext::new();
    if let Some(ref with) = input.handlers.with_clause {
        keywords::with_ctx::apply_to_context(with, &mut ctx);
    }

    let condition = &input.condition;
    let body = transform_nested(input.body.clone());
    let ctx_chain = keywords::with_ctx::gen_ctx_chain(&ctx);

    // Check if handlers contain control flow (break/continue)
    let has_control_flow = input.handlers.has_control_flow();

    // Check if there's an unconditional catch-all handler
    let has_catch_all = input.handlers.has_catch_all();

    let core_logic = if has_control_flow {
        // Use SIGNAL MODE - transforms control flow to signals, allows error propagation
        gen_retry_signal(condition, &body, &input.handlers, &ctx_chain, has_catch_all)
    } else {
        // Use closure mode - better type inference, no control flow
        let error_handler = error_handler::generate_for_loop(&input.handlers, &ctx);
        gen_retry_closure(condition, &body, &error_handler, &ctx_chain)
    };

    let code = if let Some(ref finally_body) = input.handlers.finally {
        let finally_transformed = transform_nested(finally_body.clone());
        keywords::finally::wrap(core_logic, &finally_transformed)
    } else {
        core_logic
    };

    quote! { #code }
}

// ============================================================
// Closure Mode Generator (no control flow)
// ============================================================

/// Generate retry loop in closure mode (better type inference).
fn gen_retry_closure(
    condition: &TokenStream,
    body: &TokenStream,
    error_handler: &TokenStream,
    ctx_chain: &TokenStream,
) -> TokenStream {
    quote! {
        (|| -> ::core::result::Result<_, ::handle_this::Handled> {
            let mut __last_err: ::core::option::Option<::handle_this::Handled> = ::core::option::Option::None;

            loop {
                if !(#condition) {
                    return match __last_err {
                        // __err must be mutable because throw can transform it
                        ::core::option::Option::Some(mut __err) => {
                            #[allow(unreachable_code)]
                            { #error_handler }
                        }
                        ::core::option::Option::None => {
                            // Condition was false on first check - run body once
                            match ::handle_this::__try_block!(#body) {
                                ::core::result::Result::Ok(__v) => ::core::result::Result::Ok(__v),
                                ::core::result::Result::Err(__e) => {
                                    // __err must be mutable because throw can transform it
                                    let mut __err = ::handle_this::__wrap_frame(__e, file!(), line!(), column!()) #ctx_chain;
                                    #[allow(unreachable_code)]
                                    { #error_handler }
                                }
                            }
                        }
                    };
                }

                match ::handle_this::__try_block!(#body) {
                    ::core::result::Result::Ok(__v) => return ::core::result::Result::Ok(__v),
                    ::core::result::Result::Err(__e) => {
                        __last_err = ::core::option::Option::Some(
                            ::handle_this::__wrap_frame(__e, file!(), line!(), column!())
                        );
                    }
                }
            }
        })()
    }
}

// ============================================================
// Signal Mode Generator (control flow via signals)
// ============================================================

/// Generate retry loop in signal mode.
///
/// Returns `Result<LoopSignal<T>, Handled>` from closure, then matches on result
/// to translate signals to actual control flow.
///
/// # Key Differences from Closure Mode
///
/// - Handler `continue` becomes `return Ok(LoopSignal::Continue)`
/// - Handler `break` becomes `return Ok(LoopSignal::Break)`
/// - Unmatched errors: `Err(e)` propagates (typed catch) or `unreachable!()` (catch-all)
fn gen_retry_signal(
    condition: &TokenStream,
    body: &TokenStream,
    handlers: &Handlers,
    ctx_chain: &TokenStream,
    has_catch_all: bool,
) -> TokenStream {
    let signal = signal_type();
    let handler_code = signal_handler::gen_signal_handler(handlers, ctx_chain);

    // For catch-all handlers, errors are always handled, so Err arm is unreachable.
    // For typed handlers, errors may not match, so we propagate them.
    let err_arm = if has_catch_all {
        quote! {
            ::core::result::Result::Err(_) => {
                ::core::unreachable!("catch-all handler should have handled all errors")
            }
        }
    } else {
        quote! {
            ::core::result::Result::Err(__e) => {
                return ::core::result::Result::Err(__e);
            }
        }
    };

    quote! {
        {
            #[allow(unreachable_code)]
            match (|| -> ::core::result::Result<#signal<_>, ::handle_this::Handled> {
                let mut __last_err: ::core::option::Option<::handle_this::Handled> = ::core::option::Option::None;

                loop {
                    if !(#condition) {
                        return match __last_err {
                            // __err must be mutable because throw can transform it
                            ::core::option::Option::Some(mut __err) => {
                                // Handler returns Ok(LoopSignal::*) or Err(e) for unmatched
                                #[allow(unreachable_code)]
                                { #handler_code }
                            }
                            ::core::option::Option::None => {
                                // Condition was false on first check - run body once
                                match ::handle_this::__try_block!(#body) {
                                    ::core::result::Result::Ok(__v) => {
                                        ::core::result::Result::Ok(#signal::Value(__v))
                                    }
                                    ::core::result::Result::Err(__e) => {
                                        // __err must be mutable because throw can transform it
                                        let mut __err = ::handle_this::__wrap_frame(__e, file!(), line!(), column!()) #ctx_chain;
                                        #[allow(unreachable_code)]
                                        { #handler_code }
                                    }
                                }
                            }
                        };
                    }

                    match ::handle_this::__try_block!(#body) {
                        ::core::result::Result::Ok(__v) => {
                            return ::core::result::Result::Ok(#signal::Value(__v));
                        }
                        ::core::result::Result::Err(__e) => {
                            __last_err = ::core::option::Option::Some(
                                ::handle_this::__wrap_frame(__e, file!(), line!(), column!())
                            );
                        }
                    }
                }
            })() {
                ::core::result::Result::Ok(#signal::Value(__v)) => __v,
                ::core::result::Result::Ok(#signal::Continue) => continue,
                ::core::result::Result::Ok(#signal::Break) => break,
                #err_arm
            }
        }
    }
}
