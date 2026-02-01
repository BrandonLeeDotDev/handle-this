//! Detection utilities for nested pattern analysis.
//!
//! Provides functions to detect control flow, question marks, and skip over
//! nested try patterns without transforming them.

use proc_macro2::{TokenStream, TokenTree, Delimiter};

/// Check if a token stream starts with a control flow statement (continue/break).
/// Note: Use `contains_control_flow` for checking if control flow appears anywhere.
#[allow(dead_code)]
pub fn is_control_flow(tokens: &TokenStream) -> bool {
    let tokens: Vec<_> = tokens.clone().into_iter().collect();
    if tokens.is_empty() {
        return false;
    }

    match &tokens[0] {
        TokenTree::Ident(ident) => {
            let s = ident.to_string();
            s == "continue" || s == "break"
        }
        _ => false,
    }
}

/// Check if a token stream contains the `?` operator (error propagation).
/// This is used to determine if a handler body might produce new errors.
///
/// IMPORTANT: This skips over:
/// - Nested `try` blocks because those handle their own errors
/// - Proc macro calls like `__sync_try_proc!` because those are transformed nested patterns
///
/// A `?` inside `try { x? } catch { }` doesn't propagate to the outer handler.
pub fn contains_question_mark(tokens: &TokenStream) -> bool {
    contains_question_mark_impl(tokens, false)
}

/// Implementation that tracks whether we're inside a nested try pattern.
fn contains_question_mark_impl(tokens: &TokenStream, in_nested_try: bool) -> bool {
    let tokens_vec: Vec<TokenTree> = tokens.clone().into_iter().collect();
    let mut i = 0;

    while i < tokens_vec.len() {
        let token = &tokens_vec[i];

        // Check for `try` keyword starting a nested pattern
        if let TokenTree::Ident(ident) = token {
            if ident == "try" {
                // Skip the entire nested try pattern
                // A nested try with handlers captures its own errors
                if let Some(skip) = skip_nested_try_pattern(&tokens_vec[i..]) {
                    i += skip;
                    continue;
                }
            }
        }

        // Check for proc macro calls from transformed nested patterns
        // Pattern: `:: handle_this :: handle_this_macros :: __*_proc ! ( ... )`
        // These handle their own errors, so skip them entirely
        if let Some(skip) = skip_transformed_try_pattern(&tokens_vec[i..]) {
            i += skip;
            continue;
        }

        match token {
            TokenTree::Punct(p) if p.as_char() == '?' => {
                if !in_nested_try {
                    return true;
                }
            }
            TokenTree::Group(g) => {
                if contains_question_mark_impl(&g.stream(), in_nested_try) {
                    return true;
                }
            }
            _ => {}
        }
        i += 1;
    }
    false
}

/// Skip over a transformed try pattern (proc macro call), returning tokens to skip.
/// Pattern: `:: handle_this :: handle_this_macros :: __*_proc ! { ... }` or with any delimiter
/// Optionally followed by method chain like `.unwrap()` or `?`
/// Returns None if this isn't a transformed pattern.
fn skip_transformed_try_pattern(tokens: &[TokenTree]) -> Option<usize> {
    // Look for `:: handle_this :: handle_this_macros :: __*_proc ! { ... }`
    if tokens.len() < 11 {
        return None;
    }

    // Check for `::`
    let is_path_sep = |i: usize| -> bool {
        if i + 1 >= tokens.len() {
            return false;
        }
        matches!((&tokens[i], &tokens[i + 1]),
            (TokenTree::Punct(p1), TokenTree::Punct(p2))
            if p1.as_char() == ':' && p2.as_char() == ':')
    };

    // Start with `::`
    if !is_path_sep(0) {
        return None;
    }

    // Check for `handle_this`
    if let TokenTree::Ident(ident) = &tokens[2] {
        if ident != "handle_this" {
            return None;
        }
    } else {
        return None;
    }

    // `::` after handle_this
    if !is_path_sep(3) {
        return None;
    }

    // Check for `handle_this_macros`
    if let TokenTree::Ident(ident) = &tokens[5] {
        if ident != "handle_this_macros" {
            return None;
        }
    } else {
        return None;
    }

    // `::` after handle_this_macros
    if !is_path_sep(6) {
        return None;
    }

    // Check for `__*_proc` ident
    if let TokenTree::Ident(ident) = &tokens[8] {
        let name = ident.to_string();
        if !name.starts_with("__") || !name.ends_with("_proc") {
            return None;
        }
    } else {
        return None;
    }

    // Check for `!`
    if let TokenTree::Punct(p) = &tokens[9] {
        if p.as_char() != '!' {
            return None;
        }
    } else {
        return None;
    }

    // Skip the macro call arguments (can be parentheses, braces, or brackets)
    if !matches!(&tokens[10], TokenTree::Group(_)) {
        return None;
    }

    // 11 tokens so far: `:: handle_this :: handle_this_macros :: __*_proc ! ( ... )`
    let mut skip = 11;

    // Optionally skip `.unwrap()` or `.unwrap_or_else(...)` etc.
    while skip + 1 < tokens.len() {
        if let TokenTree::Punct(p) = &tokens[skip] {
            if p.as_char() == '.' {
                // Skip the `.method(...)` call
                skip += 1; // `.`
                if skip >= tokens.len() {
                    break;
                }
                if let TokenTree::Ident(_) = &tokens[skip] {
                    skip += 1; // method name
                }
                if skip < tokens.len() {
                    if let TokenTree::Group(g) = &tokens[skip] {
                        if g.delimiter() == Delimiter::Parenthesis {
                            skip += 1; // `(...)`
                        }
                    }
                }
                continue;
            }
        }
        break;
    }

    Some(skip)
}

/// Skip over a nested try pattern, returning the number of tokens to skip.
/// Returns None if this isn't a complete try pattern with handlers.
pub fn skip_nested_try_pattern(tokens: &[TokenTree]) -> Option<usize> {
    if tokens.is_empty() {
        return None;
    }

    // Must start with `try`
    if let TokenTree::Ident(ident) = &tokens[0] {
        if ident != "try" {
            return None;
        }
    } else {
        return None;
    }

    let mut i = 1;

    // Skip over `->` type annotation if present
    if i + 1 < tokens.len() {
        if let TokenTree::Punct(p) = &tokens[i] {
            if p.as_char() == '-' {
                if let TokenTree::Punct(p2) = &tokens[i + 1] {
                    if p2.as_char() == '>' {
                        i += 2;
                        // Skip type tokens until brace
                        while i < tokens.len() {
                            if let TokenTree::Group(g) = &tokens[i] {
                                if g.delimiter() == Delimiter::Brace {
                                    break;
                                }
                            }
                            i += 1;
                        }
                    }
                }
            }
        }
    }

    // Skip loop keywords (for, while, any, all)
    if i < tokens.len() {
        if let TokenTree::Ident(kw) = &tokens[i] {
            let kw_str = kw.to_string();
            if matches!(kw_str.as_str(), "for" | "while" | "any" | "all") {
                i += 1;
                // Skip until brace
                while i < tokens.len() {
                    if let TokenTree::Group(g) = &tokens[i] {
                        if g.delimiter() == Delimiter::Brace {
                            break;
                        }
                    }
                    i += 1;
                }
            }
        }
    }

    // Need a brace body
    if i >= tokens.len() {
        return None;
    }
    if let TokenTree::Group(g) = &tokens[i] {
        if g.delimiter() != Delimiter::Brace {
            return None;
        }
    } else {
        return None;
    }
    i += 1;

    // Must have at least one handler (catch/throw/inspect/else) to be a complete pattern
    // that handles its own errors
    let mut has_handler = false;
    while i < tokens.len() {
        if let TokenTree::Ident(ident) = &tokens[i] {
            let ident_str = ident.to_string();
            match ident_str.as_str() {
                "catch" | "throw" | "inspect" | "else" => {
                    has_handler = true;
                    i += 1;
                    // Skip until next handler keyword or end
                    while i < tokens.len() {
                        if let TokenTree::Ident(next) = &tokens[i] {
                            let next_str = next.to_string();
                            if matches!(next_str.as_str(), "catch" | "throw" | "inspect" | "else" | "finally" | "with") {
                                break;
                            }
                        }
                        // Skip brace groups
                        if let TokenTree::Group(g) = &tokens[i] {
                            if g.delimiter() == Delimiter::Brace {
                                i += 1;
                                break;
                            }
                        }
                        i += 1;
                    }
                }
                "finally" | "with" => {
                    i += 1;
                    // Skip until next handler or end
                    while i < tokens.len() {
                        if let TokenTree::Ident(next) = &tokens[i] {
                            let next_str = next.to_string();
                            if matches!(next_str.as_str(), "catch" | "throw" | "inspect" | "else" | "finally" | "with") {
                                break;
                            }
                        }
                        if let TokenTree::Group(g) = &tokens[i] {
                            if g.delimiter() == Delimiter::Brace {
                                i += 1;
                                break;
                            }
                        }
                        i += 1;
                    }
                }
                _ => break,
            }
        } else {
            break;
        }
    }

    if has_handler {
        Some(i)
    } else {
        None
    }
}

/// Recursively check if a token stream contains control flow (break/continue/return).
/// This scans the entire token stream including nested groups.
/// Note: `return` is included because it also cannot escape from closures.
pub fn contains_control_flow(tokens: &TokenStream) -> bool {
    for token in tokens.clone() {
        match token {
            TokenTree::Ident(ident) => {
                let s = ident.to_string();
                if s == "break" || s == "continue" || s == "return" {
                    return true;
                }
            }
            TokenTree::Group(g) => {
                if contains_control_flow(&g.stream()) {
                    return true;
                }
            }
            _ => {}
        }
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;
    use quote::quote;

    #[test]
    fn test_skip_nested_try_basic() {
        let tokens: Vec<TokenTree> = quote!(try { x? } catch { 1 }).into_iter().collect();
        let result = skip_nested_try_pattern(&tokens);
        assert!(result.is_some(), "Should recognize try-catch as nested pattern");
        assert_eq!(result.unwrap(), 4, "Should skip 4 tokens: try, body, catch, body");
    }

    #[test]
    fn test_question_mark_in_nested_try_catch() {
        // This is the exact case that's failing: nested try with ? inside catch body
        let tokens = quote!(try { io_ok()? } catch { 1 });
        let result = contains_question_mark(&tokens);
        assert!(!result, "? inside nested try should not be detected");
    }

    #[test]
    fn test_question_mark_bare() {
        let tokens = quote!(foo()?);
        assert!(contains_question_mark(&tokens), "Bare ? should be detected");
    }

    #[test]
    fn test_question_mark_in_try_without_handler() {
        // Try block without catch - errors propagate, so ? is visible
        let tokens = quote!(try { x? });
        assert!(contains_question_mark(&tokens), "? in try without handler should be detected");
    }

    #[test]
    fn test_question_mark_nested_in_group() {
        // Simulating what happens when body is wrapped in a Group (like from parse_braced_body)
        use proc_macro2::Group;

        let inner = quote!(try { io_ok()? } catch { 1 });
        // Wrap in a brace group like parse_braced_body would
        let wrapped = TokenStream::from(TokenTree::Group(Group::new(
            proc_macro2::Delimiter::None,  // No visible delimiters
            inner,
        )));
        let result = contains_question_mark(&wrapped);
        assert!(!result, "? inside nested try-catch in group should not be detected");
    }

    #[test]
    fn test_debug_token_structure() {
        // Debug: print what the tokens look like
        let tokens = quote!(try { io_ok()? } catch { 1 });
        let tokens_vec: Vec<TokenTree> = tokens.clone().into_iter().collect();
        eprintln!("Token count: {}", tokens_vec.len());
        for (i, t) in tokens_vec.iter().enumerate() {
            match t {
                TokenTree::Ident(id) => eprintln!("[{}] Ident: {}", i, id),
                TokenTree::Group(g) => eprintln!("[{}] Group({:?}): {}", i, g.delimiter(), g.stream()),
                TokenTree::Punct(p) => eprintln!("[{}] Punct: {}", i, p.as_char()),
                TokenTree::Literal(l) => eprintln!("[{}] Literal: {}", i, l),
            }
        }
    }

    #[test]
    fn test_skip_transformed_try_pattern_with_braces() {
        // Simulate transformed pattern: ::handle_this::handle_this_macros::__sync_try_proc!{ ... }
        let tokens = quote!(::handle_this::handle_this_macros::__sync_try_proc!{ try { x? } catch { 1 } });
        let tokens_vec: Vec<TokenTree> = tokens.clone().into_iter().collect();

        let skip = skip_transformed_try_pattern(&tokens_vec);
        assert!(skip.is_some(), "Should recognize transformed pattern with braces");

        // Should skip the entire pattern
        let skip_count = skip.unwrap();
        assert_eq!(skip_count, 11, "Should skip 11 tokens: :: handle_this :: handle_this_macros :: __sync_try_proc ! {{ ... }}");

        // contains_question_mark should skip it
        let has_q = contains_question_mark(&tokens);
        assert!(!has_q, "? inside transformed pattern should be skipped");
    }

    #[test]
    fn test_skip_transformed_try_pattern_no_unwrap() {
        // Pattern without .unwrap() - used when control flow is present
        let tokens = quote!(::handle_this::handle_this_macros::__sync_try_proc!{ continue });
        let has_q = contains_question_mark(&tokens);
        assert!(!has_q, "Transformed pattern without unwrap should be skipped");
    }
}
