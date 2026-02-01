//! Loop Signal Mode - Control Flow as Data
//!
//! # Problem Statement
//!
//! Rust's type system prevents control flow (`continue`, `break`) from escaping closures.
//! Previously, when handlers contained control flow, we used "direct mode" which:
//! - Avoided closures entirely
//! - Required `unreachable!()` for unmatched typed errors
//!
//! This created a **known limitation**: typed catches with control flow could not
//! propagate unmatched errors to outer handlers.
//!
//! # Solution: Signal Mode
//!
//! Instead of actual control flow, handlers return a **signal value** that gets
//! translated to real control flow at the macro expansion site (outside any closure).
//!
//! ```text
//! User writes:           catch io::Error { continue }
//! Handler returns:       Ok(LoopSignal::Continue)
//! Expansion site does:   match result { Ok(LoopSignal::Continue) => continue, ... }
//! ```
//!
//! # Signal Type
//!
//! ```rust,ignore
//! pub enum __LoopSignal<T> {
//!     Value(T),    // Normal completion with value
//!     Continue,    // Execute `continue` on target loop
//!     Break,       // Execute `break` on target loop
//! }
//! ```
//!
//! # Return Type in Signal Mode
//!
//! Handlers return `Result<LoopSignal<T>, Handled>`:
//! - `Ok(LoopSignal::Value(v))` - Handler produced a value
//! - `Ok(LoopSignal::Continue)` - Handler wants to `continue`
//! - `Ok(LoopSignal::Break)` - Handler wants to `break`
//! - `Err(e)` - Unmatched error, propagate to outer handler
//!
//! # Safety Considerations
//!
//! 1. **Type Safety**: `LoopSignal<T>` preserves the value type `T`
//! 2. **Exhaustive Matching**: All signal variants must be handled
//! 3. **Error Propagation**: Unmatched errors always propagate (no silent drops)
//! 4. **No Panics**: Signal mode never uses `unreachable!()` for unmatched errors

use proc_macro2::{TokenStream, TokenTree, Delimiter};
use quote::quote;

/// Generate the signal enum reference path.
pub fn signal_type() -> TokenStream {
    quote! { ::handle_this::__LoopSignal }
}

// ============================================================
// Control Flow Transformation
// ============================================================

/// Transform control flow statements in a handler body to signal returns.
///
/// This is the core transformation that enables signal mode:
/// - `continue` → `return __signal_continue()`
/// - `break` → `return __signal_break()`
///
/// # Correctness Guarantees
///
/// 1. Only transforms top-level control flow (not inside nested loops/closures)
/// 2. Preserves all other tokens exactly
/// 3. Handles edge cases: `continue;`, `continue` as last expression, etc.
///
/// # Parameters
///
/// - `tokens`: The handler body tokens to transform
///
/// # Returns
///
/// Transformed token stream with control flow replaced by signal returns.
pub fn transform_control_flow(tokens: TokenStream) -> TokenStream {
    let tokens_vec: Vec<TokenTree> = tokens.into_iter().collect();
    transform_cf_tokens(&tokens_vec, 0, false)
}

/// Transform control flow using non-generic control signals.
///
/// - `continue` → `return __ctrl_continue()`
/// - `break` → `return __ctrl_break()`
///
/// Used when all handlers are pure control flow to avoid type inference issues.
pub fn transform_control_flow_nongeneric(tokens: TokenStream) -> TokenStream {
    let tokens_vec: Vec<TokenTree> = tokens.into_iter().collect();
    transform_cf_tokens(&tokens_vec, 0, true)
}

/// Internal transformation with nesting depth tracking.
///
/// `depth` tracks nesting inside loops/closures where control flow should NOT be transformed.
/// `nongeneric` uses __ctrl_* functions instead of __signal_* functions.
fn transform_cf_tokens(tokens: &[TokenTree], depth: usize, nongeneric: bool) -> TokenStream {
    let mut result = Vec::new();
    let mut i = 0;

    while i < tokens.len() {
        match &tokens[i] {
            TokenTree::Ident(ident) => {
                let ident_str = ident.to_string();

                // Check for loop/closure constructs that create new control flow targets
                if matches!(ident_str.as_str(), "for" | "while" | "loop") {
                    // Found a nested loop - increase depth for its body
                    result.push(tokens[i].clone());
                    i += 1;

                    // Collect tokens until we find the body brace
                    while i < tokens.len() {
                        if let TokenTree::Group(g) = &tokens[i] {
                            if g.delimiter() == Delimiter::Brace {
                                // Transform body with increased depth
                                let inner = transform_cf_tokens(
                                    &g.stream().into_iter().collect::<Vec<_>>(),
                                    depth + 1,
                                    nongeneric,
                                );
                                let mut new_group = proc_macro2::Group::new(Delimiter::Brace, inner);
                                new_group.set_span(g.span());
                                result.push(TokenTree::Group(new_group));
                                i += 1;
                                break;
                            }
                        }
                        result.push(tokens[i].clone());
                        i += 1;
                    }
                    continue;
                }

                // Check for closure: `|...| { }` or `move |...| { }`
                if ident_str == "move" && i + 1 < tokens.len() {
                    if let TokenTree::Punct(p) = &tokens[i + 1] {
                        if p.as_char() == '|' {
                            // `move |...| body` - closure, increase depth
                            result.push(tokens[i].clone()); // move
                            i += 1;
                            i = transform_closure_body(tokens, i, &mut result, depth, nongeneric);
                            continue;
                        }
                    }
                }

                // Check for control flow at depth 0 (our handler body level)
                if depth == 0 && (ident_str == "continue" || ident_str == "break") {
                    // Check for labeled control flow: `continue 'label` or `break 'label`
                    if i + 1 < tokens.len() {
                        if let TokenTree::Punct(p) = &tokens[i + 1] {
                            if p.as_char() == '\'' {
                                // Labeled control flow - don't transform, it targets an outer loop
                                // This is intentional: `continue 'outer` should escape
                                result.push(tokens[i].clone());
                                i += 1;
                                continue;
                            }
                        }
                    }

                    // Transform unlabeled control flow to signal using helper functions.
                    // Use nongeneric (__ctrl_*) or generic (__signal_*) versions.
                    let return_stmt = if nongeneric {
                        if ident_str == "continue" {
                            quote! { return ::handle_this::__ctrl_continue() }
                        } else {
                            quote! { return ::handle_this::__ctrl_break() }
                        }
                    } else {
                        if ident_str == "continue" {
                            quote! { return ::handle_this::__signal_continue() }
                        } else {
                            quote! { return ::handle_this::__signal_break() }
                        }
                    };

                    // Add the return statement tokens
                    for tt in return_stmt {
                        result.push(tt);
                    }

                    i += 1;

                    // Skip trailing semicolon if present (it's now part of the return)
                    if i < tokens.len() {
                        if let TokenTree::Punct(p) = &tokens[i] {
                            if p.as_char() == ';' {
                                result.push(tokens[i].clone());
                                i += 1;
                            }
                        }
                    }
                    continue;
                }

                // Regular identifier - pass through
                result.push(tokens[i].clone());
            }

            TokenTree::Punct(p) => {
                // Check for closure start: `|...|`
                if p.as_char() == '|' {
                    i = transform_closure_body(tokens, i, &mut result, depth, nongeneric);
                    continue;
                }
                result.push(tokens[i].clone());
            }

            TokenTree::Group(g) => {
                // Recursively transform group contents at same depth
                let inner_tokens: Vec<TokenTree> = g.stream().into_iter().collect();
                let transformed = transform_cf_tokens(&inner_tokens, depth, nongeneric);
                let mut new_group = proc_macro2::Group::new(g.delimiter(), transformed);
                new_group.set_span(g.span());
                result.push(TokenTree::Group(new_group));
            }

            _ => {
                result.push(tokens[i].clone());
            }
        }
        i += 1;
    }

    result.into_iter().collect()
}

/// Transform a closure body, tracking depth correctly.
///
/// Handles both `|args| body` and `|args| { body }` forms.
fn transform_closure_body(
    tokens: &[TokenTree],
    mut i: usize,
    result: &mut Vec<TokenTree>,
    depth: usize,
    nongeneric: bool,
) -> usize {
    // Copy closure start: `|args|`
    result.push(tokens[i].clone()); // |
    i += 1;

    // Copy args until closing |
    while i < tokens.len() {
        result.push(tokens[i].clone());
        if let TokenTree::Punct(p) = &tokens[i] {
            if p.as_char() == '|' {
                i += 1;
                break;
            }
        }
        i += 1;
    }

    // Now handle the body - could be `{ ... }` or a single expression
    if i < tokens.len() {
        if let TokenTree::Group(g) = &tokens[i] {
            if g.delimiter() == Delimiter::Brace {
                // Braced closure body - transform with increased depth
                let inner = transform_cf_tokens(
                    &g.stream().into_iter().collect::<Vec<_>>(),
                    depth + 1,
                    nongeneric,
                );
                let mut new_group = proc_macro2::Group::new(Delimiter::Brace, inner);
                new_group.set_span(g.span());
                result.push(TokenTree::Group(new_group));
                i += 1;
            }
        }
        // For expression closures like `|x| x + 1`, we don't need to transform
        // as there's no room for control flow statements
    }

    i
}
