//! Async try pattern: `async try { body } [handlers...]`
//!
//! Handles asynchronous try blocks with catch/throw/inspect/finally/with.
//!
//! Handlers are processed in declaration order, matching sync behavior.

use proc_macro2::{Span, TokenStream};
use quote::quote;
use syn::parse::{Parse, ParseStream};
use syn::{Result, Ident, braced};

use crate::keywords::{self, GenContext, peek_keyword, ChainVariant, Guard};
use crate::keywords::catch::CatchClause;
use crate::keywords::throw::ThrowClause;
use crate::keywords::inspect::InspectClause;
use crate::keywords::with_ctx::WithClause;
use crate::nested::{transform_nested, contains_question_mark};
use super::checks::{self, CheckAction};
use super::common::Handler;

/// Parsed async try input.
struct AsyncTryInput {
    body: TokenStream,
    /// All handlers in declaration order
    handlers: Vec<Handler>,
    finally: Option<TokenStream>,
    with_clause: Option<WithClause>,
}

impl Parse for AsyncTryInput {
    fn parse(input: ParseStream) -> Result<Self> {
        // Parse try body: { ... }
        let content;
        braced!(content in input);
        let body: TokenStream = content.parse()?;

        // Parse handlers in declaration order
        let mut handlers = Vec::new();
        let mut finally = None;
        let mut with_clause = None;

        while !input.is_empty() {
            if peek_keyword(input, "catch") {
                let clause = keywords::catch::parse(input)?;
                // Catch bodies must be infallible - reject `?` operator
                let has_question_mark = contains_question_mark(&clause.body)
                    || matches!(&clause.guard, Some(Guard::Match { arms, .. }) if contains_question_mark(arms));
                if has_question_mark {
                    return Err(syn::Error::new(
                        clause.catch_span,
                        "catch handlers must be infallible; use `try catch { ... }` to return Result",
                    ));
                }
                handlers.push(Handler::Catch(clause.clone()));

                // Check for `catch Type {} else {}` - creates catch-all after typed catch
                if clause.type_path.is_some() && input.peek(syn::Token![else]) {
                    input.parse::<syn::Token![else]>()?;
                    let else_body = keywords::parsing::parse_braced_body(input)?;
                    if contains_question_mark(&else_body) {
                        return Err(syn::Error::new(
                            clause.catch_span,
                            "else handlers must be infallible; use `try catch { ... }` to return Result",
                        ));
                    }
                    let else_clause = keywords::catch::CatchClause {
                        catch_span: clause.catch_span,
                        variant: keywords::ChainVariant::Root,
                        type_path: None,
                        binding: keywords::parsing::underscore_ident(),
                        guard: None,
                        body: else_body,
                    };
                    handlers.push(Handler::Catch(else_clause));
                }
            } else if peek_keyword(input, "throw") {
                let clause = keywords::throw::parse(input)?;
                handlers.push(Handler::Throw(clause.clone()));

                // Check for `throw Type {} else {}` - creates catch-all CATCH after typed throw
                if clause.type_path.is_some() && input.peek(syn::Token![else]) {
                    input.parse::<syn::Token![else]>()?;
                    let else_body = keywords::parsing::parse_braced_body(input)?;
                    // Else bodies must be infallible - reject `?` operator
                    if contains_question_mark(&else_body) {
                        return Err(syn::Error::new(
                            Span::call_site(),
                            "else handlers must be infallible; use `try catch { ... }` to return Result",
                        ));
                    }
                    let else_clause = keywords::catch::CatchClause {
                        catch_span: Span::call_site(),
                        variant: keywords::ChainVariant::Root,
                        type_path: None,
                        binding: keywords::parsing::underscore_ident(),
                        guard: None,
                        body: else_body,
                    };
                    handlers.push(Handler::Catch(else_clause));
                }
            } else if peek_keyword(input, "inspect") {
                let clause = keywords::inspect::parse(input)?;
                // Inspect bodies must be infallible - reject `?` operator
                let has_question_mark = contains_question_mark(&clause.body)
                    || matches!(&clause.guard, Some(Guard::Match { arms, .. }) if contains_question_mark(arms));
                if has_question_mark {
                    return Err(syn::Error::new(
                        clause.inspect_span,
                        "inspect handlers must be infallible; use `try catch { ... }` for fallible error handling",
                    ));
                }
                handlers.push(Handler::Inspect(clause));
            } else if peek_keyword(input, "finally") {
                let finally_span = input.span();
                if finally.is_some() {
                    return Err(syn::Error::new(
                        finally_span,
                        "multiple `finally` blocks are not allowed; combine into a single block",
                    ));
                }
                finally = Some(keywords::finally::parse(input)?);
            } else if peek_keyword(input, "with") {
                if with_clause.is_some() {
                    return Err(syn::Error::new(
                        input.span(),
                        "multiple `with` clauses are not allowed; combine context into a single `with { ... }`",
                    ));
                }
                with_clause = Some(keywords::with_ctx::parse(input)?);
            } else {
                let ident: Ident = input.parse()?;
                return Err(syn::Error::new(
                    ident.span(),
                    format!("unexpected keyword `{}`", ident),
                ));
            }
        }

        // async try { } alone is valid - just wraps error with stack frame

        // Validate handler order: untyped catch must be last
        validate_handler_order(&handlers)?;

        Ok(AsyncTryInput {
            body,
            handlers,
            finally,
            with_clause,
        })
    }
}

/// Validate that no handlers follow an untyped catch.
/// Untyped catch catches ALL errors, making subsequent handlers unreachable.
fn validate_handler_order(handlers: &[Handler]) -> Result<()> {
    let mut untyped_catch_span: Option<proc_macro2::Span> = None;

    for handler in handlers {
        // If we already saw an untyped catch, any subsequent handler is an error
        if let Some(span) = untyped_catch_span {
            let handler_name = handler.name();
            return Err(syn::Error::new(
                span,
                format!(
                    "untyped `catch` handles all errors; `{}` after it will never execute. \
                     If catch body can fail, use `try catch {{ ... }}` instead which returns Result",
                    handler_name
                ),
            ));
        }

        // Check if this handler is an untyped catch
        if let Handler::Catch(clause) = handler {
            if clause.type_path.is_none() {
                untyped_catch_span = Some(clause.catch_span);
            }
        }
    }

    Ok(())
}

/// Process async try pattern.
pub fn process(input: TokenStream) -> Result<TokenStream> {
    let parsed: AsyncTryInput = syn::parse2(input)?;
    Ok(generate(parsed))
}

/// Generate code for async try.
fn generate(input: AsyncTryInput) -> TokenStream {
    let mut ctx = GenContext::new().async_mode();
    if let Some(ref with) = input.with_clause {
        keywords::with_ctx::apply_to_context(with, &mut ctx);
    }

    let body = transform_nested(input.body.clone());
    let ctx_chain = keywords::with_ctx::gen_ctx_chain(&ctx);

    // Check if we have any handlers
    let has_handlers = !input.handlers.is_empty();

    let code = if has_handlers {
        // Generate handler checks in declaration order (like sync.rs)
        let handler_checks = generate_handler_checks(&input, &ctx_chain);

        // Always include fallback for proper type inference (may be dead code if catch-all exists)
        let fallback = quote! {
            ::core::result::Result::Err(__err)
        };

        // Use or_else closure like sync.rs - the checks use `return` which needs closure context
        // Always add #[allow(unreachable_code)] - we can't statically detect guarded catch-alls
        // like `catch _ when true` that are semantically catch-all
        quote! {
            let __result: ::core::result::Result<_, ::handle_this::Handled> =
                ::handle_this::__async_try_block!(#body)
                    .await
                    .or_else(|__raw_err| -> ::core::result::Result<_, ::handle_this::Handled> {
                        #[allow(unreachable_code)]
                        {
                            // __err must be mutable because throw can transform it
                            let mut __err: ::handle_this::Handled = ::handle_this::__wrap_frame(__raw_err, file!(), line!(), column!()) #ctx_chain;
                            #handler_checks
                            #fallback
                        }
                    });
            __result
        }
    } else {
        // No handlers - just wrap error with frame
        quote! {
            let __result: ::core::result::Result<_, ::handle_this::Handled> =
                ::handle_this::__async_try_block!(#body)
                    .await
                    .map_err(|__e| ::handle_this::__wrap_frame(__e, file!(), line!(), column!()) #ctx_chain);
            __result
        }
    };

    // Wrap with async finally if present
    let code = if let Some(ref finally_body) = input.finally {
        let finally_transformed = transform_nested(finally_body.clone());
        keywords::finally::wrap(code, &finally_transformed)
    } else {
        code
    };

    quote! { { #code } }
}

/// Generate handler checks in declaration order.
/// Uses flat check statements with early returns, matching sync behavior.
fn generate_handler_checks(input: &AsyncTryInput, ctx_chain: &TokenStream) -> TokenStream {
    let mut all_checks = Vec::new();

    // Generate checks in declaration order
    for handler in &input.handlers {
        match handler {
            Handler::Catch(clause) => {
                all_checks.push(generate_catch_check(clause));
            }
            Handler::Throw(clause) => {
                all_checks.push(generate_throw_check(clause, ctx_chain));
            }
            Handler::Inspect(clause) => {
                all_checks.push(generate_inspect_check(clause));
            }
            // TryCatch is not supported in async try - parsing rejects it
            Handler::TryCatch(_) => unreachable!("try catch not supported in async try"),
        }
    }

    quote! {
        #(#all_checks)*
    }
}

/// Generate a single catch check.
fn generate_catch_check(clause: &CatchClause) -> TokenStream {
    let binding = &clause.binding;
    let body = transform_nested(clause.body.clone());

    match (&clause.type_path, clause.variant) {
        // Catch-all
        (None, ChainVariant::Root) => {
            checks::gen_catchall_check(binding, &clause.guard, &body, CheckAction::ReturnOk, true)
        }
        // Typed catch
        (Some(type_path), variant) => {
            checks::gen_typed_check(variant, type_path, binding, &clause.guard, &body, CheckAction::ReturnOk)
        }
        // Invalid: catch-all with any/all variant
        (None, _) => {
            syn::Error::new(binding.span(), "catch any/all requires a type")
                .to_compile_error()
        }
    }
}

/// Generate a single throw check.
/// Throw transforms the error and continues the chain (does not return).
fn generate_throw_check(clause: &ThrowClause, _ctx_chain: &TokenStream) -> TokenStream {
    let throw_expr = transform_nested(clause.throw_expr.clone());
    let binding = clause.binding.as_ref();

    match (&clause.type_path, clause.variant) {
        // Untyped throw - transforms error
        (None, _) => {
            let binding_ident = binding.cloned().unwrap_or_else(|| {
                syn::Ident::new("_", proc_macro2::Span::call_site())
            });
            checks::gen_catchall_check(&binding_ident, &clause.guard, &throw_expr, CheckAction::Transform, false)
        }
        // Typed throw - transforms error if type matches
        (Some(type_path), variant) => {
            let binding_ident = binding.cloned().unwrap_or_else(|| {
                syn::Ident::new("_", proc_macro2::Span::call_site())
            });
            checks::gen_typed_check(variant, type_path, &binding_ident, &clause.guard, &throw_expr, CheckAction::Transform)
        }
    }
}

/// Generate a single inspect check.
fn generate_inspect_check(clause: &InspectClause) -> TokenStream {
    let binding = &clause.binding;
    let body = transform_nested(clause.body.clone());

    match (&clause.type_path, clause.variant) {
        // Untyped inspect
        (None, _) => {
            checks::gen_catchall_check(binding, &clause.guard, &body, CheckAction::Execute, false)
        }
        // Typed inspect
        (Some(type_path), variant) => {
            checks::gen_typed_check(variant, type_path, binding, &clause.guard, &body, CheckAction::Execute)
        }
    }
}
