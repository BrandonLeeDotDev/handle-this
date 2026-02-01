//! Sync try pattern: `try { body } [handlers...]`
//!
//! Handles synchronous try blocks with catch/throw/inspect/finally/with.
//!
//! Uses a hybrid approach for code generation:
//! - If no handler bodies contain control flow, uses `.or_else()` closure
//!   for proper error propagation between handlers
//! - If handlers contain control flow (continue/break), uses **signal mode**
//!   which transforms control flow to signals, allowing unmatched errors to propagate

use proc_macro2::TokenStream;
use quote::quote;
use syn::parse::{Parse, ParseStream};
use syn::{Result, Ident, braced};

use crate::keywords::{self, GenContext, peek_keyword, ChainVariant, Guard};
use crate::keywords::catch::CatchClause;
use crate::keywords::throw::ThrowClause;
use crate::keywords::inspect::InspectClause;
use crate::keywords::try_catch::TryCatchClause;
use crate::keywords::with_ctx::WithClause;
use crate::nested::{transform_nested, contains_control_flow, contains_question_mark};
use super::chain_builder;
use super::checks::{self, CheckAction};
use super::common::Handler;
use super::handlers::Handlers;
use super::signal::signal_type;
use super::signal_handler;

/// Parsed sync try input.
struct SyncTryInput {
    body: TokenStream,
    /// All handlers in declaration order
    handlers: Vec<Handler>,
    catches: Vec<CatchClause>,
    throws: Vec<ThrowClause>,
    inspects: Vec<InspectClause>,
    try_catches: Vec<TryCatchClause>,
    finally: Option<TokenStream>,
    with_clause: Option<WithClause>,
    /// Explicit return type for direct mode: `try -> T { ... }`
    /// When present, forces direct mode and provides type annotation.
    explicit_type: Option<syn::Type>,
}

impl Parse for SyncTryInput {
    fn parse(input: ParseStream) -> Result<Self> {
        // Check for explicit type: `-> T { ... }` (forces direct mode)
        let explicit_type = if input.peek(syn::Token![->]) {
            input.parse::<syn::Token![->]>()?;
            Some(input.parse::<syn::Type>()?)
        } else {
            None
        };

        // Parse try body: { ... }
        let content;
        braced!(content in input);
        let body: TokenStream = content.parse()?;

        // Check for empty body - use span of next token (first handler) for better error location
        if body.is_empty() {
            return Err(syn::Error::new(
                input.span(),
                "try body cannot be empty: `try { EXPR }`",
            ));
        }

        // Parse handlers - store both in typed vecs and ordered list
        let mut handlers = Vec::new();
        let mut catches = Vec::new();
        let mut throws = Vec::new();
        let mut inspects = Vec::new();
        let mut try_catches = Vec::new();
        let mut finally = None;
        let mut with_clause = None;

        while !input.is_empty() {
            // Check for `try catch` (result-returning catch)
            if input.peek(syn::Token![try]) {
                input.parse::<syn::Token![try]>()?;
                let clause = keywords::try_catch::parse(input)?;
                handlers.push(Handler::TryCatch(clause.clone()));
                try_catches.push(clause);
            } else if peek_keyword(input, "catch") {
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
                catches.push(clause.clone());

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
                    handlers.push(Handler::Catch(else_clause.clone()));
                    catches.push(else_clause);
                }
            } else if peek_keyword(input, "throw") {
                let clause = keywords::throw::parse(input)?;
                handlers.push(Handler::Throw(clause.clone()));
                throws.push(clause.clone());

                // Check for `throw Type {} else {}` - creates catch-all CATCH after typed throw
                if clause.type_path.is_some() && input.peek(syn::Token![else]) {
                    input.parse::<syn::Token![else]>()?;
                    let else_body = keywords::parsing::parse_braced_body(input)?;
                    // Else bodies must be infallible - reject `?` operator
                    if contains_question_mark(&else_body) {
                        return Err(syn::Error::new(
                            proc_macro2::Span::call_site(),
                            "else handlers must be infallible; use `try catch { ... }` to return Result",
                        ));
                    }
                    let else_clause = keywords::catch::CatchClause {
                        catch_span: proc_macro2::Span::call_site(),
                        variant: keywords::ChainVariant::Root,
                        type_path: None,
                        binding: keywords::parsing::underscore_ident(),
                        guard: None,
                        body: else_body,
                    };
                    handlers.push(Handler::Catch(else_clause.clone()));
                    catches.push(else_clause);
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
                handlers.push(Handler::Inspect(clause.clone()));
                inspects.push(clause);
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
            } else if input.peek(syn::Token![else]) {
                // `else { }` is syntactic sugar for catch-all in direct mode (try -> T)
                let else_token = input.parse::<syn::Token![else]>()?;
                if explicit_type.is_none() {
                    return Err(syn::Error::new(
                        else_token.span,
                        "`else` requires either direct mode (`try -> T { } else { }`) or a typed handler (`catch Type { } else { }`)",
                    ));
                }
                let else_body = keywords::parsing::parse_braced_body(input)?;
                // Else bodies must be infallible - reject `?` operator
                if contains_question_mark(&else_body) {
                    return Err(syn::Error::new(
                        else_token.span,
                        "else handlers must be infallible; use `try catch { ... }` to return Result",
                    ));
                }
                // Create a catch-all clause
                let clause = keywords::catch::CatchClause {
                    catch_span: else_token.span,
                    variant: ChainVariant::Root,
                    type_path: None,
                    binding: keywords::parsing::underscore_ident(),
                    guard: None,
                    body: else_body,
                };
                handlers.push(Handler::Catch(clause.clone()));
                catches.push(clause);
            } else {
                let ident: Ident = input.parse()?;
                let msg = if ident == "scope" {
                    "`scope` must appear before `try`, not after handlers. Use: `scope \"name\", try { } catch { }`".to_string()
                } else {
                    format!("unexpected keyword `{}`, expected catch/throw/inspect/finally/with/else", ident)
                };
                return Err(syn::Error::new(ident.span(), msg));
            }
        }

        // Validate handler order: untyped catch/try_catch must be last
        // (handlers after them are unreachable)
        validate_handler_order(&handlers)?;

        Ok(SyncTryInput {
            body,
            handlers,
            catches,
            throws,
            inspects,
            try_catches,
            finally,
            with_clause,
            explicit_type,
        })
    }
}

/// Validate that no handlers follow an untyped catch or try catch.
/// Untyped catch/try_catch catches ALL errors, making subsequent handlers unreachable.
fn validate_handler_order(handlers: &[Handler]) -> Result<()> {
    let mut untyped_catch_span: Option<proc_macro2::Span> = None;
    let mut untyped_catch_is_try: bool = false;

    for handler in handlers {
        // If we already saw an untyped catch, any subsequent handler is an error
        if let Some(span) = untyped_catch_span {
            let handler_name = match handler {
                Handler::Catch(_) => "catch",
                Handler::Throw(_) => "throw",
                Handler::Inspect(_) => "inspect",
                Handler::TryCatch(_) => "try catch",
            };
            let suggestion = if untyped_catch_is_try {
                format!(
                    "untyped `try catch` handles all errors; `{}` after it will never execute",
                    handler_name
                )
            } else {
                format!(
                    "untyped `catch` handles all errors; `{}` after it will never execute. \
                     If catch body can fail, use `try catch {{ ... }}` instead which returns Result",
                    handler_name
                )
            };
            return Err(syn::Error::new(span, suggestion));
        }

        // Check if this handler is an untyped catch or try_catch
        match handler {
            Handler::Catch(clause) if clause.type_path.is_none() => {
                untyped_catch_span = Some(clause.catch_span);
                untyped_catch_is_try = false;
            }
            Handler::TryCatch(clause) if clause.type_path.is_none() => {
                untyped_catch_span = Some(clause.binding.span());
                untyped_catch_is_try = true;
            }
            _ => {}
        }
    }

    Ok(())
}

/// Process sync try pattern.
pub fn process(input: TokenStream) -> Result<TokenStream> {
    let parsed: SyncTryInput = syn::parse2(input)?;
    Ok(generate(parsed))
}

/// Check if any handler body or the try body contains control flow (continue/break).
/// Only use match-based approach when control flow is present, as it has
/// type inference issues with deeply nested structures.
///
/// This checks:
/// 1. The try body - for nested try blocks with control flow in their handlers
/// 2. All handler bodies - for direct control flow in catch/throw/inspect
///
/// When nested try blocks use signal mode (control flow in handlers), they generate
/// code that contains break/continue at the expression level. The outer try must
/// also use signal mode to avoid wrapping that in a closure.
fn handlers_have_control_flow(input: &SyncTryInput) -> bool {
    // Check if body contains control flow (from nested try handlers using signal mode)
    // The body may contain transformed proc macro calls like:
    //   ::handle_this_macros::__sync_try_proc!({ ... } throw _ { break })
    // The break/continue is inside the macro arguments and must propagate up
    if contains_control_flow(&input.body) {
        return true;
    }

    // Check handlers for direct control flow
    for handler in &input.handlers {
        let (body, guard) = match handler {
            Handler::Catch(c) => (&c.body, &c.guard),
            Handler::Throw(t) => (&t.throw_expr, &t.guard),
            Handler::Inspect(i) => (&i.body, &i.guard),
            Handler::TryCatch(tc) => (&tc.body, &tc.guard),
        };
        if contains_control_flow(body) {
            return true;
        }
        if let Some(Guard::Match { arms, .. }) = guard {
            if contains_control_flow(arms) {
                return true;
            }
        }
    }
    false
}

/// Generate code for sync try.
fn generate(input: SyncTryInput) -> TokenStream {
    // Build context
    let mut ctx = GenContext::new();
    if let Some(ref with) = input.with_clause {
        keywords::with_ctx::apply_to_context(with, &mut ctx);
    }

    // Transform nested patterns in the try body
    let body = transform_nested(input.body.clone());
    let ctx_chain = keywords::with_ctx::gen_ctx_chain(&ctx);

    // Check if we have any handlers
    let has_handlers = !input.catches.is_empty()
        || !input.throws.is_empty()
        || !input.inspects.is_empty()
        || !input.try_catches.is_empty();

    let code = if has_handlers {
        // Check if handlers contain control flow (continue/break)
        let has_control_flow = handlers_have_control_flow(&input);

        // Check if there's a catch-all handler (typed handlers may not match all errors)
        let has_catch_all = has_unconditional_catchall(&input);

        // Explicit type annotation forces direct mode: `try -> T { ... }`
        let force_direct = input.explicit_type.is_some();

        if force_direct {
            // Use DIRECT MODE - explicit type annotation (try -> T) forces direct mode
            // Error propagation uses ? for typed-only catches, unreachable for catch-all
            generate_direct_mode(&input, &body, has_catch_all, &ctx_chain)
        } else if has_control_flow {
            // Use SIGNAL MODE - transforms break/continue to signals, allows error propagation
            // Works in fallible mode (returns Result) unlike direct mode
            generate_signal_mode(&input, &body, has_catch_all, &ctx_chain)
        } else if can_skip_frame_wrapping(&input) {
            // OPTIMIZATION: Simple catch-all with `_` binding - skip frame wrapping entirely
            // The error is never accessed, so no stack frame info is needed
            let catch_body = transform_nested(input.catches[0].body.clone());
            quote! {
                ::handle_this::__try_block!(#body).or_else(|_| -> ::core::result::Result<_, ::handle_this::Handled> {
                    #[allow(unreachable_code)]
                    ::core::result::Result::Ok({ #catch_body })
                })
            }
        } else if let Some((early_exits, catchall)) = can_use_early_type_check(&input) {
            // OPTIMIZATION: Typed catches with `_` binding - check type directly on Box<dyn Error>
            // No Handled wrapping needed since bindings are unused.
            // Must check both raw error AND unwrapped Handled (for nested try blocks).
            let early_checks: Vec<TokenStream> = early_exits.iter().map(|catch| {
                let type_path = catch.type_path.as_ref().unwrap();
                let catch_body = transform_nested(catch.body.clone());
                quote! {
                    if __raw_err.downcast_ref::<#type_path>().is_some()
                       || __raw_err.downcast_ref::<::handle_this::Handled>()
                           .map_or(false, |__h| __h.downcast_ref::<#type_path>().is_some())
                    {
                        return ::core::result::Result::Ok({ #catch_body });
                    }
                }
            }).collect();
            let catchall_body = transform_nested(catchall.body.clone());
            quote! {
                ::handle_this::__try_block!(#body).or_else(|__raw_err| -> ::core::result::Result<_, ::handle_this::Handled> {
                    #(#early_checks)*
                    #[allow(unreachable_code)]
                    ::core::result::Result::Ok({ #catchall_body })
                })
            }
        } else if is_simple_catchall_only(&input) {
            // OPTIMIZATION: Simple catch-all with used binding - inline directly without handler chain
            // Frame wrapping needed since binding is used, but no handler iteration overhead
            let catch = &input.catches[0];
            let catch_body = transform_nested(catch.body.clone());
            let binding = &catch.binding;
            quote! {
                ::handle_this::__try_block!(#body).or_else(|__raw_err| -> ::core::result::Result<_, ::handle_this::Handled> {
                    let #binding = ::handle_this::__wrap_frame(__raw_err, file!(), line!(), column!()) #ctx_chain;
                    #[allow(unreachable_code)]
                    ::core::result::Result::Ok({ #catch_body })
                })
            }
        } else {
            // Use or_else closure for better type inference with deeply nested structures
            // Note: This means continue/break won't work in catch bodies
            // generate_handler_checks_closure handles lazy wrapping internally
            let handler_checks = generate_handler_checks_closure(&input, &ctx, &ctx_chain);

            // Always add #[allow(unreachable_code)] - we can't statically detect guarded
            // catch-alls like `catch _ when true` that are semantically catch-all
            quote! {
                ::handle_this::__try_block!(#body).or_else(|__raw_err| -> ::core::result::Result<_, ::handle_this::Handled> {
                    #[allow(unreachable_code)]
                    {
                        #handler_checks
                    }
                })
            }
        }
    } else {
        // No handlers
        if contains_control_flow(&body) {
            // Body contains control flow (break/continue from nested try handler)
            // Can't use closure-based __try_block as it would block control flow
            // Must transform nested try blocks in the body first
            let transformed_body = transform_nested(body.clone());
            quote! { { #transformed_body } }
        } else {
            // Standard case - wrap error with frame
            quote! {
                ::handle_this::__try_block!(#body)
                    .map_err(|__e| ::handle_this::__wrap_frame(__e, file!(), line!(), column!()) #ctx_chain)
            }
        }
    };

    // Wrap with finally if present
    let code = if let Some(ref finally_body) = input.finally {
        let finally_transformed = transform_nested(finally_body.clone());
        keywords::finally::wrap(code, &finally_transformed)
    } else {
        code
    };

    quote! { { #code } }
}

/// Find the span of the first handler that contains control flow.
/// Used for error reporting - returns the span of the handler keyword (e.g., `catch`).
fn find_control_flow_handler_span(input: &SyncTryInput) -> proc_macro2::Span {
    for catch in &input.catches {
        if contains_control_flow(&catch.body) {
            return catch.catch_span;
        }
        if let Some(Guard::Match { arms, .. }) = &catch.guard {
            if contains_control_flow(arms) {
                return catch.catch_span;
            }
        }
    }
    for throw in &input.throws {
        if contains_control_flow(&throw.throw_expr) {
            return throw.binding.as_ref()
                .map(|b| b.span())
                .unwrap_or_else(proc_macro2::Span::call_site);
        }
    }
    for inspect in &input.inspects {
        if contains_control_flow(&inspect.body) {
            return inspect.binding.span();
        }
    }
    for try_catch in &input.try_catches {
        if contains_control_flow(&try_catch.body) {
            return try_catch.binding.span();
        }
    }
    proc_macro2::Span::call_site()
}

/// Check if we have a simple catch-all only pattern.
/// True when there's exactly one unguarded catch-all (regardless of binding).
/// This allows us to use the fast path for direct inlining.
fn is_simple_catchall_only(input: &SyncTryInput) -> bool {
    // Must have exactly one catch, nothing else
    if input.catches.len() != 1
        || !input.throws.is_empty()
        || !input.inspects.is_empty()
        || !input.try_catches.is_empty()
    {
        return false;
    }

    let catch = &input.catches[0];
    // Must be untyped catch-all with no guard
    catch.type_path.is_none() && catch.guard.is_none()
}

/// Check if we can skip `__wrap_frame` for this input.
/// True when the ONLY handler is an unconditional catch-all with `_` binding.
/// In this case, the error is never accessed, so no frame info is needed.
fn can_skip_frame_wrapping(input: &SyncTryInput) -> bool {
    if !is_simple_catchall_only(input) {
        return false;
    }
    // Additional check: binding must be `_`
    input.catches[0].binding.to_string() == "_"
}

/// Check if a catch can be checked early on raw Box<dyn Error>.
/// Returns true for Root variant typed catches (any binding, any guard).
/// These can use downcast_ref directly on Box<dyn Error> without Handled wrapper.
fn can_check_early(catch: &CatchClause) -> bool {
    catch.type_path.is_some()  // Must be typed (not catch-all)
        && matches!(catch.variant, ChainVariant::Root)  // Only root variant (downcast_ref)
}

/// Check if a catch has unused binding (is `_`) and can be checked early.
/// Returns true for typed catches with `_` binding, no guard, and Root variant.
fn is_early_exit_catch(catch: &CatchClause) -> bool {
    can_check_early(catch)
        && catch.binding.to_string() == "_"  // Binding unused
        && catch.guard.is_none()  // No guard
}

/// Check if we can use the early type check optimization.
/// This applies when:
/// 1. We have typed catches with `_` binding (early exit candidates)
/// 2. We have an unconditional catch-all with `_` binding as fallback
/// 3. No other handlers that would need the wrapped error
fn can_use_early_type_check(input: &SyncTryInput) -> Option<(Vec<&CatchClause>, &CatchClause)> {
    // Must have no throws, inspects, or try_catches (they need the wrapped error)
    if !input.throws.is_empty() || !input.inspects.is_empty() || !input.try_catches.is_empty() {
        return None;
    }

    // Find early exit catches and the catch-all
    let mut early_exits: Vec<&CatchClause> = Vec::new();
    let mut catchall: Option<&CatchClause> = None;

    for catch in &input.catches {
        if is_early_exit_catch(catch) {
            early_exits.push(catch);
        } else if catch.type_path.is_none() && catch.guard.is_none() {
            // Catch-all - must also have `_` binding for this optimization
            if catch.binding.to_string() == "_" {
                catchall = Some(catch);
            } else {
                // Catch-all needs binding, can't use optimization
                return None;
            }
        } else {
            // Some other handler that needs wrapping
            return None;
        }
    }

    // Must have at least one early exit and a catch-all
    if early_exits.is_empty() || catchall.is_none() {
        return None;
    }

    Some((early_exits, catchall.unwrap()))
}

/// Check if any handler is an unconditional catch-all (no type, no guard).
/// Such handlers always return, making any subsequent code unreachable.
fn has_unconditional_catchall(input: &SyncTryInput) -> bool {
    // Check catches: catch-all without guard always returns
    for catch in &input.catches {
        if catch.type_path.is_none() && catch.guard.is_none() {
            return true;
        }
    }
    // Check throws: untyped throw without guard always returns
    for throw in &input.throws {
        if throw.type_path.is_none() && throw.guard.is_none() {
            return true;
        }
    }
    // Check try_catches: catch-all without guard always returns
    for tc in &input.try_catches {
        if tc.type_path.is_none() && tc.guard.is_none() {
            return true;
        }
    }
    // Note: inspect doesn't return, so it doesn't make fallback unreachable
    false
}

// ============================================================
// Signal Mode Generation (for control flow in fallible mode)
// ============================================================

/// Generate signal mode code for handlers with control flow in fallible mode.
///
/// Signal mode transforms `break`/`continue` to signal values, allowing:
/// 1. Control flow to escape the closure boundary
/// 2. Unmatched typed errors to propagate (unlike direct mode's `unreachable!()`)
///
/// Returns `Result<T, Handled>` like normal fallible mode, but internally uses
/// `Result<__LoopSignal<T>, Handled>` to encode control flow.
fn generate_signal_mode(
    input: &SyncTryInput,
    body: &TokenStream,
    has_catch_all: bool,
    ctx_chain: &TokenStream,
) -> TokenStream {
    let signal = signal_type();

    // Check if body contains control flow from nested try patterns.
    // If so, the body is a proc macro call that:
    // 1. Returns Result<T, Handled> directly (not T)
    // 2. Contains break/continue that must escape ALL closures
    let body_has_control_flow = contains_control_flow(body);
    let body_has_question_mark = contains_question_mark(body);

    // Check if outer handlers have control flow
    let handlers_have_cf = input.handlers.iter().any(|h| {
        let (hbody, guard) = match h {
            Handler::Catch(c) => (&c.body, &c.guard),
            Handler::Throw(t) => (&t.throw_expr, &t.guard),
            Handler::Inspect(i) => (&i.body, &i.guard),
            Handler::TryCatch(tc) => (&tc.body, &tc.guard),
        };
        contains_control_flow(hbody) || matches!(guard, Some(Guard::Match { arms, .. }) if contains_control_flow(arms))
    });

    // When body has control flow from nested try, we CANNOT use any closure.
    // The nested try expands to code with break/continue that must escape.
    // Use inline expansion instead.
    if body_has_control_flow && !body_has_question_mark && !handlers_have_cf {
        // INLINE MODE: No closures at all. Body's break/continue escape directly.
        // Outer handlers evaluate inline and produce Result values directly.
        //
        // Since we're not in a closure, we can't use `return`. Instead, we generate
        // a nested if-else chain where each branch produces the final value.
        //
        // Use __force_result_type to help type inference when body is a control signal mode block.
        // Specify Handled as the error type since break/continue arms don't produce a Result.
        let handler_code = generate_inline_handler_chain(input, has_catch_all, ctx_chain);

        return quote! {
            {
                #[allow(unreachable_code)]
                match ::handle_this::__force_result_type::<_, ::handle_this::Handled>({ #body }) {
                    ::core::result::Result::Ok(__v) => ::core::result::Result::Ok(__v),
                    ::core::result::Result::Err(__handled_err) => {
                        #[allow(unused_mut, unused_variables)]
                        let mut __err = __handled_err #ctx_chain;
                        #handler_code
                    }
                }
            }
        };
    }

    // CLOSURE MODE: Use closure-based signal handling.
    // Safe when body doesn't have nested control flow, or when it uses __try_block!.
    let handlers = build_handlers_from_input(input);
    let handler_code = signal_handler::gen_signal_handler(&handlers, ctx_chain);

    // For catch-all handlers, errors are always handled, so Err arm is unreachable.
    // For typed-only handlers, errors may not match, so we propagate them.
    let err_arm = if has_catch_all {
        quote! {
            ::core::result::Result::Err(_) => {
                ::core::unreachable!("catch-all handler should have handled all errors")
            }
        }
    } else {
        quote! {
            ::core::result::Result::Err(__e) => {
                ::core::result::Result::Err(__e)
            }
        }
    };

    // Check if all handlers contain control flow (break/continue)
    // When handlers are pure control flow AND body uses `?` (so we need closure),
    // use __ControlSignal to avoid type inference issues.
    // But NOT for nested try blocks (body has control flow but no `?`) - those use inline mode.
    let all_handlers_have_cf = handlers.handlers.iter().all(|h| {
        match h {
            Handler::Catch(c) => contains_control_flow(&c.body),
            Handler::Throw(t) => contains_control_flow(&t.throw_expr),
            Handler::Inspect(i) => contains_control_flow(&i.body),
            _ => false,
        }
    });

    // Only use control signal mode when body needs a closure (has `?`) AND all handlers are pure CF.
    // Skip control signal mode when body has control flow (nested try) - that uses inline mode above.
    if all_handlers_have_cf && !handlers.handlers.is_empty() && !body_has_control_flow {
        // Use non-generic __ControlSignal - value is stored outside closure
        let ctrl_handler_code = signal_handler::gen_control_signal_handler(&handlers, ctx_chain);
        let ctrl_err_arm = if has_catch_all {
            quote! {
                ::core::result::Result::Err(_) => {
                    ::core::unreachable!("catch-all handler should have handled all errors")
                }
            }
        } else {
            quote! {
                ::core::result::Result::Err(__e) => {
                    ::core::result::Result::Err(__e)
                }
            }
        };

        // Evaluate body first so we can use its type to infer the Option's type.
        // Use __ctrl_none_like to create Option<T> where T is inferred from the body Result.
        return quote! {
            {
                #[allow(unreachable_code)]
                {
                    let __body_result = ::handle_this::__try_block!(#body);
                    let mut __signal_value = ::handle_this::__ctrl_none_like(&__body_result);
                    let __result = (|| -> ::handle_this::__ControlResult {
                        match __body_result {
                            ::core::result::Result::Ok(__v) => {
                                ::handle_this::__ctrl_store_value(&mut __signal_value, __v)
                            }
                            ::core::result::Result::Err(__raw_err) => {
                                #[allow(unused_mut, unused_variables)]
                                let mut __err = ::handle_this::__wrap_frame(__raw_err, file!(), line!(), column!()) #ctx_chain;
                                #ctrl_handler_code
                            }
                        }
                    })();
                    match __result {
                        ::core::result::Result::Ok(::handle_this::__ControlSignal::Value) => {
                            // Option is always Some when Value is returned
                            ::core::result::Result::Ok(__signal_value.unwrap())
                        }
                        ::core::result::Result::Ok(::handle_this::__ControlSignal::Continue) => continue,
                        ::core::result::Result::Ok(::handle_this::__ControlSignal::Break) => break,
                        #ctrl_err_arm
                    }
                }
            }
        };
    }

    // HYBRID MODE: Body has control flow from nested try, handlers also have control flow.
    // We can't use closures around the body (would trap nested control flow),
    // but we need signals for handler control flow.
    //
    // Solution: Evaluate body inline (control flow escapes), then use a closure
    // just for the handlers so they can use `return` for signal-based control flow.
    if body_has_control_flow && !body_has_question_mark && handlers_have_cf {
        let signal = signal_type();
        let handler_code = signal_handler::gen_signal_handler(&handlers, ctx_chain);

        let err_arm = if has_catch_all {
            quote! {
                ::core::result::Result::Err(_) => {
                    ::core::unreachable!("catch-all handler should have handled all errors")
                }
            }
        } else {
            quote! {
                ::core::result::Result::Err(__e) => {
                    ::core::result::Result::Err(__e)
                }
            }
        };

        return quote! {
            {
                #[allow(unreachable_code)]
                match ::handle_this::__force_result_type::<_, ::handle_this::Handled>({
                    // Evaluate body inline - control flow escapes here
                    match { #body } {
                        ::core::result::Result::Ok(__v) => {
                            ::core::result::Result::Ok(#signal::Value(__v))
                        }
                        ::core::result::Result::Err(__raw_err) => {
                            // Wrap handlers in closure so they can use `return`
                            (|| {
                                #[allow(unused_mut, unused_variables)]
                                let mut __err = __raw_err #ctx_chain;
                                #handler_code
                            })()
                        }
                    }
                }) {
                    ::core::result::Result::Ok(#signal::Value(__v)) => ::core::result::Result::Ok(__v),
                    ::core::result::Result::Ok(#signal::Continue) => continue,
                    ::core::result::Result::Ok(#signal::Break) => break,
                    #err_arm
                }
            }
        };
    }

    // Standard signal mode with LoopSignal<T>
    quote! {
        {
            #[allow(unreachable_code)]
            match (|| {
                match ::handle_this::__try_block!(#body) {
                    ::core::result::Result::Ok(__v) => {
                        ::core::result::Result::Ok(#signal::Value(__v))
                    }
                    ::core::result::Result::Err(__raw_err) => {
                        // __err must be mutable because throw can transform it
                        #[allow(unused_mut, unused_variables)]
                        let mut __err = ::handle_this::__wrap_frame(__raw_err, file!(), line!(), column!()) #ctx_chain;
                        // Handler returns Ok(LoopSignal::*) or Err(e) for unmatched
                        #handler_code
                    }
                }
            })() {
                ::core::result::Result::Ok(#signal::Value(__v)) => ::core::result::Result::Ok(__v),
                ::core::result::Result::Ok(#signal::Continue) => continue,
                ::core::result::Result::Ok(#signal::Break) => break,
                #err_arm
            }
        }
    }
}

/// Build a Handlers struct from SyncTryInput for use with signal_handler.
fn build_handlers_from_input(input: &SyncTryInput) -> Handlers {
    Handlers {
        handlers: input.handlers.clone(),
        catches: input.catches.clone(),
        throws: input.throws.clone(),
        inspects: input.inspects.clone(),
        finally: input.finally.clone(),
        with_clause: input.with_clause.clone(),
    }
}

// ============================================================
// Direct Mode Generation (for explicit type annotation)
// ============================================================

/// Generate direct mode code for handlers with control flow or explicit type.
///
/// Direct mode expands without a closure, allowing labeled break/continue to work.
/// When triggered by control flow, requires an unconditional catch-all handler.
/// When triggered by explicit type (`try -> T`), requires catch-all for clear errors.
fn generate_direct_mode(
    input: &SyncTryInput,
    body: &TokenStream,
    has_catch_all: bool,
    ctx_chain: &TokenStream,
) -> TokenStream {
    let has_control_flow = handlers_have_control_flow(input);

    // Direct mode requires a catch-all handler (whether from control flow or explicit type).
    // Without one, unmatched errors have nowhere to go.
    if !has_catch_all {
        let (span, msg) = if has_control_flow {
            (
                find_control_flow_handler_span(input),
                "catch with break/continue requires a catch-all fallback: add `catch { ... }`",
            )
        } else {
            // Explicit direct mode (try -> T) - point to the first catch
            let span = input.catches.first()
                .map(|c| c.catch_span)
                .unwrap_or_else(proc_macro2::Span::call_site);
            (span, "direct mode (try -> T) requires a catch-all fallback: add `catch { ... }`")
        };
        return syn::Error::new(span, msg).to_compile_error();
    }

    // In direct mode, throw must be followed by a catch/else handler.
    // Without a subsequent handler, the transformed error has nowhere to go.
    if !input.throws.is_empty() && input.catches.is_empty() {
        let throw_span = proc_macro2::Span::call_site(); // TODO: get actual throw span
        return syn::Error::new(
            throw_span,
            "direct mode (try -> T) with throw requires a catch handler: add `catch { ... }` after throw",
        ).to_compile_error();
    }

    // OPTIMIZATION: Fast paths for simple catch-all patterns
    let skip_frame = can_skip_frame_wrapping(input);
    let simple_catchall = is_simple_catchall_only(input);
    let early_type_check = can_use_early_type_check(input);

    // Generate the match expression
    let match_expr = if skip_frame {
        // Simple catch-all with `_` binding - no frame wrapping needed
        let catch_body = transform_nested(input.catches[0].body.clone());
        quote! {
            match ::handle_this::__try_block!(#body) {
                ::core::result::Result::Ok(__v) => __v,
                ::core::result::Result::Err(_) => { #catch_body }
            }
        }
    } else if let Some((early_exits, catchall)) = early_type_check {
        // OPTIMIZATION: Typed catches with `_` binding - check type directly on Box<dyn Error>
        // No Handled wrapping needed since bindings are unused.
        // Must check both raw error AND unwrapped Handled (for nested try blocks).
        let early_checks: Vec<TokenStream> = early_exits.iter().map(|catch| {
            let type_path = catch.type_path.as_ref().unwrap();
            let catch_body = transform_nested(catch.body.clone());
            quote! {
                if __raw_err.downcast_ref::<#type_path>().is_some()
                   || __raw_err.downcast_ref::<::handle_this::Handled>()
                       .map_or(false, |__h| __h.downcast_ref::<#type_path>().is_some())
                {
                    #catch_body
                } else
            }
        }).collect();
        let catchall_body = transform_nested(catchall.body.clone());
        quote! {
            match ::handle_this::__try_block!(#body) {
                ::core::result::Result::Ok(__v) => __v,
                ::core::result::Result::Err(__raw_err) => {
                    #(#early_checks)* { #catchall_body }
                }
            }
        }
    } else if simple_catchall {
        // Simple catch-all with used binding - inline directly without handler chain
        let catch = &input.catches[0];
        let catch_body = transform_nested(catch.body.clone());
        let binding = &catch.binding;
        quote! {
            match ::handle_this::__try_block!(#body) {
                ::core::result::Result::Ok(__v) => __v,
                ::core::result::Result::Err(__raw_err) => {
                    let #binding = ::handle_this::__wrap_frame(__raw_err, file!(), line!(), column!()) #ctx_chain;
                    #catch_body
                }
            }
        }
    } else {
        // Full handler chain machinery
        let fallback = quote! {
            ::core::unreachable!("catch-all handler should have handled all errors")
        };
        let chain = generate_handler_chain_direct_with_fallback(input, fallback);
        quote! {
            {
                #[allow(unreachable_code)]
                match ::handle_this::__try_block!(#body) {
                    ::core::result::Result::Ok(__v) => __v,
                    ::core::result::Result::Err(__raw_err) => {
                        // __err must be mutable because throw can transform it
                        #[allow(unused_mut, unused_variables)]
                        let mut __err = ::handle_this::__wrap_frame(__raw_err, file!(), line!(), column!()) #ctx_chain;
                        #chain
                    }
                }
            }
        }
    };

    // If explicit type provided, wrap with type annotation
    // Note: Don't add extra block here - generate() adds the outer block
    if let Some(ref ty) = input.explicit_type {
        quote! {
            let __typed_result: #ty = #match_expr;
            __typed_result
        }
    } else {
        match_expr
    }
}

/// Generate handler chain with a configurable fallback.
/// Handlers are processed in declaration order.
fn generate_handler_chain_direct_with_fallback(input: &SyncTryInput, fallback: TokenStream) -> TokenStream {
    let mut handlers = Vec::new();

    // Build handlers in declaration order
    for handler in &input.handlers {
        match handler {
            Handler::Catch(catch) => {
                handlers.push(HandlerSpec::Catch {
                    type_path: catch.type_path.clone(),
                    variant: catch.variant,
                    binding: catch.binding.clone(),
                    guard: catch.guard.clone(),
                    body: transform_nested(catch.body.clone()),
                });
            }
            Handler::Throw(throw) => {
                handlers.push(HandlerSpec::Throw {
                    type_path: throw.type_path.clone(),
                    variant: throw.variant,
                    binding: throw.binding.clone(),
                    guard: throw.guard.clone(),
                    throw_expr: transform_nested(throw.throw_expr.clone()),
                });
            }
            Handler::Inspect(inspect) => {
                handlers.push(HandlerSpec::Inspect {
                    type_path: inspect.type_path.clone(),
                    variant: inspect.variant,
                    binding: inspect.binding.clone(),
                    guard: inspect.guard.clone(),
                    body: transform_nested(inspect.body.clone()),
                });
            }
            Handler::TryCatch(try_catch) => {
                handlers.push(HandlerSpec::TryCatch {
                    type_path: try_catch.type_path.clone(),
                    variant: try_catch.variant,
                    binding: try_catch.binding.clone(),
                    guard: try_catch.guard.clone(),
                    body: transform_nested(try_catch.body.clone()),
                });
            }
        }
    }

    build_nested_chain_direct(&handlers, fallback)
}

/// Generate handler checks for inside the or_else closure.
/// Uses `return` statements since we're in a closure context.
///
/// Note: We cannot do "early" type checks on raw Box<dyn Error> because
/// nested try blocks produce errors already wrapped in Handled. The wrap_box
/// function handles this by extracting existing Handled wrappers, so we must
/// wrap first, then check types.
fn generate_handler_checks_closure(input: &SyncTryInput, _ctx: &GenContext, ctx_chain: &TokenStream) -> TokenStream {
    let mut all_checks = Vec::new();

    // Generate checks in declaration order
    for handler in &input.handlers {
        match handler {
            Handler::Catch(clause) => {
                all_checks.push(generate_single_catch_check(clause));
            }
            Handler::Throw(clause) => {
                all_checks.push(generate_single_throw_check(clause));
            }
            Handler::Inspect(clause) => {
                all_checks.push(generate_single_inspect_check(clause));
            }
            Handler::TryCatch(clause) => {
                all_checks.push(generate_single_try_catch_check(clause));
            }
        }
    }

    // Always include fallback for proper type-checking, even when there's a catch-all.
    // The catch-all's return makes it unreachable, but the compiler still needs it.
    let fallback = quote! {
        ::core::result::Result::Err(__err)
    };

    // Always wrap at the beginning - wrap_box handles nested Handled efficiently
    // __err must be mutable because throw can transform it
    quote! {
        let mut __err = ::handle_this::__wrap_frame(__raw_err, file!(), line!(), column!()) #ctx_chain;
        #(#all_checks)*
        #fallback
    }
}

/// Generate a single catch check.
fn generate_single_catch_check(clause: &CatchClause) -> TokenStream {
    let binding = &clause.binding;
    let body = &clause.body;

    match (&clause.type_path, clause.variant) {
        // Catch-all
        (None, ChainVariant::Root) => {
            checks::gen_catchall_check(binding, &clause.guard, body, CheckAction::ReturnOk, true)
        }
        // Typed catch
        (Some(type_path), variant) => {
            checks::gen_typed_check(variant, type_path, binding, &clause.guard, body, CheckAction::ReturnOk)
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
fn generate_single_throw_check(clause: &ThrowClause) -> TokenStream {
    let throw_expr = &clause.throw_expr;
    let binding = clause.binding.as_ref();

    match (&clause.type_path, clause.variant) {
        // Untyped throw - transforms error
        (None, _) => {
            let binding_ident = binding.cloned().unwrap_or_else(|| {
                syn::Ident::new("_", proc_macro2::Span::call_site())
            });
            checks::gen_catchall_check(&binding_ident, &clause.guard, throw_expr, CheckAction::Transform, false)
        }
        // Typed throw - transforms error if type matches
        (Some(type_path), variant) => {
            let binding_ident = binding.cloned().unwrap_or_else(|| {
                syn::Ident::new("_", proc_macro2::Span::call_site())
            });
            checks::gen_typed_check(variant, type_path, &binding_ident, &clause.guard, throw_expr, CheckAction::Transform)
        }
    }
}

/// Generate a single inspect check.
fn generate_single_inspect_check(clause: &InspectClause) -> TokenStream {
    let binding = &clause.binding;
    let body = &clause.body;

    match (&clause.type_path, clause.variant) {
        // Untyped inspect
        (None, _) => {
            checks::gen_catchall_check(binding, &clause.guard, body, CheckAction::Execute, false)
        }
        // Typed inspect
        (Some(type_path), variant) => {
            checks::gen_typed_check(variant, type_path, binding, &clause.guard, body, CheckAction::Execute)
        }
    }
}

/// Generate a single try catch check.
fn generate_single_try_catch_check(clause: &TryCatchClause) -> TokenStream {
    let binding = &clause.binding;
    let body = &clause.body;

    match (&clause.type_path, clause.variant) {
        // Catch-all try catch
        (None, ChainVariant::Root) => {
            checks::gen_catchall_check(binding, &clause.guard, body, CheckAction::ReturnDirect, true)
        }
        // Typed try catch
        (Some(type_path), variant) => {
            checks::gen_typed_check(variant, type_path, binding, &clause.guard, body, CheckAction::ReturnDirect)
        }
        // Invalid: catch-all with any/all variant
        (None, _) => {
            syn::Error::new(binding.span(), "try catch any/all requires a type")
                .to_compile_error()
        }
    }
}

/// Represents a handler in the chain for nested if-else generation.
enum HandlerSpec {
    /// catch Type(binding) [guard] { body }
    Catch {
        type_path: Option<TokenStream>,
        variant: ChainVariant,
        binding: Ident,
        guard: Option<Guard>,
        body: TokenStream,
    },
    /// throw Type(binding) [guard] { expr }
    Throw {
        type_path: Option<TokenStream>,
        variant: ChainVariant,
        binding: Option<Ident>,
        guard: Option<Guard>,
        throw_expr: TokenStream,
    },
    /// inspect Type(binding) [guard] { body }
    Inspect {
        type_path: Option<TokenStream>,
        variant: ChainVariant,
        binding: Ident,
        guard: Option<Guard>,
        body: TokenStream,
    },
    /// try catch Type(binding) [guard] { body }
    TryCatch {
        type_path: Option<TokenStream>,
        variant: ChainVariant,
        binding: Ident,
        guard: Option<Guard>,
        body: TokenStream,
    },
}

/// Build a nested if-else chain that returns direct values (for control flow cases).
fn build_nested_chain_direct(handlers: &[HandlerSpec], fallback: TokenStream) -> TokenStream {
    if handlers.is_empty() {
        return fallback;
    }

    let mut result = fallback;

    for handler in handlers.iter().rev() {
        result = match handler {
            HandlerSpec::Catch { type_path, variant, binding, guard, body } => {
                gen_catch_nested_direct(type_path.as_ref(), *variant, binding, guard, body, result)
            }
            HandlerSpec::Throw { type_path, variant, binding, guard, throw_expr } => {
                // Throw in direct mode should use panic or unreachable
                gen_throw_nested_direct(type_path.as_ref(), *variant, binding.as_ref(), guard, throw_expr, result)
            }
            HandlerSpec::Inspect { type_path, variant, binding, guard, body } => {
                gen_inspect_nested(type_path.as_ref(), *variant, binding, guard, body, result)
            }
            HandlerSpec::TryCatch { type_path, variant, binding, guard, body } => {
                // try catch in direct mode extracts Ok value or executes control flow
                gen_try_catch_nested_direct(type_path.as_ref(), *variant, binding, guard, body, result)
            }
        };
    }

    result
}

/// Generate catch handler in direct mode - returns value directly, not wrapped in Result.
fn gen_catch_nested_direct(
    type_path: Option<&TokenStream>,
    variant: ChainVariant,
    binding: &Ident,
    guard: &Option<Guard>,
    body: &TokenStream,
    else_branch: TokenStream,
) -> TokenStream {
    // In direct mode, the body should be control flow (continue/break) or a direct value
    chain_builder::gen_handler(type_path, variant, binding, guard, body, else_branch)
}

/// Generate throw handler in direct mode - transforms error and continues chain.
fn gen_throw_nested_direct(
    type_path: Option<&TokenStream>,
    variant: ChainVariant,
    binding: Option<&Ident>,
    guard: &Option<Guard>,
    throw_expr: &TokenStream,
    else_branch: TokenStream,
) -> TokenStream {
    let binding_ident = binding.cloned().unwrap_or_else(|| {
        syn::Ident::new("_", proc_macro2::Span::call_site())
    });

    // In direct mode, throw transforms the error and continues to the next handler.
    // We need to reassign __err and then fall through to else_branch.
    let transform_body = quote! {
        {
            #[allow(unused_imports)]
            use ::handle_this::__Thrown;
            let __new_err = ::handle_this::__ThrowExpr(#throw_expr).__thrown()
                .frame(file!(), line!(), column!());
            __err = __new_err.chain_after(__err);
        }
    };

    // Untyped throw needs special handling for binding (uses &__err for condition check)
    if type_path.is_none() {
        let binding_str = binding_ident.to_string();
        let bind_stmt = if binding_str == "_" {
            quote! {}
        } else {
            quote! { let #binding_ident = &__err; }
        };
        return match guard {
            Some(Guard::When(cond)) => quote! {
                {
                    #bind_stmt
                    if #cond {
                        #transform_body
                    }
                    #else_branch
                }
            },
            _ => quote! {
                {
                    #bind_stmt
                    #transform_body
                    #else_branch
                }
            },
        };
    }

    // Typed throw: if type matches, transform error. Always continue to else_branch.
    // Unlike catch (which returns), throw is a side effect that transforms __err.
    let type_path = type_path.unwrap(); // We know it's Some from the check above

    // Generate type check with transform as side effect, then continue to else_branch
    match variant {
        ChainVariant::Root => {
            let inner = match guard {
                Some(Guard::When(cond)) => quote! {
                    if let ::core::option::Option::Some(__typed_err) = __err.downcast_ref::<#type_path>() {
                        let #binding_ident = __typed_err;
                        if #cond {
                            #transform_body
                        }
                    }
                },
                Some(Guard::Match { expr, arms }) => quote! {
                    if let ::core::option::Option::Some(__typed_err) = __err.downcast_ref::<#type_path>() {
                        let #binding_ident = __typed_err;
                        #[allow(unused_imports)]
                        use ::handle_this::__Thrown;
                        let __new_err = ::handle_this::__ThrowExpr(match #expr { #arms }).__thrown()
                            .frame(file!(), line!(), column!());
                        __err = __new_err.chain_after(__err);
                    }
                },
                None => quote! {
                    if let ::core::option::Option::Some(__typed_err) = __err.downcast_ref::<#type_path>() {
                        let #binding_ident = __typed_err;
                        #transform_body
                    }
                },
            };
            quote! {
                {
                    #inner
                    #else_branch
                }
            }
        }
        ChainVariant::Any => {
            let inner = match guard {
                Some(Guard::When(cond)) => quote! {
                    if let ::core::option::Option::Some(__typed_err) = __err.chain_any::<#type_path>() {
                        let #binding_ident = __typed_err;
                        if #cond {
                            #transform_body
                        }
                    }
                },
                Some(Guard::Match { expr, arms }) => quote! {
                    if let ::core::option::Option::Some(__typed_err) = __err.chain_any::<#type_path>() {
                        let #binding_ident = __typed_err;
                        #[allow(unused_imports)]
                        use ::handle_this::__Thrown;
                        let __new_err = ::handle_this::__ThrowExpr(match #expr { #arms }).__thrown()
                            .frame(file!(), line!(), column!());
                        __err = __new_err.chain_after(__err);
                    }
                },
                None => quote! {
                    if let ::core::option::Option::Some(__typed_err) = __err.chain_any::<#type_path>() {
                        let #binding_ident = __typed_err;
                        #transform_body
                    }
                },
            };
            quote! {
                {
                    #inner
                    #else_branch
                }
            }
        }
        ChainVariant::All => {
            let inner = match guard {
                Some(Guard::When(cond)) => quote! {
                    {
                        let #binding_ident: ::std::vec::Vec<&#type_path> = __err.chain_all::<#type_path>();
                        if !#binding_ident.is_empty() && #cond {
                            #transform_body
                        }
                    }
                },
                Some(Guard::Match { expr, arms }) => quote! {
                    {
                        let #binding_ident: ::std::vec::Vec<&#type_path> = __err.chain_all::<#type_path>();
                        if !#binding_ident.is_empty() {
                            #[allow(unused_imports)]
                            use ::handle_this::__Thrown;
                            let __new_err = ::handle_this::__ThrowExpr(match #expr { #arms }).__thrown()
                                .frame(file!(), line!(), column!());
                            __err = __new_err.chain_after(__err);
                        }
                    }
                },
                None => quote! {
                    {
                        let #binding_ident: ::std::vec::Vec<&#type_path> = __err.chain_all::<#type_path>();
                        if !#binding_ident.is_empty() {
                            #transform_body
                        }
                    }
                },
            };
            quote! {
                {
                    #inner
                    #else_branch
                }
            }
        }
    }
}

/// Generate try catch handler in direct mode.
fn gen_try_catch_nested_direct(
    type_path: Option<&TokenStream>,
    variant: ChainVariant,
    binding: &Ident,
    guard: &Option<Guard>,
    body: &TokenStream,
    else_branch: TokenStream,
) -> TokenStream {
    // try catch returns Result, so we need to extract Ok or continue with Err
    // In direct mode, assume the body handles control flow
    let direct_body = quote! {
        {
            #[allow(unused_imports)]
            use ::handle_this::result::{Ok, Err};
            match (#body) {
                ::core::result::Result::Ok(__v) => __v,
                ::core::result::Result::Err(__e) => {
                    // Error from try catch - should be unreachable if there's a catch-all
                    ::core::panic!("try catch returned error: {:?}", __e)
                }
            }
        }
    };

    chain_builder::gen_handler(type_path, variant, binding, guard, &direct_body, else_branch)
}

/// Generate nested inspect handler.
/// Inspect runs side effects but doesn't stop the chain - always continues to else_branch.
fn gen_inspect_nested(
    type_path: Option<&TokenStream>,
    variant: ChainVariant,
    binding: &Ident,
    guard: &Option<Guard>,
    body: &TokenStream,
    else_branch: TokenStream,
) -> TokenStream {
    chain_builder::gen_inspect_handler(type_path, variant, binding, guard, body, else_branch)
}

/// Generate catch check statements.
pub(crate) fn generate_catch_checks(catches: &[CatchClause]) -> Vec<TokenStream> {
    catches.iter().map(|clause| {
        let binding = &clause.binding;
        let body = &clause.body;

        match (&clause.type_path, clause.variant) {
            // Catch-all
            (None, ChainVariant::Root) => {
                checks::gen_catchall_check(binding, &clause.guard, body, CheckAction::ReturnOk, true)
            }
            // Typed catch
            (Some(type_path), variant) => {
                checks::gen_typed_check(variant, type_path, binding, &clause.guard, body, CheckAction::ReturnOk)
            }
            // Invalid: catch-all with any/all variant
            (None, _) => {
                syn::Error::new(binding.span(), "catch any/all requires a type")
                    .to_compile_error()
            }
        }
    }).collect()
}

/// Generate throw check statements.
/// Throw transforms the error and continues the chain.
pub(crate) fn generate_throw_checks(throws: &[ThrowClause]) -> Vec<TokenStream> {
    throws.iter().map(|clause| {
        let throw_expr = &clause.throw_expr;
        let binding = clause.binding.as_ref();

        match (&clause.type_path, clause.variant) {
            // Untyped throw - transforms error
            (None, _) => {
                let binding_ident = binding.cloned().unwrap_or_else(|| {
                    syn::Ident::new("_", proc_macro2::Span::call_site())
                });
                checks::gen_catchall_check(&binding_ident, &clause.guard, throw_expr, CheckAction::Transform, false)
            }
            // Typed throw - transforms error if type matches
            (Some(type_path), variant) => {
                let binding_ident = binding.cloned().unwrap_or_else(|| {
                    syn::Ident::new("_", proc_macro2::Span::call_site())
                });
                checks::gen_typed_check(variant, type_path, &binding_ident, &clause.guard, throw_expr, CheckAction::Transform)
            }
        }
    }).collect()
}

/// Generate inspect check statements.
pub(crate) fn generate_inspect_checks(inspects: &[InspectClause]) -> Vec<TokenStream> {
    inspects.iter().map(|clause| {
        let binding = &clause.binding;
        let body = &clause.body;

        match (&clause.type_path, clause.variant) {
            // Untyped inspect
            (None, _) => {
                checks::gen_catchall_check(binding, &clause.guard, body, CheckAction::Execute, false)
            }
            // Typed inspect
            (Some(type_path), variant) => {
                checks::gen_typed_check(variant, type_path, binding, &clause.guard, body, CheckAction::Execute)
            }
        }
    }).collect()
}

// ============================================================
// Inline Mode Generation (for nested try blocks with control flow)
// ============================================================

/// Generate handler chain for inline mode.
///
/// In inline mode, we're NOT inside a closure, so we can't use `return`.
/// Instead, we generate a nested if-else chain where each branch produces
/// the `Result` value directly.
///
/// This is used when the body contains control flow from nested try blocks
/// (break/continue) that must escape without being trapped by closures.
fn generate_inline_handler_chain(
    input: &SyncTryInput,
    has_catch_all: bool,
    _ctx_chain: &TokenStream,
) -> TokenStream {
    // Build a list of inline handler specs
    let mut handlers = Vec::new();

    for handler in &input.handlers {
        match handler {
            Handler::Catch(catch) => {
                handlers.push(InlineHandlerSpec::Catch {
                    type_path: catch.type_path.clone(),
                    variant: catch.variant,
                    binding: catch.binding.clone(),
                    guard: catch.guard.clone(),
                    body: transform_nested(catch.body.clone()),
                });
            }
            Handler::Throw(throw) => {
                handlers.push(InlineHandlerSpec::Throw {
                    type_path: throw.type_path.clone(),
                    variant: throw.variant,
                    binding: throw.binding.clone(),
                    guard: throw.guard.clone(),
                    throw_expr: transform_nested(throw.throw_expr.clone()),
                });
            }
            Handler::Inspect(inspect) => {
                handlers.push(InlineHandlerSpec::Inspect {
                    type_path: inspect.type_path.clone(),
                    variant: inspect.variant,
                    binding: inspect.binding.clone(),
                    guard: inspect.guard.clone(),
                    body: transform_nested(inspect.body.clone()),
                });
            }
            Handler::TryCatch(try_catch) => {
                handlers.push(InlineHandlerSpec::TryCatch {
                    type_path: try_catch.type_path.clone(),
                    variant: try_catch.variant,
                    binding: try_catch.binding.clone(),
                    guard: try_catch.guard.clone(),
                    body: transform_nested(try_catch.body.clone()),
                });
            }
        }
    }

    // Fallback depends on whether we have a catch-all
    let fallback = if has_catch_all {
        quote! {
            ::core::unreachable!("catch-all handler should have handled all errors")
        }
    } else {
        quote! {
            ::core::result::Result::Err(__err)
        }
    };

    build_inline_chain(&handlers, fallback)
}

/// Represents a handler in the inline chain.
enum InlineHandlerSpec {
    Catch {
        type_path: Option<TokenStream>,
        variant: ChainVariant,
        binding: Ident,
        guard: Option<Guard>,
        body: TokenStream,
    },
    Throw {
        type_path: Option<TokenStream>,
        variant: ChainVariant,
        binding: Option<Ident>,
        guard: Option<Guard>,
        throw_expr: TokenStream,
    },
    Inspect {
        type_path: Option<TokenStream>,
        variant: ChainVariant,
        binding: Ident,
        guard: Option<Guard>,
        body: TokenStream,
    },
    TryCatch {
        type_path: Option<TokenStream>,
        variant: ChainVariant,
        binding: Ident,
        guard: Option<Guard>,
        body: TokenStream,
    },
}

/// Build inline handler chain - returns Result values directly without return statements.
fn build_inline_chain(handlers: &[InlineHandlerSpec], fallback: TokenStream) -> TokenStream {
    if handlers.is_empty() {
        return fallback;
    }

    let mut result = fallback;

    // Build chain from back to front
    for handler in handlers.iter().rev() {
        result = match handler {
            InlineHandlerSpec::Catch { type_path, variant, binding, guard, body } => {
                gen_inline_catch(type_path.as_ref(), *variant, binding, guard, body, result)
            }
            InlineHandlerSpec::Throw { type_path, variant, binding, guard, throw_expr } => {
                gen_inline_throw(type_path.as_ref(), *variant, binding.as_ref(), guard, throw_expr, result)
            }
            InlineHandlerSpec::Inspect { type_path, variant, binding, guard, body } => {
                gen_inline_inspect(type_path.as_ref(), *variant, binding, guard, body, result)
            }
            InlineHandlerSpec::TryCatch { type_path, variant, binding, guard, body } => {
                gen_inline_try_catch(type_path.as_ref(), *variant, binding, guard, body, result)
            }
        };
    }

    result
}

/// Generate inline catch handler - returns Ok(body) directly.
fn gen_inline_catch(
    type_path: Option<&TokenStream>,
    variant: ChainVariant,
    binding: &Ident,
    guard: &Option<Guard>,
    body: &TokenStream,
    else_branch: TokenStream,
) -> TokenStream {
    // Catch produces Ok(body) directly
    let ok_body = quote! { ::core::result::Result::Ok({ #body }) };

    match type_path {
        None => {
            // Catch-all
            match guard {
                Some(Guard::When(cond)) => quote! {
                    {
                        let #binding = &__err;
                        if #cond {
                            #ok_body
                        } else {
                            #else_branch
                        }
                    }
                },
                Some(Guard::Match { expr, arms }) => quote! {
                    {
                        let #binding = &__err;
                        ::core::result::Result::Ok(match #expr { #arms })
                    }
                },
                None => quote! {
                    {
                        let #binding = __err;
                        #ok_body
                    }
                },
            }
        }
        Some(tp) => {
            // Typed catch
            let type_check = match variant {
                ChainVariant::Root => quote! { __err.downcast_ref::<#tp>() },
                ChainVariant::Any => quote! { __err.chain_any::<#tp>() },
                ChainVariant::All => quote! { __err.chain_all::<#tp>() },
            };

            let is_all = matches!(variant, ChainVariant::All);

            if is_all {
                match guard {
                    Some(Guard::When(cond)) => quote! {
                        {
                            let #binding: ::std::vec::Vec<&#tp> = #type_check;
                            if !#binding.is_empty() && #cond {
                                #ok_body
                            } else {
                                #else_branch
                            }
                        }
                    },
                    Some(Guard::Match { expr, arms }) => quote! {
                        {
                            let #binding: ::std::vec::Vec<&#tp> = #type_check;
                            if !#binding.is_empty() {
                                ::core::result::Result::Ok(match #expr { #arms })
                            } else {
                                #else_branch
                            }
                        }
                    },
                    None => quote! {
                        {
                            let #binding: ::std::vec::Vec<&#tp> = #type_check;
                            if !#binding.is_empty() {
                                #ok_body
                            } else {
                                #else_branch
                            }
                        }
                    },
                }
            } else {
                match guard {
                    Some(Guard::When(cond)) => quote! {
                        if let ::core::option::Option::Some(__typed_err) = #type_check {
                            let #binding = __typed_err;
                            if #cond {
                                #ok_body
                            } else {
                                #else_branch
                            }
                        } else {
                            #else_branch
                        }
                    },
                    Some(Guard::Match { expr, arms }) => quote! {
                        if let ::core::option::Option::Some(__typed_err) = #type_check {
                            let #binding = __typed_err;
                            ::core::result::Result::Ok(match #expr { #arms })
                        } else {
                            #else_branch
                        }
                    },
                    None => quote! {
                        if let ::core::option::Option::Some(__typed_err) = #type_check {
                            let #binding = __typed_err;
                            #ok_body
                        } else {
                            #else_branch
                        }
                    },
                }
            }
        }
    }
}

/// Generate inline throw handler - transforms error, continues chain.
fn gen_inline_throw(
    type_path: Option<&TokenStream>,
    variant: ChainVariant,
    binding: Option<&Ident>,
    guard: &Option<Guard>,
    throw_expr: &TokenStream,
    else_branch: TokenStream,
) -> TokenStream {
    let binding_ident = binding.cloned().unwrap_or_else(|| {
        syn::Ident::new("_", proc_macro2::Span::call_site())
    });

    // Transform body - updates __err and falls through
    let transform_body = quote! {
        {
            #[allow(unused_imports)]
            use ::handle_this::__Thrown;
            let __new_err = ::handle_this::__ThrowExpr(#throw_expr).__thrown()
                .frame(file!(), line!(), column!());
            __err = __new_err.chain_after(__err);
        }
    };

    match type_path {
        None => {
            // Untyped throw
            let bind_stmt = if binding_ident.to_string() == "_" {
                quote! {}
            } else {
                quote! { let #binding_ident = &__err; }
            };

            match guard {
                Some(Guard::When(cond)) => quote! {
                    {
                        #bind_stmt
                        if #cond {
                            #transform_body
                        }
                        #else_branch
                    }
                },
                _ => quote! {
                    {
                        #bind_stmt
                        #transform_body
                        #else_branch
                    }
                },
            }
        }
        Some(tp) => {
            // Typed throw
            let type_check = match variant {
                ChainVariant::Root => quote! { __err.downcast_ref::<#tp>() },
                ChainVariant::Any => quote! { __err.chain_any::<#tp>() },
                ChainVariant::All => quote! { __err.chain_all::<#tp>() },
            };

            let is_all = matches!(variant, ChainVariant::All);

            if is_all {
                match guard {
                    Some(Guard::When(cond)) => quote! {
                        {
                            let #binding_ident: ::std::vec::Vec<&#tp> = #type_check;
                            if !#binding_ident.is_empty() && #cond {
                                #transform_body
                            }
                            #else_branch
                        }
                    },
                    _ => quote! {
                        {
                            let #binding_ident: ::std::vec::Vec<&#tp> = #type_check;
                            if !#binding_ident.is_empty() {
                                #transform_body
                            }
                            #else_branch
                        }
                    },
                }
            } else {
                match guard {
                    Some(Guard::When(cond)) => quote! {
                        {
                            if let ::core::option::Option::Some(__typed_err) = #type_check {
                                let #binding_ident = __typed_err;
                                if #cond {
                                    #transform_body
                                }
                            }
                            #else_branch
                        }
                    },
                    _ => quote! {
                        {
                            if let ::core::option::Option::Some(__typed_err) = #type_check {
                                let #binding_ident = __typed_err;
                                #transform_body
                            }
                            #else_branch
                        }
                    },
                }
            }
        }
    }
}

/// Generate inline inspect handler - runs side effect, always continues.
fn gen_inline_inspect(
    type_path: Option<&TokenStream>,
    variant: ChainVariant,
    binding: &Ident,
    guard: &Option<Guard>,
    body: &TokenStream,
    else_branch: TokenStream,
) -> TokenStream {
    match type_path {
        None => {
            // Untyped inspect
            match guard {
                Some(Guard::When(cond)) => quote! {
                    {
                        let #binding = &__err;
                        if #cond {
                            { #body }
                        }
                        #else_branch
                    }
                },
                _ => quote! {
                    {
                        let #binding = &__err;
                        { #body }
                        #else_branch
                    }
                },
            }
        }
        Some(tp) => {
            // Typed inspect
            let type_check = match variant {
                ChainVariant::Root => quote! { __err.downcast_ref::<#tp>() },
                ChainVariant::Any => quote! { __err.chain_any::<#tp>() },
                ChainVariant::All => quote! { __err.chain_all::<#tp>() },
            };

            let is_all = matches!(variant, ChainVariant::All);

            if is_all {
                match guard {
                    Some(Guard::When(cond)) => quote! {
                        {
                            let #binding: ::std::vec::Vec<&#tp> = #type_check;
                            if !#binding.is_empty() && #cond {
                                { #body }
                            }
                            #else_branch
                        }
                    },
                    _ => quote! {
                        {
                            let #binding: ::std::vec::Vec<&#tp> = #type_check;
                            if !#binding.is_empty() {
                                { #body }
                            }
                            #else_branch
                        }
                    },
                }
            } else {
                match guard {
                    Some(Guard::When(cond)) => quote! {
                        {
                            if let ::core::option::Option::Some(__typed_err) = #type_check {
                                let #binding = __typed_err;
                                if #cond {
                                    { #body }
                                }
                            }
                            #else_branch
                        }
                    },
                    _ => quote! {
                        {
                            if let ::core::option::Option::Some(__typed_err) = #type_check {
                                let #binding = __typed_err;
                                { #body }
                            }
                            #else_branch
                        }
                    },
                }
            }
        }
    }
}

/// Generate inline try catch handler - returns result directly.
fn gen_inline_try_catch(
    type_path: Option<&TokenStream>,
    variant: ChainVariant,
    binding: &Ident,
    guard: &Option<Guard>,
    body: &TokenStream,
    else_branch: TokenStream,
) -> TokenStream {
    // try catch body returns Result, we return it directly
    let result_body = quote! {
        {
            #[allow(unused_imports)]
            use ::handle_this::result::{Ok, Err};
            #body
        }
    };

    match type_path {
        None => {
            // Catch-all try catch
            match guard {
                Some(Guard::When(cond)) => quote! {
                    {
                        let #binding = &__err;
                        if #cond {
                            #result_body
                        } else {
                            #else_branch
                        }
                    }
                },
                Some(Guard::Match { expr, arms }) => quote! {
                    {
                        let #binding = &__err;
                        match #expr { #arms }
                    }
                },
                None => quote! {
                    {
                        let #binding = __err;
                        #result_body
                    }
                },
            }
        }
        Some(tp) => {
            // Typed try catch
            let type_check = match variant {
                ChainVariant::Root => quote! { __err.downcast_ref::<#tp>() },
                ChainVariant::Any => quote! { __err.chain_any::<#tp>() },
                ChainVariant::All => quote! { __err.chain_all::<#tp>() },
            };

            let is_all = matches!(variant, ChainVariant::All);

            if is_all {
                match guard {
                    Some(Guard::When(cond)) => quote! {
                        {
                            let #binding: ::std::vec::Vec<&#tp> = #type_check;
                            if !#binding.is_empty() && #cond {
                                #result_body
                            } else {
                                #else_branch
                            }
                        }
                    },
                    Some(Guard::Match { expr, arms }) => quote! {
                        {
                            let #binding: ::std::vec::Vec<&#tp> = #type_check;
                            if !#binding.is_empty() {
                                match #expr { #arms }
                            } else {
                                #else_branch
                            }
                        }
                    },
                    None => quote! {
                        {
                            let #binding: ::std::vec::Vec<&#tp> = #type_check;
                            if !#binding.is_empty() {
                                #result_body
                            } else {
                                #else_branch
                            }
                        }
                    },
                }
            } else {
                match guard {
                    Some(Guard::When(cond)) => quote! {
                        if let ::core::option::Option::Some(__typed_err) = #type_check {
                            let #binding = __typed_err;
                            if #cond {
                                #result_body
                            } else {
                                #else_branch
                            }
                        } else {
                            #else_branch
                        }
                    },
                    Some(Guard::Match { expr, arms }) => quote! {
                        if let ::core::option::Option::Some(__typed_err) = #type_check {
                            let #binding = __typed_err;
                            match #expr { #arms }
                        } else {
                            #else_branch
                        }
                    },
                    None => quote! {
                        if let ::core::option::Option::Some(__typed_err) = #type_check {
                            let #binding = __typed_err;
                            #result_body
                        } else {
                            #else_branch
                        }
                    },
                }
            }
        }
    }
}
