//! Then chain pattern: `try { }, then |x| { }, then |y| { } catch { }`
//!
//! Chains operations together, passing success values through the pipeline.
//! Supports all try variants as the source:
//! - `try { }` - basic
//! - `try -> T { }` - direct mode
//! - `try for i in iter { }` - first success iteration
//! - `try any i in iter { }` - alias for try for
//! - `try all i in iter { }` - collect all
//! - `try while cond { }` - retry
//! - `async try { }` - async

use proc_macro2::{TokenStream, TokenTree, Span};
use quote::quote;
use syn::parse::{Parse, ParseStream};
use syn::ext::IdentExt;
use syn::{Result, Error, Ident, braced, Token, Expr, Pat};

use crate::keywords::{self, peek_keyword, GenContext};
use crate::keywords::with_ctx::{self, WithClause};
use crate::nested;
use crate::patterns::r#try::common::Handler;

/// Source type for the first step in the chain.
#[derive(Debug)]
enum SourceType {
    /// Basic try { body }
    Basic { body: TokenStream },
    /// Direct mode try -> T { body }
    Direct { ret_type: TokenStream, body: TokenStream },
    /// try for binding in iter { body }
    For { binding: Pat, iter: Expr, body: TokenStream },
    /// try any binding in iter { body }
    Any { binding: Pat, iter: Expr, body: TokenStream },
    /// try all binding in iter { body }
    All { binding: Pat, iter: Expr, body: TokenStream },
    /// try while condition { body }
    While { condition: Expr, body: TokenStream },
    /// async try { body }
    Async { body: TokenStream },
}

/// A then step in the chain.
struct ThenStep {
    /// The binding for this step
    binding: Ident,
    /// Optional type annotation for the binding
    binding_type: Option<TokenStream>,
    /// The body of this step
    body: TokenStream,
    /// Optional context (with "msg", { key: value })
    with_clause: Option<WithClause>,
}

/// Parsed then chain input.
struct ThenChainInput {
    /// The source (first try expression)
    source: SourceType,
    /// Optional with clause for source
    source_with: Option<WithClause>,
    /// The then steps
    then_steps: Vec<ThenStep>,
    /// Handlers in declaration order
    handlers: Vec<Handler>,
    /// Optional else clause (for direct mode)
    else_body: Option<TokenStream>,
    /// Optional finally body
    finally: Option<TokenStream>,
}

impl Parse for ThenChainInput {
    fn parse(input: ParseStream) -> Result<Self> {
        // First token might be THEN (from iter routing) or the actual source type
        let mut source_type: Ident = input.parse()?;
        if source_type == "THEN" {
            // Skip THEN marker, get actual source type
            source_type = input.parse()?;
        }
        let source_type_str = source_type.to_string();

        // Parse source based on type
        let (source, source_with) = match source_type_str.as_str() {
            "BASIC" => {
                let body = parse_braced_body(input)?;
                let with_clause = parse_optional_with(input)?;
                (SourceType::Basic { body }, with_clause)
            }
            "DIRECT" => {
                input.parse::<Token![->]>()?;
                // Parse type until we hit a brace
                let mut type_tokens = Vec::new();
                while !input.peek(syn::token::Brace) {
                    let tt: TokenTree = input.parse()?;
                    type_tokens.push(tt);
                }
                let ret_type: TokenStream = type_tokens.into_iter().collect();
                let body = parse_braced_body(input)?;
                let with_clause = parse_optional_with(input)?;
                (SourceType::Direct { ret_type, body }, with_clause)
            }
            "FOR" => {
                // Parse: binding in iter { body }
                let binding = Pat::parse_single(input)?;
                input.parse::<Token![in]>()?;
                // Parse iter tokens until we hit the brace
                let iter = parse_expr_until_brace(input)?;
                let body = parse_braced_body(input)?;
                let with_clause = parse_optional_with(input)?;
                (SourceType::For { binding, iter, body }, with_clause)
            }
            "ANY" => {
                let binding = Pat::parse_single(input)?;
                input.parse::<Token![in]>()?;
                let iter = parse_expr_until_brace(input)?;
                let body = parse_braced_body(input)?;
                let with_clause = parse_optional_with(input)?;
                (SourceType::Any { binding, iter, body }, with_clause)
            }
            "ALL" => {
                let binding = Pat::parse_single(input)?;
                input.parse::<Token![in]>()?;
                let iter = parse_expr_until_brace(input)?;
                let body = parse_braced_body(input)?;
                let with_clause = parse_optional_with(input)?;
                (SourceType::All { binding, iter, body }, with_clause)
            }
            "WHILE" => {
                // Parse condition tokens until we hit the brace
                let condition = parse_expr_until_brace(input)?;
                let body = parse_braced_body(input)?;
                let with_clause = parse_optional_with(input)?;
                (SourceType::While { condition, body }, with_clause)
            }
            "ASYNC" => {
                let body = parse_braced_body(input)?;
                let with_clause = parse_optional_with(input)?;
                (SourceType::Async { body }, with_clause)
            }
            other => {
                return Err(Error::new(source_type.span(), format!("unknown source type: {}", other)));
            }
        };

        // Expect comma after source
        if input.peek(Token![,]) {
            input.parse::<Token![,]>()?;
        }

        // Parse then steps
        let mut then_steps = Vec::new();
        while !input.is_empty() {
            if peek_keyword(input, "then") {
                input.parse::<Ident>()?; // consume `then`

                // Parse binding: |x| or |x: Type|
                input.parse::<Token![|]>()?;
                let binding: Ident = if input.peek(Token![_]) {
                    input.parse::<Token![_]>()?;
                    Ident::new("_", Span::call_site())
                } else {
                    input.parse()?
                };

                // Optional type annotation
                let binding_type = if input.peek(Token![:]) {
                    input.parse::<Token![:]>()?;
                    let mut type_tokens = Vec::new();
                    while !input.peek(Token![|]) {
                        let tt: TokenTree = input.parse()?;
                        type_tokens.push(tt);
                    }
                    Some(type_tokens.into_iter().collect())
                } else {
                    None
                };

                input.parse::<Token![|]>()?;

                // Parse body
                let body = parse_braced_body(input)?;
                let with_clause = parse_optional_with(input)?;

                then_steps.push(ThenStep {
                    binding,
                    binding_type,
                    body,
                    with_clause,
                });

                // Expect comma after step if there are more
                if input.peek(Token![,]) {
                    input.parse::<Token![,]>()?;
                }
            } else {
                // Not a then, break to parse handlers
                break;
            }
        }

        // Parse optional handlers in declaration order
        let mut handlers = Vec::new();
        let mut else_body = None;
        let mut finally = None;

        while !input.is_empty() {
            if peek_keyword(input, "catch") {
                handlers.push(Handler::Catch(keywords::catch::parse(input)?));
            } else if peek_keyword(input, "throw") {
                handlers.push(Handler::Throw(keywords::throw::parse(input)?));
            } else if peek_keyword(input, "inspect") {
                handlers.push(Handler::Inspect(keywords::inspect::parse(input)?));
            } else if peek_keyword(input, "else") {
                crate::keywords::parse_keyword(input, "else")?; // consume `else`
                else_body = Some(parse_braced_body(input)?);
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

        Ok(ThenChainInput {
            source,
            source_with,
            then_steps,
            handlers,
            else_body,
            finally,
        })
    }
}

fn parse_braced_body(input: ParseStream) -> Result<TokenStream> {
    let content;
    braced!(content in input);
    content.parse()
}

/// Parse an expression by collecting tokens until we hit a brace.
fn parse_expr_until_brace(input: ParseStream) -> Result<Expr> {
    let mut tokens = Vec::new();
    while !input.peek(syn::token::Brace) && !input.is_empty() {
        let tt: TokenTree = input.parse()?;
        tokens.push(tt);
    }
    let ts: TokenStream = tokens.into_iter().collect();
    syn::parse2(ts)
}

fn parse_optional_with(input: ParseStream) -> Result<Option<WithClause>> {
    if peek_keyword(input, "with") {
        Ok(Some(with_ctx::parse(input)?))
    } else {
        Ok(None)
    }
}

/// Generate context chain (`.ctx(...)`, `.kv(...)`) from WithClause.
fn gen_with_chain(with_clause: &Option<WithClause>) -> TokenStream {
    match with_clause {
        Some(wc) => {
            let mut ctx = GenContext::new();
            with_ctx::apply_to_context(wc, &mut ctx);
            with_ctx::gen_ctx_chain(&ctx)
        }
        None => TokenStream::new(),
    }
}

/// Process then chain pattern.
pub fn process(input: TokenStream) -> Result<TokenStream> {
    let parsed: ThenChainInput = syn::parse2(input)?;
    Ok(generate(parsed))
}

/// Generate code for then chain.
fn generate(input: ThenChainInput) -> TokenStream {
    let source_ctx_chain = gen_with_chain(&input.source_with);

    // Generate the source expression based on type
    let source_expr = match &input.source {
        SourceType::Basic { body } => {
            let body = nested::transform_nested(body.clone());
            quote! {
                ::handle_this::__try_block!(#body)
                    .map_err(|__e| ::handle_this::__wrap_frame(__e, file!(), line!(), column!()) #source_ctx_chain)
            }
        }
        SourceType::Direct { ret_type, body } => {
            let body = nested::transform_nested(body.clone());
            // Direct mode: body returns T, we wrap in Ok
            quote! {
                (|| -> ::core::result::Result<#ret_type, ::handle_this::Handled> {
                    ::core::result::Result::Ok(#body)
                })()
            }
        }
        SourceType::For { binding, iter, body } => {
            let body = nested::transform_nested(body.clone());
            quote! {
                (|| -> ::core::result::Result<_, ::handle_this::Handled> {
                    let mut __last_err: Option<::handle_this::Handled> = None;
                    for #binding in #iter {
                        match ::handle_this::__try_block!(#body) {
                            ::core::result::Result::Ok(__v) => return ::core::result::Result::Ok(__v),
                            ::core::result::Result::Err(__e) => {
                                __last_err = Some(::handle_this::__wrap_frame(__e, file!(), line!(), column!()));
                                continue;
                            }
                        }
                    }
                    ::core::result::Result::Err(
                        __last_err.unwrap_or_else(|| ::handle_this::Handled::from("no iterations")) #source_ctx_chain
                    )
                })()
            }
        }
        SourceType::Any { binding, iter, body } => {
            // Same as For
            let body = nested::transform_nested(body.clone());
            quote! {
                (|| -> ::core::result::Result<_, ::handle_this::Handled> {
                    let mut __last_err: Option<::handle_this::Handled> = None;
                    for #binding in #iter {
                        match ::handle_this::__try_block!(#body) {
                            ::core::result::Result::Ok(__v) => return ::core::result::Result::Ok(__v),
                            ::core::result::Result::Err(__e) => {
                                __last_err = Some(::handle_this::__wrap_frame(__e, file!(), line!(), column!()));
                                continue;
                            }
                        }
                    }
                    ::core::result::Result::Err(
                        __last_err.unwrap_or_else(|| ::handle_this::Handled::from("no iterations")) #source_ctx_chain
                    )
                })()
            }
        }
        SourceType::All { binding, iter, body } => {
            let body = nested::transform_nested(body.clone());
            quote! {
                (|| -> ::core::result::Result<::std::vec::Vec<_>, ::handle_this::Handled> {
                    let mut __results = ::std::vec::Vec::new();
                    for #binding in #iter {
                        match ::handle_this::__try_block!(#body) {
                            ::core::result::Result::Ok(__v) => __results.push(__v),
                            ::core::result::Result::Err(_) => {},
                        }
                    }
                    ::core::result::Result::Ok(__results)
                })()
            }
        }
        SourceType::While { condition, body } => {
            let body = nested::transform_nested(body.clone());
            quote! {
                (|| -> ::core::result::Result<_, ::handle_this::Handled> {
                    let mut __last_err: Option<::handle_this::Handled> = None;
                    while #condition {
                        match ::handle_this::__try_block!(#body) {
                            ::core::result::Result::Ok(__v) => return ::core::result::Result::Ok(__v),
                            ::core::result::Result::Err(__e) => {
                                __last_err = Some(::handle_this::__wrap_frame(__e, file!(), line!(), column!()));
                                continue;
                            }
                        }
                    }
                    ::core::result::Result::Err(
                        __last_err.unwrap_or_else(|| ::handle_this::Handled::from("condition was never true")) #source_ctx_chain
                    )
                })()
            }
        }
        SourceType::Async { body } => {
            let body = nested::transform_nested(body.clone());
            quote! {
                (async {
                    ::handle_this::__try_block!(#body)
                        .map_err(|__e| ::handle_this::__wrap_frame(__e, file!(), line!(), column!()) #source_ctx_chain)
                }).await
            }
        }
    };

    // Build the chain with then steps
    let mut chain = source_expr;

    for step in &input.then_steps {
        let binding = &step.binding;
        let body = nested::transform_nested(step.body.clone());
        let ctx_chain = gen_with_chain(&step.with_clause);

        let binding_with_type = if let Some(ref ty) = step.binding_type {
            quote! { #binding: #ty }
        } else {
            quote! { #binding }
        };

        // Check if source is async
        let is_async = matches!(input.source, SourceType::Async { .. });

        if is_async {
            chain = quote! {
                match #chain {
                    ::core::result::Result::Ok(#binding_with_type) => {
                        (async {
                            ::handle_this::__try_block!(#body)
                                .map_err(|__e| ::handle_this::__wrap_frame(__e, file!(), line!(), column!()) #ctx_chain)
                        }).await
                    }
                    ::core::result::Result::Err(__e) => ::core::result::Result::Err(__e),
                }
            };
        } else {
            chain = quote! {
                #chain.and_then(|#binding_with_type| {
                    ::handle_this::__try_block!(#body)
                        .map_err(|__e| ::handle_this::__wrap_frame(__e, file!(), line!(), column!()) #ctx_chain)
                })
            };
        }
    }

    // Check if we have any handlers
    let has_handlers = !input.handlers.is_empty();

    // Check if direct mode
    let is_direct = matches!(input.source, SourceType::Direct { .. });

    let code = if is_direct && input.else_body.is_some() && !has_handlers {
        // Direct mode with else only (no other handlers): unwrap_or_else
        let else_body = nested::transform_nested(input.else_body.as_ref().unwrap().clone());
        quote! {
            #chain.unwrap_or_else(|_| { #else_body })
        }
    } else if is_direct && input.else_body.is_some() && has_handlers {
        // Direct mode with handlers AND else: run handlers first, else as fallback
        let else_body = nested::transform_nested(input.else_body.as_ref().unwrap().clone());
        let handler_checks = generate_handler_checks_with_fallback(&input, Some(&else_body));

        quote! {
            #chain.or_else(|__raw_err| -> ::core::result::Result<_, ::handle_this::Handled> {
                #[allow(unreachable_code)]
                {
                    let mut __err = __raw_err;
                    #handler_checks
                }
            }).expect("direct mode with else should always succeed")
        }
    } else if is_direct && has_handlers {
        // Direct mode with catch/throw/inspect handlers (no else)
        // Use match to handle errors, then unwrap since catch provides fallback
        let handler_checks = generate_handler_checks(&input);

        quote! {
            #chain.or_else(|__raw_err| -> ::core::result::Result<_, ::handle_this::Handled> {
                #[allow(unreachable_code)]
                {
                    let mut __err = __raw_err;
                    #handler_checks
                }
            }).expect("direct mode requires catch-all handler")
        }
    } else if has_handlers && input.else_body.is_some() {
        // Non-direct mode with handlers AND else
        let else_body = nested::transform_nested(input.else_body.as_ref().unwrap().clone());
        let handler_checks = generate_handler_checks_with_fallback(&input, Some(&else_body));

        quote! {
            #chain.or_else(|__raw_err| -> ::core::result::Result<_, ::handle_this::Handled> {
                #[allow(unreachable_code)]
                {
                    let mut __err = __raw_err;
                    #handler_checks
                }
            })
        }
    } else if has_handlers {
        // Use or_else pattern for handlers
        let handler_checks = generate_handler_checks(&input);

        quote! {
            #chain.or_else(|__raw_err| -> ::core::result::Result<_, ::handle_this::Handled> {
                #[allow(unreachable_code)]
                {
                    let mut __err = __raw_err;
                    #handler_checks
                }
            })
        }
    } else {
        chain
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

/// Generate handler checks in declaration order.
fn generate_handler_checks(input: &ThenChainInput) -> TokenStream {
    generate_handler_checks_with_fallback(input, None)
}

/// Generate handler checks with optional else fallback.
fn generate_handler_checks_with_fallback(input: &ThenChainInput, else_body: Option<&TokenStream>) -> TokenStream {
    let mut checks = Vec::new();

    for handler in &input.handlers {
        let check = match handler {
            Handler::Catch(clause) => generate_catch_check(clause),
            Handler::Throw(clause) => generate_throw_check(clause),
            Handler::Inspect(clause) => generate_inspect_check(clause),
            Handler::TryCatch(_) => continue, // Not used in then chains
        };
        checks.push(check);
    }

    // Fallback: else body if provided, otherwise propagate error
    let fallback = if let Some(else_body) = else_body {
        quote! {
            ::core::result::Result::Ok({ #else_body })
        }
    } else {
        quote! {
            ::core::result::Result::Err(__err)
        }
    };

    quote! {
        #(#checks)*
        #fallback
    }
}

fn generate_catch_check(clause: &crate::keywords::catch::CatchClause) -> TokenStream {
    use crate::patterns::r#try::checks::{self, CheckAction};
    use crate::keywords::ChainVariant;

    let binding = &clause.binding;
    let body = &clause.body;

    match (&clause.type_path, clause.variant) {
        (None, ChainVariant::Root) => {
            checks::gen_catchall_check(binding, &clause.guard, body, CheckAction::ReturnOk, true)
        }
        (Some(type_path), variant) => {
            checks::gen_typed_check(variant, type_path, binding, &clause.guard, body, CheckAction::ReturnOk)
        }
        (None, _) => {
            syn::Error::new(binding.span(), "catch any/all requires a type")
                .to_compile_error()
        }
    }
}

fn generate_throw_check(clause: &crate::keywords::throw::ThrowClause) -> TokenStream {
    use crate::patterns::r#try::checks::{self, CheckAction};

    let throw_expr = &clause.throw_expr;
    let binding = clause.binding.as_ref();

    match (&clause.type_path, clause.variant) {
        (None, _) => {
            let binding_ident = binding.cloned().unwrap_or_else(|| {
                syn::Ident::new("_", proc_macro2::Span::call_site())
            });
            checks::gen_catchall_check(&binding_ident, &clause.guard, throw_expr, CheckAction::Transform, false)
        }
        (Some(type_path), variant) => {
            let binding_ident = binding.cloned().unwrap_or_else(|| {
                syn::Ident::new("_", proc_macro2::Span::call_site())
            });
            checks::gen_typed_check(variant, type_path, &binding_ident, &clause.guard, throw_expr, CheckAction::Transform)
        }
    }
}

fn generate_inspect_check(clause: &crate::keywords::inspect::InspectClause) -> TokenStream {
    use crate::patterns::r#try::checks::{self, CheckAction};

    let binding = &clause.binding;
    let body = &clause.body;

    match (&clause.type_path, clause.variant) {
        (None, _) => {
            checks::gen_catchall_check(binding, &clause.guard, body, CheckAction::Execute, false)
        }
        (Some(type_path), variant) => {
            checks::gen_typed_check(variant, type_path, binding, &clause.guard, body, CheckAction::Execute)
        }
    }
}
