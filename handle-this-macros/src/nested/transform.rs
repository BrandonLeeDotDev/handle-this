//! Main transformation logic for nested patterns.
//!
//! This module contains all the pattern transformation functions for converting
//! nested try/catch/throw/inspect patterns into macro invocations.

use proc_macro2::{TokenStream, TokenTree, Delimiter};
use quote::{quote, quote_spanned};

use super::detection::{contains_control_flow, contains_question_mark};

/// Recursively transform any nested handle! patterns in a token stream.
pub fn transform_nested(tokens: TokenStream) -> TokenStream {
    let tokens_vec: Vec<TokenTree> = tokens.into_iter().collect();
    transform_tokens(&tokens_vec)
}

/// Transform a slice of tokens, looking for nested patterns.
fn transform_tokens(tokens: &[TokenTree]) -> TokenStream {
    let mut result = Vec::new();
    let mut i = 0;

    while i < tokens.len() {
        // Check if this is a macro invocation: `name! { ... }`
        // If so, don't transform inside the macro - copy as-is
        if is_macro_invocation(tokens, i) {
            // Copy the ident
            result.push(tokens[i].clone());
            i += 1;
            // Copy the `!`
            result.push(tokens[i].clone());
            i += 1;
            // Copy the group (don't transform inside)
            if i < tokens.len() {
                if let TokenTree::Group(g) = &tokens[i] {
                    result.push(TokenTree::Group(g.clone()));
                    i += 1;
                }
            }
            continue;
        }

        // Check if we're at the start of a nested pattern
        if let Some((transformed, consumed)) = try_transform_pattern(&tokens[i..]) {
            result.push(TokenTree::Group(proc_macro2::Group::new(
                Delimiter::None,
                transformed,
            )));
            i += consumed;
        } else {
            // Not a pattern - copy token, but recurse into groups
            let token = &tokens[i];
            match token {
                TokenTree::Group(g) => {
                    let inner = transform_nested(g.stream());
                    let mut new_group = proc_macro2::Group::new(g.delimiter(), inner);
                    new_group.set_span(g.span());
                    result.push(TokenTree::Group(new_group));
                }
                _ => {
                    result.push(token.clone());
                }
            }
            i += 1;
        }
    }

    result.into_iter().collect()
}

/// Check if tokens[i] starts a macro invocation: `ident!` followed by a group
fn is_macro_invocation(tokens: &[TokenTree], i: usize) -> bool {
    if i + 2 >= tokens.len() {
        return false;
    }
    if let TokenTree::Ident(_) = &tokens[i] {
        if let TokenTree::Punct(p) = &tokens[i + 1] {
            if p.as_char() == '!' {
                if let TokenTree::Group(_) = &tokens[i + 2] {
                    return true;
                }
            }
        }
    }
    false
}

/// Try to match and transform a pattern starting at the given tokens.
fn try_transform_pattern(tokens: &[TokenTree]) -> Option<(TokenStream, usize)> {
    if tokens.is_empty() {
        return None;
    }

    if let TokenTree::Ident(ident) = &tokens[0] {
        let ident_str = ident.to_string();
        match ident_str.as_str() {
            // Check for `async try` keyword pair
            "async" if tokens.len() > 1 => {
                if let TokenTree::Ident(next) = &tokens[1] {
                    if *next == "try" {
                        return try_transform_async_try(tokens);
                    }
                }
                None
            }
            // Check for `try` keyword
            "try" => try_transform_try(tokens),
            // Check for `scope` keyword
            "scope" => try_transform_scope(tokens),
            // Check for `require` keyword
            "require" => try_transform_require(tokens),
            _ => None,
        }
    } else {
        None
    }
}

/// Transform a `try` pattern - dispatches to specific pattern handlers
fn try_transform_try(tokens: &[TokenTree]) -> Option<(TokenStream, usize)> {
    if tokens.len() < 2 {
        return None;
    }

    // Check what follows `try`
    match &tokens[1] {
        // try { } ... - basic try block
        TokenTree::Group(g) if g.delimiter() == Delimiter::Brace => {
            try_transform_try_block(tokens)
        }
        // try while/for/any/all/when - special patterns
        TokenTree::Ident(next_ident) => {
            let next_str = next_ident.to_string();
            match next_str.as_str() {
                "while" => try_transform_try_while(tokens),
                "for" => try_transform_try_for(tokens),
                "any" => try_transform_try_any(tokens),
                "all" => try_transform_try_all(tokens),
                "when" => try_transform_try_when(tokens),
                // Note: "try catch" at expression start is handled by collect_handlers
                _ => None,
            }
        }
        // try -> Type { } ... - try with explicit return type
        TokenTree::Punct(p) if p.as_char() == '-' => {
            // Check for -> (minus followed by greater-than)
            if tokens.len() > 2 {
                if let TokenTree::Punct(p2) = &tokens[2] {
                    if p2.as_char() == '>' {
                        return try_transform_try_block_with_type(tokens);
                    }
                }
            }
            None
        }
        _ => None,
    }
}

/// Transform `async try { } ...` patterns
fn try_transform_async_try(tokens: &[TokenTree]) -> Option<(TokenStream, usize)> {
    // tokens[0] = "async", tokens[1] = "try"
    if tokens.len() < 3 {
        return None;
    }

    let try_body = match &tokens[2] {
        TokenTree::Group(g) if g.delimiter() == Delimiter::Brace => {
            transform_nested(g.stream())
        }
        _ => return None,
    };

    let mut i = 3;
    let (handler_tokens, _, _, consumed) = collect_handlers(&tokens[i..]);
    i += consumed;

    let transformed = if handler_tokens.is_empty() {
        // No handlers - inline the async try expansion to preserve type inference
        // Use or_else instead of map_err to avoid type inference issues
        quote! {
            {
                ::handle_this::__async_try_block!({ #try_body })
                    .await
                    .or_else(|__e| ::core::result::Result::Err(
                        ::handle_this::__wrap_frame(__e, file!(), line!(), column!())
                    ))?
            }
        }
    } else {
        let handler_stream: TokenStream = handler_tokens.into_iter().collect();
        // Use ? to propagate - allows catch bodies with ? to propagate new errors
        quote! {
            ::handle_this_macros::__async_try_proc!({ #try_body } #handler_stream)?
        }
    };

    Some((transformed, i))
}

/// Transform `try -> Type { } catch/throw/inspect/finally/else ...`
/// Handles explicit return type annotation which forces direct mode.
fn try_transform_try_block_with_type(tokens: &[TokenTree]) -> Option<(TokenStream, usize)> {
    // tokens[0] = "try", tokens[1] = "-", tokens[2] = ">"
    if tokens.len() < 5 {
        return None;
    }

    let mut i = 3; // Start after "try ->"

    // Collect type tokens until we hit a brace
    let mut type_tokens = Vec::new();
    while i < tokens.len() {
        if let TokenTree::Group(g) = &tokens[i] {
            if g.delimiter() == Delimiter::Brace {
                break;
            }
        }
        type_tokens.push(tokens[i].clone());
        i += 1;
    }

    if i >= tokens.len() || type_tokens.is_empty() {
        return None;
    }

    let try_body = match &tokens[i] {
        TokenTree::Group(g) if g.delimiter() == Delimiter::Brace => {
            transform_nested(g.stream())
        }
        _ => return None,
    };
    i += 1;

    let (handler_tokens, _has_catch_all, _has_control_flow_catch, consumed) = collect_handlers(&tokens[i..]);
    i += consumed;

    let type_stream: TokenStream = type_tokens.into_iter().collect();
    let handler_stream: TokenStream = handler_tokens.into_iter().collect();

    // Direct mode with explicit type always returns the direct value (not Result).
    // The __sync_try_proc handles the match internally and extracts the value.
    // No .unwrap() or ? needed.
    let transformed = quote! {
        ::handle_this_macros::__sync_try_proc!(-> #type_stream { #try_body } #handler_stream)
    };

    Some((transformed, i))
}

/// Transform `try { } catch/throw/inspect/finally ...`
/// Also transforms bare `try { }` without handlers.
fn try_transform_try_block(tokens: &[TokenTree]) -> Option<(TokenStream, usize)> {
    if tokens.len() < 2 {
        return None;
    }

    // Get span from the try keyword for accurate line/column
    let try_span = tokens[0].span();

    let try_body = match &tokens[1] {
        TokenTree::Group(g) if g.delimiter() == Delimiter::Brace => {
            transform_nested(g.stream())
        }
        _ => return None,
    };

    let mut i = 2;
    let (handler_tokens, has_catch_all, has_control_flow_catch, consumed) = collect_handlers(&tokens[i..]);
    i += consumed;

    // Check if body contains control flow from nested try handlers
    let body_has_control_flow = contains_control_flow(&try_body);

    let transformed = if handler_tokens.is_empty() {
        // No handlers
        if body_has_control_flow {
            // Body contains control flow (from nested try with break/continue in catch)
            // Don't use ? because the body returns a direct value, not Result
            quote_spanned! {try_span=>
                ::handle_this_macros::__sync_try_proc!({ #try_body })
            }
        } else {
            // Standard case - wrap in try block and propagate with ?
            quote_spanned! {try_span=>
                ::handle_this_macros::__sync_try_proc!({ #try_body })?
            }
        }
    } else {
        let handler_stream: TokenStream = handler_tokens.into_iter().collect();
        // Control flow can come from either:
        // 1. Handlers at this level (has_control_flow_catch)
        // 2. Nested try handlers in the body (body_has_control_flow)
        if has_control_flow_catch || body_has_control_flow {
            // Control flow present - returns direct value (signal mode)
            // Even typed catches use unreachable fallback to maintain type compatibility
            quote_spanned! {try_span=>
                ::handle_this_macros::__sync_try_proc!({ #try_body } #handler_stream)
            }
        } else if has_catch_all {
            // Safe catch-all: handler guarantees all errors are handled, result is always Ok
            quote_spanned! {try_span=>
                ::handle_this_macros::__sync_try_proc!({ #try_body } #handler_stream).unwrap()
            }
        } else {
            // Use ? to propagate errors - this allows catch bodies with ? to propagate new errors
            quote_spanned! {try_span=>
                ::handle_this_macros::__sync_try_proc!({ #try_body } #handler_stream)?
            }
        }
    };

    Some((transformed, i))
}

/// Transform `try while COND { body } ...`
fn try_transform_try_while(tokens: &[TokenTree]) -> Option<(TokenStream, usize)> {
    if tokens.len() < 4 {
        return None;
    }

    let mut i = 2; // Start after "try while"
    let mut condition_tokens = Vec::new();

    // Collect condition tokens until we hit a brace
    while i < tokens.len() {
        if let TokenTree::Group(g) = &tokens[i] {
            if g.delimiter() == Delimiter::Brace {
                break;
            }
        }
        condition_tokens.push(tokens[i].clone());
        i += 1;
    }

    if i >= tokens.len() {
        return None;
    }

    let body = match &tokens[i] {
        TokenTree::Group(g) if g.delimiter() == Delimiter::Brace => {
            transform_nested(g.stream())
        }
        _ => return None,
    };
    i += 1;

    let (handler_tokens, has_catch_all, has_control_flow_catch, consumed) = collect_handlers(&tokens[i..]);
    i += consumed;

    // Check if body contains control flow from nested try handlers
    let body_has_control_flow = contains_control_flow(&body);

    let condition: TokenStream = condition_tokens.into_iter().collect();
    let handlers: TokenStream = handler_tokens.into_iter().collect();

    let transformed = if has_control_flow_catch || body_has_control_flow {
        // Signal mode: generates a complete match expression, no wrapper needed
        quote! {
            ::handle_this_macros::__try_while_proc!(#condition { #body } #handlers)
        }
    } else if has_catch_all {
        quote! {
            ::handle_this_macros::__try_while_proc!(#condition { #body } #handlers).unwrap()
        }
    } else {
        quote! {
            ::handle_this_macros::__try_while_proc!(#condition { #body } #handlers)?
        }
    };

    Some((transformed, i))
}

/// Iterator pattern kind for unified handling.
enum IterPatternKind {
    For,
    Any,
    All,
}

/// Unified transform for `try for/any/all PAT in ITER { body } ...`
fn try_transform_iter_pattern(tokens: &[TokenTree], kind: IterPatternKind) -> Option<(TokenStream, usize)> {
    if tokens.len() < 6 {
        return None;
    }

    let mut i = 2; // Start after "try for/any/all"

    // Collect pattern tokens until we hit "in"
    let mut pattern_tokens = Vec::new();
    while i < tokens.len() {
        if let TokenTree::Ident(ident) = &tokens[i] {
            if *ident == "in" {
                i += 1; // Skip "in"
                break;
            }
        }
        pattern_tokens.push(tokens[i].clone());
        i += 1;
    }

    // Collect iterator tokens until we hit a brace
    let mut iter_tokens = Vec::new();
    while i < tokens.len() {
        if let TokenTree::Group(g) = &tokens[i] {
            if g.delimiter() == Delimiter::Brace {
                break;
            }
        }
        iter_tokens.push(tokens[i].clone());
        i += 1;
    }

    if i >= tokens.len() {
        return None;
    }

    let body = match &tokens[i] {
        TokenTree::Group(g) if g.delimiter() == Delimiter::Brace => {
            transform_nested(g.stream())
        }
        _ => return None,
    };
    i += 1;

    let pattern: TokenStream = pattern_tokens.into_iter().collect();
    let iter: TokenStream = iter_tokens.into_iter().collect();

    // Check if body contains control flow from nested try handlers
    let body_has_control_flow = contains_control_flow(&body);

    let transformed = match kind {
        IterPatternKind::All => {
            // try all supports handlers too
            let (handler_tokens, has_catch_all, has_control_flow_catch, consumed) = collect_handlers(&tokens[i..]);
            i += consumed;
            let handlers: TokenStream = handler_tokens.into_iter().collect();

            if has_control_flow_catch || body_has_control_flow {
                // Signal mode: generates a complete match expression, no wrapper needed
                quote! {
                    ::handle_this_macros::__try_all_proc!(#pattern in #iter { #body } #handlers)
                }
            } else if has_catch_all {
                quote! {
                    ::handle_this_macros::__try_all_proc!(#pattern in #iter { #body } #handlers).unwrap()
                }
            } else {
                quote! {
                    ::handle_this_macros::__try_all_proc!(#pattern in #iter { #body } #handlers)?
                }
            }
        }
        _ => {
            // for/any support handlers
            let (handler_tokens, has_catch_all, has_control_flow_catch, consumed) = collect_handlers(&tokens[i..]);
            i += consumed;
            let handlers: TokenStream = handler_tokens.into_iter().collect();

            let proc_macro = match kind {
                IterPatternKind::For => quote! { ::handle_this_macros::__try_for_proc },
                IterPatternKind::Any => quote! { ::handle_this_macros::__try_any_proc },
                IterPatternKind::All => unreachable!(),
            };

            if has_control_flow_catch || body_has_control_flow {
                // Signal mode: generates a complete match expression, no wrapper needed
                quote! { #proc_macro!(#pattern in #iter { #body } #handlers) }
            } else if has_catch_all {
                quote! { #proc_macro!(#pattern in #iter { #body } #handlers).unwrap() }
            } else {
                quote! { #proc_macro!(#pattern in #iter { #body } #handlers)? }
            }
        }
    };

    Some((transformed, i))
}

/// Transform `try for PAT in ITER { body } ...`
fn try_transform_try_for(tokens: &[TokenTree]) -> Option<(TokenStream, usize)> {
    try_transform_iter_pattern(tokens, IterPatternKind::For)
}

/// Transform `try any PAT in ITER { body } ...`
fn try_transform_try_any(tokens: &[TokenTree]) -> Option<(TokenStream, usize)> {
    try_transform_iter_pattern(tokens, IterPatternKind::Any)
}

/// Transform `try all PAT in ITER { body } ...`
fn try_transform_try_all(tokens: &[TokenTree]) -> Option<(TokenStream, usize)> {
    try_transform_iter_pattern(tokens, IterPatternKind::All)
}

/// Check if an identifier is a handler keyword that ends the current handler.
#[inline]
fn is_handler_keyword(s: &str) -> bool {
    matches!(s, "catch" | "throw" | "inspect" | "finally" | "with" | "try" | "else")
}

/// Collect tokens for a handler until hitting the body brace group.
/// Returns the number of tokens consumed.
fn collect_handler_body(
    tokens: &[TokenTree],
    start: usize,
    handler_tokens: &mut Vec<TokenTree>,
) -> usize {
    let mut i = start;
    while i < tokens.len() {
        if let TokenTree::Ident(next) = &tokens[i] {
            if is_handler_keyword(&next.to_string()) {
                break;
            }
        }
        match &tokens[i] {
            TokenTree::Group(g) if g.delimiter() == Delimiter::Brace => {
                let inner = transform_nested(g.stream());
                let mut new_group = proc_macro2::Group::new(g.delimiter(), inner);
                new_group.set_span(g.span());
                handler_tokens.push(TokenTree::Group(new_group));
                i += 1;
                break;
            }
            _ => handler_tokens.push(tokens[i].clone()),
        }
        i += 1;
    }
    i - start
}

/// Find the first brace group body in a slice of tokens and check for control flow.
/// Returns (has_control_flow, has_question_mark, is_catch_all)
fn analyze_handler_body(tokens: &[TokenTree]) -> (bool, bool, bool) {
    // Scan forward to find the first brace group (the handler body)
    for token in tokens {
        if let TokenTree::Group(g) = token {
            if g.delimiter() == Delimiter::Brace {
                let has_cf = contains_control_flow(&g.stream());
                let has_qm = contains_question_mark(&g.stream());
                // It's a catch-all if no type path precedes the body
                // For simplicity, we check if there's control flow - that's our main concern
                return (has_cf, has_qm, !has_qm);
            }
        }
    }
    (false, false, false)
}

/// Collect handler tokens (catch/throw/inspect/finally/with) and detect catch-all
/// Returns (handler_tokens, has_catch_all, has_control_flow_catch, tokens_consumed)
fn collect_handlers(tokens: &[TokenTree]) -> (Vec<TokenTree>, bool, bool, usize) {
    let mut handler_tokens = Vec::new();
    let mut has_catch_all = false;
    let mut has_control_flow_catch = false;
    let mut i = 0;

    while i < tokens.len() {
        if let TokenTree::Ident(ident) = &tokens[i] {
            let ident_str = ident.to_string();
            match ident_str.as_str() {
                "catch" => {
                    handler_tokens.push(tokens[i].clone());
                    i += 1;

                    // Check if this is a catch-all and analyze the body for control flow
                    // Use contains_control_flow (not is_control_flow) because body may have
                    // multiple statements before control flow, e.g. `catch { log(e); continue }`
                    if i < tokens.len() {
                        match &tokens[i] {
                            TokenTree::Group(g) if g.delimiter() == Delimiter::Brace => {
                                // Direct catch-all: `catch { body }`
                                if contains_control_flow(&g.stream()) {
                                    has_control_flow_catch = true;
                                    has_catch_all = true;
                                } else if !contains_question_mark(&g.stream()) {
                                    has_catch_all = true;
                                }
                            }
                            TokenTree::Ident(next_ident) => {
                                let next_str = next_ident.to_string();
                                if next_str == "any" || next_str == "all" {
                                    // `catch any/all Type...` - analyze body
                                    let (has_cf, _has_qm, _) = analyze_handler_body(&tokens[i..]);
                                    if has_cf {
                                        has_control_flow_catch = true;
                                    }
                                    // any/all catches are not catch-all
                                } else if i + 1 < tokens.len() {
                                    // Check for `catch binding { body }` or `catch Type... { body }`
                                    let (has_cf, has_qm, _) = analyze_handler_body(&tokens[i..]);
                                    if has_cf {
                                        has_control_flow_catch = true;
                                    }
                                    // Check if it's a catch-all (simple binding followed by brace)
                                    if let TokenTree::Group(g) = &tokens[i + 1] {
                                        if g.delimiter() == Delimiter::Brace {
                                            // Simple binding catch-all: `catch e { body }`
                                            if !has_qm {
                                                has_catch_all = true;
                                            }
                                        }
                                    }
                                }
                            }
                            _ => {
                                // Other cases (e.g., underscore binding) - analyze body
                                let (has_cf, _, _) = analyze_handler_body(&tokens[i..]);
                                if has_cf {
                                    has_control_flow_catch = true;
                                }
                            }
                        }
                    }

                    i += collect_handler_body(tokens, i, &mut handler_tokens);
                }
                "throw" => {
                    handler_tokens.push(tokens[i].clone());
                    i += 1;
                    // Check throw body for control flow
                    let (has_cf, _, _) = analyze_handler_body(&tokens[i..]);
                    if has_cf {
                        has_control_flow_catch = true;
                    }
                    i += collect_handler_body(tokens, i, &mut handler_tokens);
                }
                "inspect" | "finally" => {
                    handler_tokens.push(tokens[i].clone());
                    i += 1;
                    i += collect_handler_body(tokens, i, &mut handler_tokens);
                }
                // `else` is syntactic sugar for catch-all in direct mode (try -> T)
                "else" => {
                    handler_tokens.push(tokens[i].clone());
                    i += 1;

                    // else always sets has_catch_all
                    if i < tokens.len() {
                        if let TokenTree::Group(g) = &tokens[i] {
                            if g.delimiter() == Delimiter::Brace {
                                if contains_control_flow(&g.stream()) {
                                    has_control_flow_catch = true;
                                }
                                if !contains_question_mark(&g.stream()) {
                                    has_catch_all = true;
                                }
                            }
                        }
                    }

                    i += collect_handler_body(tokens, i, &mut handler_tokens);
                }
                "with" => {
                    handler_tokens.push(tokens[i].clone());
                    i += 1;

                    // `with` doesn't have a brace body - collect until handler keyword or semicolon
                    while i < tokens.len() {
                        if let TokenTree::Ident(next) = &tokens[i] {
                            if is_handler_keyword(&next.to_string()) {
                                break;
                            }
                        }
                        if let TokenTree::Punct(p) = &tokens[i] {
                            if p.as_char() == ';' {
                                break;
                            }
                        }
                        handler_tokens.push(tokens[i].clone());
                        i += 1;
                    }
                }
                // `try catch` is a result-returning catch handler
                // NOTE: `try catch` does NOT set has_catch_all because it can return Err
                "try" => {
                    // Check if next token is `catch`
                    if i + 1 < tokens.len() {
                        if let TokenTree::Ident(next) = &tokens[i + 1] {
                            if *next == "catch" {
                                handler_tokens.push(tokens[i].clone());
                                i += 1;
                                handler_tokens.push(tokens[i].clone());
                                i += 1;
                                i += collect_handler_body(tokens, i, &mut handler_tokens);
                                continue;
                            }
                        }
                    }
                    // Not `try catch`, break
                    break;
                }
                _ => break,
            }
        } else if let TokenTree::Punct(p) = &tokens[i] {
            if p.as_char() == ',' {
                handler_tokens.push(tokens[i].clone());
                i += 1;
            } else {
                break;
            }
        } else {
            break;
        }
    }

    (handler_tokens, has_catch_all, has_control_flow_catch, i)
}

/// Transform `scope "name", rest...` or `scope "name", { kv }, rest...` pattern
fn try_transform_scope(tokens: &[TokenTree]) -> Option<(TokenStream, usize)> {
    // tokens[0] = "scope", tokens[1] = string literal, tokens[2] = comma, tokens[3..] = rest or { kv }
    if tokens.len() < 4 {
        return None;
    }

    // Get span from the scope keyword for accurate line/column
    let scope_span = tokens[0].span();

    // tokens[1] should be the scope name (string literal)
    let name = tokens[1].clone();

    // tokens[2] should be a comma
    if let TokenTree::Punct(p) = &tokens[2] {
        if p.as_char() != ',' {
            return None;
        }
    } else {
        return None;
    }

    let mut i = 3;
    let mut kv_chain = TokenStream::new();

    // Check if tokens[3] is a brace group (kv data)
    if let Some(TokenTree::Group(g)) = tokens.get(3) {
        if g.delimiter() == Delimiter::Brace {
            // Parse kv pairs from the braces
            kv_chain = parse_kv_chain(g.stream());
            i = 4;

            // Skip optional comma after kv braces
            if let Some(TokenTree::Punct(p)) = tokens.get(4) {
                if p.as_char() == ',' {
                    i = 5;
                }
            }
        }
    }

    // Try to match a nested pattern (try block, require, etc.) and track consumed tokens
    let (transformed_rest, rest_consumed) = if let Some((inner_transformed, inner_consumed)) = try_transform_pattern(&tokens[i..]) {
        // A nested pattern was matched - use its result and consumed count
        (inner_transformed, inner_consumed)
    } else if let Some(TokenTree::Group(g)) = tokens.get(i) {
        // A single brace group - transform its contents
        let inner = transform_nested(g.stream());
        let mut new_group = proc_macro2::Group::new(g.delimiter(), inner);
        new_group.set_span(g.span());
        (quote! { #new_group }, 1)
    } else {
        // Fallback: collect tokens until we hit a statement terminator or end
        let mut rest_tokens = Vec::new();
        let mut consumed = 0;
        let mut j = i;
        while j < tokens.len() {
            // Stop at semicolon (statement end) or closing brace context
            if let TokenTree::Punct(p) = &tokens[j] {
                if p.as_char() == ';' {
                    break;
                }
            }
            rest_tokens.push(tokens[j].clone());
            consumed += 1;
            j += 1;
        }
        if rest_tokens.is_empty() {
            return None;
        }
        let rest: TokenStream = rest_tokens.into_iter().collect();
        (transform_nested(rest), consumed)
    };

    // Generate line!/column! with the scope's span so they resolve to the right location
    let line_call = quote_spanned! {scope_span=> line!() };
    let col_call = quote_spanned! {scope_span=> column!() };

    // Generate scope code directly (like patterns/scope.rs does)
    let transformed = quote! {
        {
            let __scope_result: ::core::result::Result<_, ::handle_this::Handled> = (|| {
                ::core::result::Result::Ok({ #transformed_rest })
            })();
            match __scope_result {
                ::core::result::Result::Ok(__v) => ::core::result::Result::Ok(__v),
                ::core::result::Result::Err(__e) => ::core::result::Result::Err(
                    __e.frame(file!(), #line_call, #col_call).ctx(#name) #kv_chain
                ),
            }
        }?
    };

    Some((transformed, i + rest_consumed))
}

/// Parse kv pairs from a brace group and generate `.kv(...)` chain
fn parse_kv_chain(tokens: TokenStream) -> TokenStream {
    let tokens_vec: Vec<TokenTree> = tokens.into_iter().collect();
    let mut chain = TokenStream::new();
    let mut i = 0;

    while i < tokens_vec.len() {
        // Expect: ident : expr [, ...]
        let key = match &tokens_vec.get(i) {
            Some(TokenTree::Ident(id)) => id.clone(),
            _ => break,
        };
        i += 1;

        // Expect colon
        match tokens_vec.get(i) {
            Some(TokenTree::Punct(p)) if p.as_char() == ':' => i += 1,
            _ => break,
        }

        // Collect value tokens until comma or end
        let mut value_tokens = Vec::new();
        while i < tokens_vec.len() {
            if let TokenTree::Punct(p) = &tokens_vec[i] {
                if p.as_char() == ',' {
                    i += 1; // Skip comma
                    break;
                }
            }
            value_tokens.push(tokens_vec[i].clone());
            i += 1;
        }
        let value: TokenStream = value_tokens.into_iter().collect();

        chain.extend(quote! { .kv(stringify!(#key), #value) });
    }

    chain
}

/// Transform `require COND else "msg", rest...` pattern
fn try_transform_require(tokens: &[TokenTree]) -> Option<(TokenStream, usize)> {
    // tokens[0] = "require", then COND tokens, then "else", then msg, then ",", then rest
    if tokens.len() < 5 {
        return None;
    }

    // Find "else" keyword to split condition from message
    let mut else_idx = None;
    for (idx, token) in tokens[1..].iter().enumerate() {
        if let TokenTree::Ident(ident) = token {
            if *ident == "else" {
                else_idx = Some(idx + 1); // +1 because we started from tokens[1]
                break;
            }
        }
    }
    let else_idx = else_idx?;

    // Find the comma that separates require clause from rest (after "else msg")
    let mut comma_idx = None;
    for (idx, token) in tokens[else_idx + 1..].iter().enumerate() {
        if let TokenTree::Punct(p) = token {
            if p.as_char() == ',' {
                comma_idx = Some(else_idx + 1 + idx + 1); // adjusted for offset
                break;
            }
        }
    }
    let comma_idx = comma_idx?;

    // Collect condition tokens (between "require" and "else")
    let condition: TokenStream = tokens[1..else_idx].iter().cloned().collect();

    // Collect message tokens (between "else" and comma)
    let message: TokenStream = tokens[else_idx + 1..comma_idx].iter().cloned().collect();

    // Transform nested patterns in the rest (after the comma)
    let rest: TokenStream = tokens[comma_idx + 1..].iter().cloned().collect();
    let transformed_rest = transform_nested(rest);

    // Generate require code directly (like patterns/require.rs does)
    // Wrap in block for #[allow] since attributes on if expressions aren't stable
    let transformed = quote! {
        {
            #[allow(unreachable_code)]
            if !(#condition) {
                ::core::result::Result::Err(
                    ::handle_this::Handled::msg(#message).frame(file!(), line!(), column!())
                )?
            } else {
                #transformed_rest
            }
        }
    };

    Some((transformed, tokens.len()))
}

/// Transform `try when COND { body } [else when COND { body }]* [else { body }] handlers...` pattern
fn try_transform_try_when(tokens: &[TokenTree]) -> Option<(TokenStream, usize)> {
    // tokens[0] = "try", tokens[1] = "when"
    if tokens.len() < 4 {
        return None;
    }

    let mut i = 2; // Start after "try when"

    // Collect condition tokens until we hit a brace
    let mut condition_tokens = Vec::new();
    while i < tokens.len() {
        if let TokenTree::Group(g) = &tokens[i] {
            if g.delimiter() == Delimiter::Brace {
                break;
            }
        }
        condition_tokens.push(tokens[i].clone());
        i += 1;
    }

    if i >= tokens.len() {
        return None;
    }

    // Parse the body
    let body = match &tokens[i] {
        TokenTree::Group(g) if g.delimiter() == Delimiter::Brace => {
            transform_nested(g.stream())
        }
        _ => return None,
    };
    i += 1;

    let condition: TokenStream = condition_tokens.into_iter().collect();

    // Build the if expression
    let mut if_chain = quote! {
        if #condition {
            ::handle_this::__try_block!(#body)
        }
    };

    // Track whether we have an explicit else clause
    let mut has_explicit_else = false;

    // Parse else-when and else branches
    while i < tokens.len() {
        if let TokenTree::Ident(ident) = &tokens[i] {
            if *ident == "else" {
                i += 1;
                if i >= tokens.len() {
                    break;
                }

                // Check for "else when" or just "else"
                if let TokenTree::Ident(next) = &tokens[i] {
                    if *next == "when" {
                        i += 1;
                        // Collect condition
                        let mut ew_cond = Vec::new();
                        while i < tokens.len() {
                            if let TokenTree::Group(g) = &tokens[i] {
                                if g.delimiter() == Delimiter::Brace {
                                    break;
                                }
                            }
                            ew_cond.push(tokens[i].clone());
                            i += 1;
                        }
                        let ew_condition: TokenStream = ew_cond.into_iter().collect();

                        if i >= tokens.len() {
                            break;
                        }
                        let ew_body = match &tokens[i] {
                            TokenTree::Group(g) if g.delimiter() == Delimiter::Brace => {
                                transform_nested(g.stream())
                            }
                            _ => break,
                        };
                        i += 1;

                        if_chain = quote! {
                            #if_chain else if #ew_condition {
                                ::handle_this::__try_block!(#ew_body)
                            }
                        };
                        continue;
                    }
                }

                // Just "else { body }"
                if let TokenTree::Group(g) = &tokens[i] {
                    if g.delimiter() == Delimiter::Brace {
                        let else_body = transform_nested(g.stream());
                        i += 1;
                        if_chain = quote! {
                            #if_chain else {
                                ::handle_this::__try_block!(#else_body)
                            }
                        };
                        has_explicit_else = true;
                        break;
                    }
                }
            }
        }
        break;
    }

    // Collect any remaining handlers
    let (handler_tokens, has_catch_all, _, consumed) = collect_handlers(&tokens[i..]);
    i += consumed;

    let transformed = if handler_tokens.is_empty() {
        if has_explicit_else {
            // Has explicit else - just wrap and propagate
            quote! {
                { #if_chain }
                    .map_err(|__e| ::handle_this::__wrap_frame(__e, file!(), line!(), column!()))?
            }
        } else {
            // No explicit else - add default else returning unit, then propagate with ?
            quote! {
                {
                    #if_chain else {
                        ::core::result::Result::Ok(())
                    }
                }
                .map_err(|__e| ::handle_this::__wrap_frame(__e, file!(), line!(), column!()))?
            }
        }
    } else {
        let handler_stream: TokenStream = handler_tokens.into_iter().collect();
        if has_explicit_else {
            if has_catch_all {
                quote! {
                    ::handle_this_macros::__sync_try_proc!({
                        #if_chain?
                    } #handler_stream).unwrap()
                }
            } else {
                quote! {
                    ::handle_this_macros::__sync_try_proc!({
                        #if_chain?
                    } #handler_stream)?
                }
            }
        } else if has_catch_all {
            quote! {
                ::handle_this_macros::__sync_try_proc!({
                    #if_chain else {
                        ::core::result::Result::Ok(())
                    }?
                } #handler_stream).unwrap()
            }
        } else {
            quote! {
                ::handle_this_macros::__sync_try_proc!({
                    #if_chain else {
                        ::core::result::Result::Ok(())
                    }?
                } #handler_stream)?
            }
        }
    };

    Some((transformed, i))
}

