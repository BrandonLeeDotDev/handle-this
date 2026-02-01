//! Iteration-based try patterns.
//!
//! - `try for item in iter { body }` - first success wins (alias: try any)
//! - `try all item in iter { body }` - collect all, fail on any error
//!
//! # Signal Mode
//!
//! When handlers contain control flow (`continue`, `break`), this module uses
//! **signal mode** instead of direct mode. Signal mode:
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
use syn::ext::IdentExt;
use syn::{Result, Ident, braced, token};

use crate::keywords::{self, GenContext};
use crate::nested::transform_nested;
use super::error_handler;
use super::handlers::{self, Handlers};
use super::signal::signal_type;
use super::signal_handler;

/// Iteration mode.
#[derive(Clone, Copy, PartialEq)]
pub enum IterMode {
    /// First success wins, chain errors
    FirstSuccess,
    /// Collect all results, fail on any error
    CollectAll,
}

/// Parsed iteration input (shared by for/any/all).
struct IterInput {
    binding: Ident,
    iterator: TokenStream,
    body: TokenStream,
    handlers: Handlers,
}

impl Parse for IterInput {
    fn parse(input: ParseStream) -> Result<Self> {
        // Parse: binding in iterator { body }
        // Use parse_any to allow `_` as a binding
        let binding = Ident::parse_any(input)?;
        input.parse::<syn::Token![in]>()?;

        // Collect iterator tokens until `{`
        let mut iter_tokens = Vec::new();
        while !input.is_empty() && !input.peek(token::Brace) {
            let tt: TokenTree = input.parse()?;
            iter_tokens.push(tt);
        }
        if iter_tokens.is_empty() {
            return Err(syn::Error::new(
                input.span(),
                "missing iterator expression: `try for x in ITERATOR { ... }`",
            ));
        }
        let iterator: TokenStream = iter_tokens.into_iter().collect();

        // Parse body
        let content;
        braced!(content in input);
        let body: TokenStream = content.parse()?;

        // Parse optional handlers
        let handlers = handlers::parse(input)?;

        Ok(IterInput { binding, iterator, body, handlers })
    }
}

/// Process try for pattern (first success).
pub fn process_for(input: TokenStream) -> Result<TokenStream> {
    let parsed: IterInput = syn::parse2(input)?;
    Ok(generate(parsed, IterMode::FirstSuccess))
}

/// Process try any pattern (alias for try for).
pub fn process_any(input: TokenStream) -> Result<TokenStream> {
    process_for(input)
}

/// Process try all pattern (collect all).
pub fn process_all(input: TokenStream) -> Result<TokenStream> {
    let parsed: IterInput = syn::parse2(input)?;
    Ok(generate(parsed, IterMode::CollectAll))
}

/// Generate code for iteration pattern.
fn generate(input: IterInput, mode: IterMode) -> TokenStream {
    let mut ctx = GenContext::new();
    if let Some(ref with) = input.handlers.with_clause {
        keywords::with_ctx::apply_to_context(with, &mut ctx);
    }

    let binding = &input.binding;
    let iterator = &input.iterator;
    let body = transform_nested(input.body.clone());
    let ctx_chain = keywords::with_ctx::gen_ctx_chain(&ctx);

    // Check if handlers contain control flow (break/continue)
    let has_control_flow = input.handlers.has_control_flow();

    // Check if there's an unconditional catch-all handler
    let has_catch_all = input.handlers.has_catch_all();

    let core_logic = if has_control_flow {
        // Use SIGNAL MODE - transforms control flow to signals, allows error propagation
        match mode {
            IterMode::FirstSuccess => gen_first_success_signal(
                binding, iterator, &body, &input.handlers, &ctx, &ctx_chain, has_catch_all,
            ),
            IterMode::CollectAll => gen_collect_all_signal(
                binding, iterator, &body, &input.handlers, &ctx, &ctx_chain, has_catch_all,
            ),
        }
    } else {
        // Use closure mode - better type inference, no control flow
        let error_handler = error_handler::generate_for_loop(&input.handlers, &ctx);

        match mode {
            IterMode::FirstSuccess => gen_first_success(binding, iterator, &body, &error_handler, &ctx_chain),
            IterMode::CollectAll => gen_collect_all(binding, iterator, &body, &error_handler),
        }
    };

    // Wrap with finally
    let code = if let Some(ref finally_body) = input.handlers.finally {
        let finally_transformed = transform_nested(finally_body.clone());
        keywords::finally::wrap(core_logic, &finally_transformed)
    } else {
        core_logic
    };

    quote! { #code }
}

// ============================================================
// Closure Mode Generators (no control flow)
// ============================================================

/// Generate first-success iteration (try for / try any).
fn gen_first_success(
    binding: &Ident,
    iterator: &TokenStream,
    body: &TokenStream,
    error_handler: &TokenStream,
    ctx_chain: &TokenStream,
) -> TokenStream {
    quote! {
        (|| -> ::core::result::Result<_, ::handle_this::Handled> {
            let mut __chained_err: ::core::option::Option<::handle_this::Handled> = ::core::option::Option::None;

            for #binding in #iterator {
                match ::handle_this::__try_block!(#body) {
                    ::core::result::Result::Ok(__v) => {
                        #[allow(unreachable_code)]
                        return ::core::result::Result::Ok(__v);
                    }
                    ::core::result::Result::Err(__e) => {
                        let __current = ::handle_this::__wrap_frame(__e, file!(), line!(), column!());
                        __chained_err = ::core::option::Option::Some(match __chained_err {
                            ::core::option::Option::Some(__prev) => __current.chain_after(__prev),
                            ::core::option::Option::None => __current,
                        });
                    }
                }
            }

            // __err must be mutable because throw can transform it
            let mut __err = __chained_err.unwrap_or_else(||
                ::handle_this::Handled::msg("empty iterator in try for")
                    .frame(file!(), line!(), column!())
                    #ctx_chain
            );
            #[allow(unreachable_code)]
            { #error_handler }
        })()
    }
}

/// Generate collect-all iteration (try all).
fn gen_collect_all(
    binding: &Ident,
    iterator: &TokenStream,
    body: &TokenStream,
    error_handler: &TokenStream,
) -> TokenStream {
    quote! {
        (|| -> ::core::result::Result<_, ::handle_this::Handled> {
            let mut __results = ::std::vec::Vec::new();
            let mut __error: ::core::option::Option<::handle_this::Handled> = ::core::option::Option::None;

            for #binding in #iterator {
                match ::handle_this::__try_block!(#body) {
                    ::core::result::Result::Ok(__v) => {
                        __results.push(__v);
                    }
                    ::core::result::Result::Err(__e) => {
                        let __wrapped = ::handle_this::__wrap_frame(__e, file!(), line!(), column!());
                        __error = ::core::option::Option::Some(match __error {
                            ::core::option::Option::Some(__prev) => __wrapped.chain_after(__prev),
                            ::core::option::Option::None => __wrapped,
                        });
                    }
                }
            }

            match __error {
                // __err must be mutable because throw can transform it
                ::core::option::Option::Some(mut __err) => {
                    #[allow(unreachable_code)]
                    { #error_handler }
                }
                ::core::option::Option::None => {
                    ::core::result::Result::Ok(__results)
                }
            }
        })()
    }
}

// ============================================================
// Signal Mode Generators (control flow via signals)
// ============================================================

/// Generate first-success iteration in signal mode.
///
/// Returns `Result<LoopSignal<T>, Handled>` from closure, then matches on result
/// to translate signals to actual control flow.
///
/// # Key Differences from Direct Mode
///
/// - Handler `continue` becomes `return Ok(LoopSignal::Continue)`
/// - Handler `break` becomes `return Ok(LoopSignal::Break)`
/// - Unmatched errors: `Err(e)` propagates (typed catch) or `unreachable!()` (catch-all)
fn gen_first_success_signal(
    binding: &Ident,
    iterator: &TokenStream,
    body: &TokenStream,
    handlers: &Handlers,
    _ctx: &GenContext,
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
                let mut __chained_err: ::core::option::Option<::handle_this::Handled> = ::core::option::Option::None;

                for #binding in #iterator {
                    match ::handle_this::__try_block!(#body) {
                        ::core::result::Result::Ok(__v) => {
                            #[allow(unreachable_code)]
                            return ::core::result::Result::Ok(#signal::Value(__v));
                        }
                        ::core::result::Result::Err(__e) => {
                            let __current = ::handle_this::__wrap_frame(__e, file!(), line!(), column!());
                            __chained_err = ::core::option::Option::Some(match __chained_err {
                                ::core::option::Option::Some(__prev) => __current.chain_after(__prev),
                                ::core::option::Option::None => __current,
                            });
                        }
                    }
                }

                // All iterations failed (or iterator was empty)
                // __err must be mutable because throw can transform it
                let mut __err = __chained_err.unwrap_or_else(||
                    ::handle_this::Handled::msg("empty iterator in try for")
                        .frame(file!(), line!(), column!())
                        #ctx_chain
                );

                // Handler returns Ok(LoopSignal::*) or Err(e) for unmatched
                #[allow(unreachable_code)]
                { #handler_code }
            })() {
                ::core::result::Result::Ok(#signal::Value(__v)) => __v,
                ::core::result::Result::Ok(#signal::Continue) => continue,
                ::core::result::Result::Ok(#signal::Break) => break,
                #err_arm
            }
        }
    }
}

/// Generate collect-all iteration in signal mode.
fn gen_collect_all_signal(
    binding: &Ident,
    iterator: &TokenStream,
    body: &TokenStream,
    handlers: &Handlers,
    _ctx: &GenContext,
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
                let mut __results = ::std::vec::Vec::new();
                let mut __error: ::core::option::Option<::handle_this::Handled> = ::core::option::Option::None;

                for #binding in #iterator {
                    match ::handle_this::__try_block!(#body) {
                        ::core::result::Result::Ok(__v) => {
                            __results.push(__v);
                        }
                        ::core::result::Result::Err(__e) => {
                            let __wrapped = ::handle_this::__wrap_frame(__e, file!(), line!(), column!())
                                #ctx_chain;
                            __error = ::core::option::Option::Some(match __error {
                                ::core::option::Option::Some(__prev) => __wrapped.chain_after(__prev),
                                ::core::option::Option::None => __wrapped,
                            });
                        }
                    }
                }

                match __error {
                    // __err must be mutable because throw can transform it
                    ::core::option::Option::Some(mut __err) => {
                        // Handler returns Ok(LoopSignal::*) or Err(e) for unmatched
                        #[allow(unreachable_code)]
                        { #handler_code }
                    }
                    ::core::option::Option::None => {
                        ::core::result::Result::Ok(#signal::Value(__results))
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
