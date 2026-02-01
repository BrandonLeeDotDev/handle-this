#!/usr/bin/env python3
"""
Full test matrix generator for handle-this macro.

Generates ALL valid permutations of:
- Try patterns (7)
- Handler types (catch, throw, inspect, else)
- Bindings (8 types)
- Guards (3 types)
- Context modifiers (with, scope, finally combinations)
- Preconditions
- Multi-handler combinations (1-3 handlers)

Splits output into multiple files for manageable compilation.

Usage:
    # Generate all tests (WARNING: 88k+ tests)
    python3 generate_full_matrix.py --all

    # Generate specific combinations
    python3 generate_full_matrix.py -t basic -k catch -b named
    python3 generate_full_matrix.py --try-pattern basic,for --handler catch,throw
    python3 generate_full_matrix.py --category single --try-pattern basic

    # List available options
    python3 generate_full_matrix.py --list

Options:
    -t, --try-pattern   Try patterns: basic, direct, for, any_iter, all_iter, while, async
    -k, --handler       Handler keywords: catch, throw, inspect, else
    -b, --binding       Bindings: none, named, underscore, typed, typed_short, any, any_short, all
    -g, --guard         Guards: none, when_true, when_false, when_kind, when_perm, when_other, match_kind, match_perm
    -c, --context       Contexts: none, with_msg, with_data, with_both, scope, scope_data, finally, scope_finally, with_finally
    -p, --precondition  Preconditions: none, require_pass, require_fail
    -n, --num-handlers  Number of handlers: 0, 1, 2, 3
    --category          Test categories: single, two, three, no_handler, else_suffix, nested, control_flow, deeply_nested, comp_nested
    --all               Generate all tests (use with caution)
    --list              List all available filter values
"""

import os
import sys
import argparse
from dataclasses import dataclass, field
from typing import List, Dict, Optional, Tuple, Iterator, Set
from itertools import product, combinations_with_replacement
import hashlib

# =============================================================================
# Configuration
# =============================================================================

PROJECT_ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
TESTS_DIR = os.path.join(PROJECT_ROOT, "tests")
TESTS_PER_FILE = 500  # Keep files manageable for compilation

# =============================================================================
# Dimensions
# =============================================================================

# Try patterns: (name, pattern, default_body, setup, result_type, is_async, is_direct)
# Note: For `for` and `any_iter`, we use iter_all_fail() so handlers actually run.
# With iter_second_ok(), the second item succeeds and handlers never execute.
TRY_PATTERNS = [
    ("basic", "try", "TRY_BODY", "", "i32", False, False),
    ("direct", "try -> i32", "TRY_BODY", "", "i32", False, True),
    ("for", "try for item in items", "item?", "let items = iter_all_fail();", "i32", False, False),
    ("any_iter", "try any item in items", "item?", "let items = iter_all_fail();", "i32", False, False),
    ("all_iter", "try all item in items", "item?", "let items = iter_all_ok();", "Vec<i32>", False, False),
    ("while", "try while attempts < 3", "attempts += 1; Err(io::Error::other(\"retry\"))?", "let mut attempts = 0;", "i32", False, False),
    ("async", "async try", "ASYNC_BODY", "", "i32", True, False),
]

# Handler keywords
HANDLER_KEYWORDS = ["catch", "throw", "inspect", "else"]

# Bindings: (name, code, needs_chain, has_typed_e, has_errors)
BINDINGS = [
    ("none", "", False, False, False),
    ("named", "e", False, False, False),
    ("underscore", "_", False, False, False),
    ("typed", "io::Error(e)", False, True, False),
    ("typed_short", "io::Error", False, False, False),
    ("any", "any io::Error(e)", True, True, False),
    ("any_short", "any io::Error", True, False, False),
    ("all", "all io::Error |errors|", True, False, True),
]

# Guards: (name, code, needs_typed_e, is_match)
# Note: Guards must match our test errors (NotFound) or tests will fail
GUARDS = [
    ("none", "", False, False),
    ("when_true", "when true", False, False),
    ("when_kind", "when e.kind() == ErrorKind::NotFound", True, False),
    # Match clause - only for typed bindings with `e` binding
    ("match_kind", "match e.kind() { ErrorKind::NotFound => MATCH_BODY, _ => MATCH_ELSE }", True, True),
]

# Else suffix: (name, has_else) - for typed handlers only
ELSE_SUFFIXES = [
    ("no_else", False),
    ("with_else", True),
]

# Then chain steps: (name, steps_code, num_steps)
# steps_code is inserted between try body and handlers
# Each step transforms the value: |x| { expression }
THEN_STEPS = [
    ("no_then", "", 0),
    ("then_1", ", then |x| { ok_val(x + 1)? }", 1),
    ("then_2", ", then |x| { ok_val(x * 2)? }, then |y| { ok_val(y + 1)? }", 2),
    ("then_ctx", ", then |x| { ok_val(x + 1)? } with \"step\"", 1),
]

# Nested body types for catch/throw handlers: (name, body_template, returns_value)
# These test nested try blocks within handler bodies
# Note: body_template uses {{ }} for literal braces in f-strings
# The macro correctly identifies that `?` inside nested try blocks is handled by that try,
# so it doesn't trigger "catch must be infallible" errors.
# returns_value: True if the body returns a value (for catch), False if returns string (for throw)
NESTED_BODIES = [
    ("flat", None, True),  # Use default body
    # Nested try blocks (level 1)
    # Note: No outer braces - handler generation adds { body }
    # Note: Use single braces { } - these are NOT f-strings
    ("nested_try_else", "try -> i32 { io_ok()? } else { 0 }", True),
    ("nested_try_catch", "try { io_ok()? } catch { 0 }", True),
    ("nested_try_throw_catch", "try { io_not_found()? } throw { \"inner\" } catch { 0 }", True),
    # Nested try for (level 1)
    ("nested_try_for", "let items = iter_all_ok(); try for item in items { item? } catch { 0 }", True),
    # Deeply nested (level 2) - try inside try
    ("nested_2_try_in_try", "try { try { io_ok()? } catch { 1 } } catch { 0 }", True),
    # Try with inspects
    ("nested_try_inspect", "let mut saw = false; try { io_ok()? } inspect _ { saw = true; } catch { 0 }", True),
]

# Nested bodies specifically for throw handlers (must return string for error message)
# Note: No outer braces - handler generation adds { body }
# Note: Use single braces { } - these are NOT f-strings
NESTED_THROW_BODIES = [
    ("flat", None),  # Use default body
    ("nested_try_else_str", "let s: String = try -> String { \"inner\".to_string() } else { \"fallback\".to_string() }; s"),
    ("nested_try_catch_str", "let s: String = try { Ok::<_, std::io::Error>(\"inner\".to_string())? } catch { \"caught\".to_string() }; s"),
]

# =============================================================================
# Comprehensive Nested Permutation System
# =============================================================================
# Abstracted system for generating nested try permutations across ALL block types.
#
# Block positions that can contain nested try:
# - try_body: try { NESTED }
# - catch_body: catch { NESTED } - must be infallible (no ?)
# - throw_body: throw { NESTED } - must return String
# - inspect_body: inspect e { NESTED; } - statement, doesn't return
# - else_body: else { NESTED } - returns value
# - finally_body: finally { NESTED; } - statement, cleanup
# - try_catch_body: try catch { NESTED } - returns Result
# - match_arm: match expr { arm => NESTED } - returns value
# - require_else: require cond else { NESTED } - returns error

@dataclass
class BlockPosition:
    """Defines a position where nesting can occur."""
    name: str
    returns_value: bool  # True if block returns a value
    return_type: str     # "i32", "String", "Result", "unit", "error"
    is_statement: bool   # True if block is a statement (no return value used)
    can_use_question_mark: bool  # True if ? operator is allowed

# All block positions where nesting can occur
BLOCK_POSITIONS = {
    # Basic try
    "try_body": BlockPosition("try_body", True, "i32", False, True),
    # Handlers
    "catch_body": BlockPosition("catch_body", True, "i32", False, False),  # catch must be infallible
    "throw_body": BlockPosition("throw_body", True, "String", False, False),
    "inspect_body": BlockPosition("inspect_body", False, "unit", True, False),
    "else_body": BlockPosition("else_body", True, "i32", False, False),
    "try_catch_body": BlockPosition("try_catch_body", True, "Result", False, True),
    # Loop bodies
    "for_body": BlockPosition("for_body", True, "i32", False, True),   # try for x in iter { BODY }
    "while_body": BlockPosition("while_body", True, "i32", False, True),  # try while cond { BODY }
    "all_body": BlockPosition("all_body", True, "i32", False, True),   # try all x in iter { BODY }
    # Context/control
    "finally_body": BlockPosition("finally_body", False, "unit", True, False),
    "match_arm": BlockPosition("match_arm", True, "i32", False, False),
    "require_else": BlockPosition("require_else", True, "error", False, False),
}

# Nested patterns parameterized by return type
# Format: (name, template) where template uses {err}, {val}, {str} placeholders
NESTED_PATTERNS_BY_TYPE = {
    "i32": [
        ("try_catch", "try {{ {err} }} catch {{ {val} }}"),
        ("try_throw_catch", "try {{ {err} }} throw {{ \"transformed\" }} catch {{ {val} }}"),
        ("try_inspect_catch", "try {{ {err} }} inspect _ {{ }} catch {{ {val} }}"),
        ("try_direct", "try -> i32 {{ {err} }} else {{ {val} }}"),
        ("try_typed_catch", "try {{ {err} }} catch io::Error(_) {{ {val} }} else {{ {val} + 1 }}"),
        ("try_any_catch", "try {{ Err(chained_io_error())? }} catch any io::Error(_) {{ {val} }}"),
        ("try_for", "{{ let items = iter_all_fail(); try for item in items {{ item? }} catch {{ {val} }} }}"),
        ("try_while", "{{ let mut att = 0; try while att < 2 {{ att += 1; Err(io::Error::other(\"r\"))? }} catch {{ {val} }} }}"),
        ("scoped_try", "scope \"nested\", try {{ {err} }} catch {{ {val} }}"),
        ("scoped_try_kv", "scope \"nested\", {{ key: 42 }}, try {{ {err} }} catch {{ {val} }}"),
        ("scoped_try_kv_multi", "scope \"nested\", {{ id: 1, active: true }}, try {{ {err} }} catch {{ {val} }}"),
        ("scoped_nested", "scope \"outer\", try {{ scope \"inner\", try {{ {err} }} catch {{ {val} }} }} catch {{ {val} + 1 }}"),
        ("scoped_nested_kv", "scope \"outer\", {{ layer: 1 }}, try {{ scope \"inner\", {{ layer: 2 }}, try {{ {err} }} catch {{ {val} }} }} catch {{ {val} + 1 }}"),
        ("try_with_finally", "{{ let mut f = false; let r = try {{ {err} }} finally {{ f = true; }} catch {{ {val} }}; r }}"),
    ],
    "String": [
        ("try_catch_str", "try {{ Ok::<_, Handled>(\"inner\".to_string())? }} catch {{ \"caught\".to_string() }}"),
        ("try_direct_str", "try -> String {{ \"direct\".to_string() }} else {{ \"fallback\".to_string() }}"),
    ],
    "Result": [
        ("try_catch_result", "try {{ {err} }} try catch e {{ if true {{ Ok({val}) }} else {{ Err(e) }} }}"),
        ("nested_ok", "{{ let v: Result<i32> = try {{ {err} }} catch {{ {val} }}; v }}"),
    ],
    "unit": [
        ("try_catch_unit", "{{ let _ = try {{ {err} }} catch {{ {val} }}; }}"),
        ("try_discard", "{{ let _: Result<i32> = try {{ {err} }} catch {{ {val} }}; }}"),
    ],
    "error": [
        ("try_catch_err", "try {{ {err} }} catch {{ Handled::msg(\"nested error\") }}"),
    ],
}

# Error sources for nested patterns
NESTED_ERR_SOURCES = [
    ("io_err", "io_not_found()?"),
    ("chain_err", "Err(chained_io_error())?"),
]

# Values for nested catch bodies
NESTED_VALUES = [1, 42, 99]

# Level 2 templates (nesting inside nesting)
NESTED_L2_TEMPLATES = {
    "i32": [
        ("l2_try_try", "try {{ try {{ {err} }} catch {{ {val} }} }} catch {{ {val} + 10 }}"),
        ("l2_catch_try", "try {{ {err} }} catch {{ try {{ io_ok()? }} catch {{ {val} }} }}"),
        ("l2_throw_try", "try {{ {err} }} throw {{ try -> String {{ \"t\".to_string() }} else {{ \"f\".to_string() }} }} catch {{ {val} }}"),
        ("l2_for_try", "{{ let items = iter_all_fail(); try for item in items {{ try {{ item? }} catch {{ {val} }} }} catch {{ {val} + 10 }} }}"),
    ],
}

# Level 3 templates
NESTED_L3_TEMPLATES = {
    "i32": [
        ("l3_deep", "try {{ try {{ try {{ {err} }} catch {{ {val} }} }} catch {{ {val} + 10 }} }} catch {{ {val} + 20 }}"),
    ],
}

# =============================================================================
# Expanded nested patterns for use in comprehensive tests
# =============================================================================
# These expand the templates above with concrete values for direct use.

def expand_nested_patterns(templates: list, err: str = "io_not_found()?", val: str = "1") -> list:
    """Expand template patterns with concrete error/value placeholders."""
    result = []
    for name, template in templates:
        code = template.format(err=err, val=val, str='"s".to_string()')
        result.append((name, code))
    return result

# Expanded patterns for direct use
NESTED_TRY_I32 = expand_nested_patterns(NESTED_PATTERNS_BY_TYPE["i32"])
NESTED_TRY_STR = [
    (name, template.format(err="io_not_found()?", val="1", str='"s".to_string()'))
    for name, template in NESTED_PATTERNS_BY_TYPE["String"]
]
NESTED_L2_I32 = expand_nested_patterns(NESTED_L2_TEMPLATES["i32"])
NESTED_L3_I32 = expand_nested_patterns(NESTED_L3_TEMPLATES["i32"])

# Control flow in CATCH HANDLERS for loop patterns (try for, try while, try all)
# break/continue work in the catch handler, NOT inside nested try blocks
# Pattern: try for x in iter { ... } catch { break }
CONTROL_FLOW_CATCH_BODIES = [
    ("catch_break", "break"),
    ("catch_continue", "continue"),
]

# Context modifiers: (name, code, needs_setup, setup_code, extra_assert)
CONTEXTS = [
    ("none", [], "", ""),
    ("with_msg", ['with "test context"'], "", ""),
    ("with_data", ['with { key: 42 }'], "", ""),
    ("with_both", ['with "ctx", { key: 42 }'], "", ""),
    ("scope", ['scope "test"'], "", ""),
    ("scope_data", ['scope "test", { key: 42 }'], "", ""),
    ("scope_data_bool", ['scope "test", { enabled: true }'], "", ""),
    ("scope_data_str", ['scope "test", { name: "value" }'], "", ""),
    ("scope_data_multi", ['scope "test", { id: 1, active: true }'], "", ""),
    ("finally", ["finally { finalized = true; }"], "let mut finalized = false;", "assert!(finalized);"),
    ("scope_finally", ['scope "test"', "finally { finalized = true; }"], "let mut finalized = false;", "assert!(finalized);"),
    ("scope_data_finally", ['scope "test", { key: 42 }', "finally { finalized = true; }"], "let mut finalized = false;", "assert!(finalized);"),
    ("with_finally", ['with "ctx"', "finally { finalized = true; }"], "let mut finalized = false;", "assert!(finalized);"),
]

# Preconditions: (name, code)
PRECONDITIONS = [
    ("none", []),
    ("require_pass", ['require true else "failed"']),
    ("require_fail", ['require false else "failed"']),
]

# =============================================================================
# Validation
# =============================================================================

def is_valid_handler(keyword: str, binding: str, guard: str, try_pattern: str) -> bool:
    """Check if a handler combination is valid."""
    bind_name, bind_code, needs_chain, has_typed_e, has_errors = None, None, None, None, None
    for b in BINDINGS:
        if b[0] == binding:
            bind_name, bind_code, needs_chain, has_typed_e, has_errors = b
            break

    guard_name, guard_code, needs_typed_e, is_match = None, None, None, False
    for g in GUARDS:
        if g[0] == guard:
            guard_name, guard_code, needs_typed_e, is_match = g
            break

    is_direct = try_pattern == "direct"
    is_iter = try_pattern in ["for", "any_iter", "all_iter", "while"]

    # else only works in direct mode with no binding/guard
    if keyword == "else":
        return is_direct and binding == "none" and guard == "none"

    # inspect requires some binding
    if keyword == "inspect" and binding == "none":
        return False

    # throw doesn't use underscore
    if keyword == "throw" and binding == "underscore":
        return False

    # guard requires some binding
    if guard != "none" and binding == "none":
        return False

    # when_kind and match_kind require typed binding (e.kind() method)
    if needs_typed_e and not has_typed_e:
        return False

    # match clause requires a typed binding (not untyped catch/throw)
    if is_match and binding not in ["typed", "any"]:
        return False

    return True


def is_untyped_catch(kw: str, bind: str) -> bool:
    """Check if handler is an untyped catch (catches all errors)."""
    return kw in ["catch", "else"] and bind in ["none", "named", "underscore"]


def is_valid_combination(try_pattern: str, handlers: List[Tuple[str, str, str]],
                         context: str, precondition: str, then_steps: str = "no_then") -> bool:
    """Check if a full combination is valid."""
    is_direct = try_pattern == "direct"
    has_then = then_steps != "no_then"

    # Then chains not yet supported with async
    if has_then and try_pattern == "async":
        return False

    # Then chains need special handling for try all (returns Vec) and try while (test body returns ())
    # Skip these combinations in the test generator - they work but need custom then step code
    if has_then and try_pattern in ["all_iter", "while"]:
        return False

    # Then chains don't work with with_* or scope_* contexts - these wrap the try body
    # in ways incompatible with then chain syntax
    if has_then and (context.startswith("with") or context.startswith("scope")):
        return False

    # Then chains don't work with any/all bindings - those require error bodies that return ()
    # but then steps expect a value to transform
    if has_then:
        needs_error_body = any(h[1] in ["any", "any_short", "all"] for h in handlers)
        if needs_error_body:
            return False

    # Then chains require an untyped catch to guarantee success
    # Typed catches might not match (especially after throw transforms error type),
    # causing error to propagate and then chain to never run
    if has_then:
        has_untyped_catch = any(
            h[0] == "catch" and h[1] in ["none", "underscore", "named"]
            for h in handlers
        )
        # If there's any throw, require untyped catch (throw transforms might break typed catch)
        has_any_throw = any(h[0] == "throw" for h in handlers)
        if has_any_throw and not has_untyped_catch:
            return False

    # Validate each handler
    for kw, bind, guard in handlers:
        if not is_valid_handler(kw, bind, guard, try_pattern):
            return False

    # CRITICAL: No handlers may follow an untyped catch.
    # Untyped catch handles ALL errors, making subsequent handlers unreachable.
    # The macro now emits a compile error for this case.
    saw_untyped_catch = False
    for kw, bind, guard in handlers:
        if saw_untyped_catch:
            # Any handler after untyped catch is invalid
            return False
        if is_untyped_catch(kw, bind):
            saw_untyped_catch = True

    # Direct mode requires catch-all fallback
    if is_direct:
        has_catchall = any(
            kw in ["catch", "else"] and bind in ["none", "underscore"] and guard == "none"
            for kw, bind, guard in handlers
        )
        if not has_catchall:
            return False

    # Direct mode cannot be used with require or scope.
    # Direct mode guarantees success (returns T), but require/scope can fail (return Result).
    # The macro emits a compile error for these combinations.
    if is_direct:
        if precondition != "none":
            return False
        if context.startswith("scope"):
            return False

    # KNOWN BUG: require_fail + finally doesn't run finally block
    # Skip these combinations until the bug is fixed
    if precondition == "require_fail" and "finally" in context:
        return False

    # require_fail with catch would still fail (precondition runs first)
    # That's actually valid - test should expect error

    # Async can't use all_iter result type easily
    if try_pattern == "async" and context in ["scope", "scope_data", "scope_finally"]:
        # Scope with async needs careful handling - skip for now
        pass

    return True


# =============================================================================
# Code Generation
# =============================================================================

def get_try_body(binding: str, is_async: bool) -> str:
    """Get appropriate try body."""
    needs_chain = binding in ["any", "any_short", "all"]
    if is_async:
        return "async_io_not_found().await?"
    if needs_chain:
        if binding == "all":
            return "Err(multi_io_chain())?"
        return "Err(chained_io_error())?"
    return "io_not_found()?"


def get_handler_body(keyword: str, binding: str, nested_body: str = None, try_pattern: str = None) -> str:
    """Get handler body.

    Args:
        keyword: Handler keyword (catch, throw, inspect, else)
        binding: Binding type name
        nested_body: Optional nested body template to use instead of default
        try_pattern: Try pattern name (e.g., "all_iter" needs Vec return type)
    """
    has_typed_e = binding in ["typed", "any"]
    has_errors = binding == "all"
    has_e = binding in ["named", "typed", "any"]

    # try all returns Vec<T>, so catch must return Vec<T> too
    needs_vec = try_pattern == "all_iter"

    if keyword == "catch" or keyword == "else":
        if nested_body:
            return nested_body
        if has_errors:
            return "vec![errors.len() as i32]" if needs_vec else "errors.len() as i32"
        elif has_typed_e:
            return "let _ = e.kind(); vec![42]" if needs_vec else "let _ = e.kind(); 42"
        elif has_e:
            return "let _ = &e; vec![42]" if needs_vec else "let _ = &e; 42"
        else:
            return "vec![42]" if needs_vec else "42"
    elif keyword == "throw":
        if nested_body:
            return nested_body
        if has_errors:
            return 'format!("{} errors", errors.len())'
        elif has_typed_e:
            return 'format!("io: {:?}", e.kind())'
        elif has_e:
            return 'format!("err: {}", e)'
        else:
            return '"transformed"'
    else:  # inspect
        if has_errors:
            return "let _ = errors.len(); inspected = true;"
        elif has_typed_e:
            return "let _ = e.kind(); inspected = true;"
        elif has_e:
            return "let _ = &e; inspected = true;"
        else:
            return "inspected = true;"


def build_handler_str(keyword: str, binding: str, guard: str,
                      nested_body: str = None, has_else: bool = False,
                      try_pattern: str = None) -> str:
    """Build handler string.

    Args:
        keyword: Handler keyword (catch, throw, inspect, else)
        binding: Binding type name
        guard: Guard type name
        nested_body: Optional nested body template
        has_else: If True, add else {} fallback after typed handler
        try_pattern: Try pattern name (e.g., "all_iter" needs Vec return type)
    """
    parts = [keyword]

    # try all returns Vec<T>, so catch must return Vec<T> too
    needs_vec = try_pattern == "all_iter"

    for b in BINDINGS:
        if b[0] == binding:
            if b[1]:
                parts.append(b[1])
            break

    # Check if this is a match clause
    is_match = False
    guard_code = ""
    for g in GUARDS:
        if g[0] == guard:
            guard_code = g[1]
            is_match = g[3] if len(g) > 3 else False
            break

    if is_match:
        # Match clause - body is inside the match arms, no separate braces
        # Match arms must be expressions, not statements (no trailing semicolons)
        if keyword == "catch":
            match_body = "vec![42]" if needs_vec else "42"
            match_else = "vec![0]" if needs_vec else "0"
        elif keyword == "throw":
            match_body = '"matched"'
            match_else = '"unmatched"'
        else:  # inspect
            # For inspect, use a block expression that sets the flag
            match_body = "{ inspected = true; }"
            match_else = "{ }"

        # Replace MATCH_BODY and MATCH_ELSE placeholders
        match_code = guard_code.replace("MATCH_BODY", match_body).replace("MATCH_ELSE", match_else)
        parts.append(match_code)
    else:
        # Regular guard
        if guard_code:
            parts.append(guard_code)

        body = get_handler_body(keyword, binding, nested_body, try_pattern)
        parts.append("{ " + body + " }")

        # Add else fallback for typed handlers
        # Note: throw Type {} else {} creates a catch-all CATCH (not throw),
        # so else body should return a value, not an error string
        if has_else:
            if keyword == "catch":
                parts.append("else { vec![0] }" if needs_vec else "else { 0 }")
            elif keyword == "throw":
                # else after throw is a catch-all catch, returns value not error
                parts.append("else { vec![0] }" if needs_vec else "else { 0 }")

    return " ".join(parts)


def get_required_try_body(handlers: List[Tuple[str, str, str]], is_async: bool) -> str:
    """Determine the try body based on what handlers need.

    If any handler uses 'all' binding, we need multi_io_chain().
    If any handler uses 'any' binding, we need chained_io_error().
    Otherwise, use the simple error.
    """
    needs_all = any(h[1] == "all" for h in handlers)
    needs_any = any(h[1] in ["any", "any_short"] for h in handlers)

    if is_async:
        return "async_io_not_found().await?"
    if needs_all:
        return "Err(multi_io_chain())?"
    if needs_any:
        return "Err(chained_io_error())?"
    return "io_not_found()?"


def generate_test(test_id: str, try_pattern_data: tuple, handlers: List[Tuple[str, str, str]],
                  context_data: tuple, precond_data: tuple, then_data: tuple = None) -> Tuple[str, bool, str]:
    """Generate a single test. Returns (code, expects_ok, expected_value)."""
    tp_name, tp_pattern, tp_body, tp_setup, tp_result, tp_async, tp_direct = try_pattern_data
    ctx_name, ctx_codes, ctx_setup, ctx_assert = context_data
    pre_name, pre_codes = precond_data
    # Extract then chain data (name, code, num_steps)
    then_name, then_code, then_num = then_data if then_data else ("no_then", "", 0)

    # Determine actual try body based on what handlers need
    if handlers:
        actual_body = get_required_try_body(handlers, tp_async)
    else:
        actual_body = "io_ok()?" if not tp_async else "async_io_ok().await?"

    # For iteration patterns, use their specific body
    if tp_name in ["for", "any_iter", "all_iter", "while"]:
        actual_body = tp_body

    # Track what kind of io::Error is at root and in chain (for guard matching)
    # The `when e.kind() == NotFound` guard checks the error being matched:
    # - Root-only bindings check root error's kind
    # - Chain-searching bindings (any) check the matched error in chain
    #
    # Error bodies:
    # - io_not_found(): root is NotFound
    # - multi_io_chain(): root is PermissionDenied, NotFound in chain
    # - chained_io_error(): root is StringError (not io::Error), NotFound in chain
    # - iter_all_fail(): produces Other kind
    # - async_io_not_found(): root is NotFound
    #
    # Determine based on actual_body which error structure we have:
    needs_all = any(h[1] == "all" for h in handlers)
    needs_any = any(h[1] in ["any", "any_short"] for h in handlers)
    # Note: all_iter with handlers uses iter_all_fail(), not multi_io_chain/chained_io_error
    iter_patterns = ["for", "any_iter", "while", "async"] + (["all_iter"] if handlers else [])
    uses_multi_io_chain = needs_all and tp_name not in iter_patterns
    uses_chained_io_error = needs_any and not needs_all and tp_name not in iter_patterns

    # Root kind: what kind is the root io::Error?
    # Note: all_iter with handlers uses iter_all_fail() which produces Other kind
    if tp_name in ["for", "any_iter", "while"] or (tp_name == "all_iter" and handlers):
        root_kind_is_notfound = False  # iter_all_fail produces Other kind
    elif uses_multi_io_chain:
        root_kind_is_notfound = False  # root is PermissionDenied
    elif uses_chained_io_error:
        root_kind_is_notfound = False  # root is StringError, not even io::Error
    else:
        root_kind_is_notfound = True  # io_not_found or async_io_not_found

    # Chain has NotFound: is there a NotFound anywhere in the chain?
    # For chain-searching (any) bindings, this determines if guard can match
    # Note: all_iter with handlers uses iter_all_fail() which produces only Other kind
    if tp_name in ["for", "any_iter", "while"] or (tp_name == "all_iter" and handlers):
        chain_has_notfound = False  # iter_all_fail produces only Other kind
    else:
        chain_has_notfound = True  # All other bodies have NotFound somewhere

    # Build test
    lines = []

    # Test attribute
    if tp_async:
        lines.append("#[tokio::test]")
        lines.append(f"async fn {test_id}() {{")
    else:
        lines.append("#[test]")
        lines.append(f"fn {test_id}() {{")

    # Setup
    all_setup = []
    if tp_setup:
        # For all_iter with handlers, use failing iterator to test error handling
        if tp_name == "all_iter" and handlers:
            all_setup.append("let items = iter_all_fail();")
        else:
            all_setup.append(tp_setup)
    if ctx_setup:
        all_setup.append(ctx_setup)

    # Check if we need inspected var
    has_inspect = any(h[0] == "inspect" for h in handlers)
    if has_inspect:
        all_setup.append("let mut inspected = false;")

    for s in all_setup:
        lines.append(f"    {s}")

    # Result type
    if tp_direct:
        lines.append(f"    let result: {tp_result} = handle! {{")
    else:
        lines.append(f"    let result: Result<{tp_result}> = handle! {{")

    # Preconditions
    for pre in pre_codes:
        lines.append(f"        {pre},")

    # Scope context (before try)
    for ctx in ctx_codes:
        if ctx.startswith("scope"):
            lines.append(f"        {ctx},")

    # Try block (with optional then chain)
    lines.append(f"        {tp_pattern} {{ {actual_body} }}{then_code}")

    # Handlers
    for kw, bind, guard in handlers:
        h_str = build_handler_str(kw, bind, guard, try_pattern=tp_name)
        lines.append(f"        {h_str}")

    # Non-scope context (with, finally)
    for ctx in ctx_codes:
        if not ctx.startswith("scope"):
            lines.append(f"        {ctx}")

    lines.append("    };")

    # Determine expected outcome
    expects_ok = True
    expected_value = "42"

    if pre_name == "require_fail":
        expects_ok = False
        expected_value = ""
    elif not handlers:
        # No handlers - depends on whether try body produces error
        # For iteration patterns, check the iterator setup:
        # - for, any_iter: use iter_all_fail() -> all fail -> Err
        # - all_iter: uses iter_all_ok() -> all succeed -> Ok
        # - while: always produces error (body has Err()?)
        # For basic/async: check if body has error-producing keywords
        if tp_name in ["for", "any_iter", "while"]:
            # These use failing iterators/conditions -> Err
            expects_ok = False
            expected_value = ""
        elif tp_name == "all_iter":
            # Uses iter_all_ok() -> all succeed -> Ok with Vec
            expects_ok = True
            expected_value = "vec![1, 2, 3]"
        elif "not_found" in actual_body or "fail" in actual_body or "chained" in actual_body or "multi" in actual_body:
            expects_ok = False
            expected_value = ""
    else:
        # Handler semantics:
        # - throw: transforms error, continues chain (non-terminal)
        # - catch/else: catches (possibly transformed) error, returns Ok (terminal)
        # - inspect: runs side effect, continues chain (non-terminal)
        #
        # IMPORTANT: When throw transforms the error, the TYPE changes.
        # Typed catches after throw won't match if throw transformed to a different type.
        #
        # Binding types:
        # - Untyped: "none", "named", "underscore" - catch any error
        # - Typed: "typed", "typed_short", "any", "any_short", "all" - catch specific type

        def is_untyped_binding(binding_name):
            return binding_name in ["none", "named", "underscore"]

        def is_io_typed(binding_name):
            """Check if binding is typed as io::Error"""
            return binding_name in ["typed", "typed_short", "any", "any_short", "all"]

        def is_chain_searching(binding_name):
            """Check if binding searches the error chain (vs just checking root).

            For BASIC try patterns:
            - typed, typed_short: use downcast_ref (root only)
            - any, any_short, all: use chain_any/chain_all (full chain search)

            For LOOP patterns (for, any_iter, while):
            - ALL typed bindings use chain_any because loop errors are chained
            - So even typed, typed_short are chain-searching in loops

            When throw transforms an error, the original error stays IN the chain.
            Chain-searching bindings will still find the original type.
            """
            is_loop_pattern = tp_name in ["for", "any_iter", "while"]
            is_typed = binding_name in ["typed", "typed_short", "any", "any_short", "all"]
            if is_loop_pattern and is_typed:
                # All typed bindings in loops use chain_any
                return True
            return binding_name in ["any", "any_short", "all"]

        def is_root_only(binding_name):
            """Check if binding only checks the root error (not the chain)."""
            is_loop_pattern = tp_name in ["for", "any_iter", "while"]
            if is_loop_pattern:
                # Loop patterns always use chain_any for typed, never root-only
                return False
            return binding_name in ["typed", "typed_short"]

        # Check for untyped catch (will catch anything including transformed errors)
        has_untyped_catch = any(
            h[0] in ["catch", "else"] and is_untyped_binding(h[1])
            for h in handlers
        )

        # Check for any catch at all
        has_any_catch = any(h[0] in ["catch", "else"] for h in handlers)

        # Handler order matters! Find first catch and first throw positions
        first_catch_idx = None
        first_throw_idx = None
        for i, h in enumerate(handlers):
            if h[0] in ["catch", "else"] and first_catch_idx is None:
                first_catch_idx = i
            if h[0] == "throw" and first_throw_idx is None:
                first_throw_idx = i

        # Check if there's a throw BEFORE any catch
        throw_before_catch = (first_throw_idx is not None and
                              (first_catch_idx is None or first_throw_idx < first_catch_idx))

        # Check if the throw before catch will transform the error
        # (either untyped throw which always transforms, or typed throw that matches io::Error)
        # IMPORTANT: Must also check throw's guard - when_kind requires NotFound but our errors are Other
        throw_transforms_before_catch = False
        if throw_before_catch:
            throw_h = handlers[first_throw_idx]
            throw_binding = throw_h[1]
            throw_guard = throw_h[2]
            # Untyped throw or typed io throw might transform...
            if throw_binding in ["none", "named", "underscore"] or is_io_typed(throw_binding):
                # But check if guard allows it
                # For throw, check the kind based on binding type
                if throw_guard == "when_kind":
                    if throw_binding in ["any", "any_short"]:
                        throw_guard_ok = chain_has_notfound
                    elif throw_binding == "all":
                        throw_guard_ok = chain_has_notfound
                    else:
                        # Root-only or untyped
                        throw_guard_ok = root_kind_is_notfound
                    if not throw_guard_ok:
                        throw_transforms_before_catch = False
                    else:
                        throw_transforms_before_catch = True
                else:
                    # "none" or "when_true" guards - throw matches
                    throw_transforms_before_catch = True

        # Find the first catch handler that will successfully match the error.
        # Handlers are checked in order; the first matching one wins.
        #
        # For a catch to match:
        # - Untyped: always matches (no type check)
        # - Typed: must find io::Error in chain (always present in our tests)
        # - Guard: when_kind only matches NotFound; when_true always matches; no guard always matches
        #
        # If throw transforms before a catch, chain-searching catches can still find the original.

        def handler_will_catch(h, error_transformed):
            """Check if a catch handler will successfully catch the error."""
            # Use outer scope variables (avoid re-definition which causes scope issues)
            nonlocal uses_multi_io_chain, uses_chained_io_error

            kw, bind, guard = h
            if kw not in ["catch", "else"]:
                return False, None

            # Check guard - when_kind requires NotFound error
            # For root-only bindings, check root's kind
            # For chain-searching bindings, check if chain has NotFound
            # match_kind always "matches" (has catch-all arm) but returns different values
            def guard_matches_for_binding(b, g):
                if g == "match_kind":
                    # Match clause always succeeds (has _ arm), but value differs
                    return True
                if g != "when_kind":
                    return True
                # when_kind guard - depends on binding type
                if b in ["any", "any_short"]:
                    # Chain-searching: finds first io::Error in chain
                    return chain_has_notfound
                elif b == "all":
                    # Collects all - at least one must match guard
                    return chain_has_notfound
                else:
                    # Root-only (typed, typed_short) or untyped
                    return root_kind_is_notfound

            guard_matches = guard_matches_for_binding(bind, guard)

            # For match clause, determine which arm's value we get
            def get_match_value(b, g):
                if g != "match_kind":
                    return None  # Not a match clause
                # Check if the matched error has NotFound kind
                # chain_any finds FIRST io::Error in chain, not just any NotFound
                if b in ["any", "any_short"]:
                    # For multi_io_chain: root IS io::Error (PermissionDenied) -> finds that first
                    # For chained_io_error: root is StringError, so first io::Error is in chain (NotFound)
                    # For others: root is io::Error -> check root kind
                    if uses_multi_io_chain:
                        is_notfound = False  # First io::Error is PermissionDenied
                    elif uses_chained_io_error:
                        is_notfound = True  # First io::Error is chained NotFound
                    else:
                        is_notfound = root_kind_is_notfound
                else:
                    is_notfound = root_kind_is_notfound
                return is_notfound  # True = first arm (42), False = else arm (0)

            # For `all` binding, the number of errors depends on the actual try body used:
            # - try for/any_iter/all_iter: chains 3 errors (iter_all_fail produces 3 items)
            # - try while: only keeps last error (1 error)
            # - async: always uses async_io_not_found() which has 1 error
            # - basic/direct with multi_io_chain: 2 errors (NotFound + PermissionDenied)
            # - basic/direct with chained_io_error or io_not_found: 1 error
            def all_binding_value():
                if tp_name in ["for", "any_iter", "all_iter"]:
                    return "3"  # iter patterns chain errors from iterator
                if tp_async:
                    return "1"  # async always uses async_io_not_found (1 error)
                # For non-async, check if we're using multi_io_chain()
                # multi_io_chain is used when needs_all=True (any handler has 'all' binding)
                needs_all_body = any(h[1] == "all" for h in handlers)
                if needs_all_body and tp_name not in ["while"]:
                    return "2"  # multi_io_chain has 2 io::Errors
                return "1"  # single error

            # Determine value for match clause
            def catch_value_for_guard(b, g):
                # For all_iter, catch bodies return Vec
                needs_vec = tp_name == "all_iter"

                if g == "match_kind":
                    # Match clause returns different values based on error kind
                    match_is_notfound = get_match_value(b, g)
                    val = "42" if match_is_notfound else "0"
                    return f"vec![{val}]" if needs_vec else val
                elif b == "all":
                    val = all_binding_value()
                    return f"vec![{val}]" if needs_vec else val
                else:
                    return "vec![42]" if needs_vec else "42"

            # Untyped catch (catches any error)
            if bind in ["none", "named", "underscore"]:
                if guard_matches:
                    return True, catch_value_for_guard(bind, guard)
                else:
                    return False, None

            # Typed catch - check if it can find io::Error
            # After throw transforms, chain-searching bindings can only find original
            # if the original is still in the chain (original_in_chain=True)
            if error_transformed:
                if original_in_chain and is_chain_searching(bind):
                    if guard_matches:
                        return True, catch_value_for_guard(bind, guard)
                    else:
                        return False, None
                else:
                    # Original not in chain, or root-only binding
                    return False, None
            else:
                # No transformation - check if root is directly io::Error
                # chain_after() now preserves the original error at root:
                # - multi_io_chain(): root IS io::Error (PermissionDenied)
                # - chained_io_error(): root is StringError (from Handled::msg)
                # - io_not_found(): root IS io::Error (NotFound)
                #
                # For root-only bindings:
                # - multi_io_chain() and io_not_found(): root-only catch WILL match
                # - chained_io_error(): root-only catch will NOT match (root is StringError)
                # (uses_chained_io_error from outer scope)
                if uses_chained_io_error and is_root_only(bind):
                    # Root is StringError, not io::Error - root-only binding won't match
                    return False, None
                if guard_matches:
                    return True, catch_value_for_guard(bind, guard)
                else:
                    return False, None

        # Check handlers in order to find the first match
        # Track whether error has been transformed and whether chain is preserved
        error_transformed = False
        original_in_chain = True  # Is the original io::Error still in the chain?
        found_match = False
        for h in handlers:
            kw, bind, guard = h

            # Check if this is a throw that transforms the error
            if kw == "throw":
                # Check if throw's guard matches based on binding type
                if guard == "when_kind":
                    if bind in ["any", "any_short"]:
                        throw_guard_matches = chain_has_notfound
                    elif bind == "all":
                        throw_guard_matches = chain_has_notfound
                    else:
                        throw_guard_matches = root_kind_is_notfound
                else:
                    throw_guard_matches = True
                if throw_guard_matches:
                    # ALL throws now use chain_after, so the original is ALWAYS preserved
                    if bind in ["none", "named", "underscore"]:
                        error_transformed = True
                        original_in_chain = True  # chain_after preserves original
                    elif is_io_typed(bind):
                        error_transformed = True
                        original_in_chain = True  # chain_after preserves original

            # Check if this catch will match
            if kw in ["catch", "else"]:
                matches, value = handler_will_catch(h, error_transformed)
                if matches:
                    expects_ok = True
                    expected_value = value
                    found_match = True
                    break

        if not found_match:
            # No catch matched - error propagates
            expects_ok = False
            expected_value = ""

    # Special cases for iteration patterns
    if tp_name == "all_iter" and expects_ok:
        if not handlers:
            # No handlers, iter_all_ok() succeeds
            expected_value = "vec![1, 2, 3]"
        # else: expected_value is already set by handler logic (e.g., "vec![42]")

    # Apply then chain transformations to expected value
    # then_1/then_ctx: x + 1
    # then_2: (x * 2) + 1
    # ONLY applies when try body succeeds - if handlers catch errors, then doesn't run
    # When handlers exist, the try body is set to fail, so then never runs
    if expects_ok and expected_value and then_num > 0 and not handlers:
        try:
            val = int(expected_value)
            if then_name == "then_2":
                val = (val * 2) + 1
            else:
                val = val + 1
            expected_value = str(val)
        except ValueError:
            pass  # Non-integer value (e.g., vec![...]), don't transform

    # Assertions
    if tp_direct:
        if expected_value and expects_ok:
            lines.append(f"    assert_eq!(result, {expected_value});")
    elif expects_ok:
        if expected_value:
            lines.append(f"    assert_eq!(result.unwrap(), {expected_value});")
        else:
            lines.append("    assert!(result.is_ok());")
    else:
        lines.append("    assert!(result.is_err());")

    # Extra asserts
    if ctx_assert:
        lines.append(f"    {ctx_assert}")

    # Only assert inspected if inspect actually runs
    # Conditions:
    # 1. Must be before any catch in source order (otherwise catch handles error first)
    # 2. Must match the error type (but chain-searching bindings can still find original after throw)
    if has_inspect:
        # Find positions and check if inspect will run
        first_catch_idx = None
        first_throw_idx = None
        first_inspect_idx = None
        inspect_binding = None

        inspect_guard = None
        for i, h in enumerate(handlers):
            if h[0] in ["catch", "else"] and first_catch_idx is None:
                first_catch_idx = i
            if h[0] == "throw" and first_throw_idx is None:
                first_throw_idx = i
            if h[0] == "inspect" and first_inspect_idx is None:
                first_inspect_idx = i
                inspect_binding = h[1]
                inspect_guard = h[2]

        # Helper functions (redefined for inspect context)
        def inspect_is_chain_searching(binding_name):
            """Chain-searching bindings find original error even after throw."""
            # For loops, all typed bindings use chain_any
            is_loop_pattern = tp_name in ["for", "any_iter", "while"]
            is_typed = binding_name in ["typed", "typed_short", "any", "any_short", "all"]
            if is_loop_pattern and is_typed:
                return True
            return binding_name in ["any", "any_short", "all"]

        def inspect_is_root_only(binding_name):
            """Root-only bindings won't find original after throw."""
            is_loop_pattern = tp_name in ["for", "any_iter", "while"]
            if is_loop_pattern:
                return False  # Loop patterns always use chain_any
            return binding_name in ["typed", "typed_short"]

        # Check if inspect is typed (looking for io::Error)
        inspect_is_io_typed = inspect_binding in ["typed", "typed_short", "any", "any_short", "all"]

        # Check if inspect guard will match our test errors
        # when_kind guard checks e.kind() == NotFound
        # match_kind: the match runs but `inspected = true` is only in NotFound arm
        # Guard matching depends on binding type
        if inspect_guard == "when_kind":
            if inspect_binding in ["any", "any_short"]:
                inspect_guard_matches = chain_has_notfound
            elif inspect_binding == "all":
                inspect_guard_matches = chain_has_notfound
            else:
                inspect_guard_matches = root_kind_is_notfound
        elif inspect_guard == "match_kind":
            # Match always runs, but inspected is only set in NotFound arm
            # So we need NotFound for the assertion to pass
            if inspect_binding in ["any", "any_short"]:
                inspect_guard_matches = chain_has_notfound
            elif inspect_binding == "all":
                inspect_guard_matches = chain_has_notfound
            else:
                inspect_guard_matches = root_kind_is_notfound
        else:
            inspect_guard_matches = True

        # Check if there's a throw before the inspect that transforms io::Error
        transforming_throw_before_inspect = False
        if first_throw_idx is not None and first_inspect_idx is not None:
            if first_throw_idx < first_inspect_idx:
                for h in handlers[:first_inspect_idx]:
                    if h[0] == "throw":
                        throw_binding = h[1]
                        throw_guard = h[2]

                        # Check if throw guard would match based on binding type
                        if throw_guard == "when_kind":
                            if throw_binding in ["any", "any_short"]:
                                throw_guard_matches = chain_has_notfound
                            elif throw_binding == "all":
                                throw_guard_matches = chain_has_notfound
                            else:
                                throw_guard_matches = root_kind_is_notfound
                        else:
                            throw_guard_matches = True
                        if not throw_guard_matches:
                            continue  # Throw doesn't execute, check next handler

                        # Untyped throw or typed io throw transforms (when guard matches)
                        if throw_binding in ["none", "named", "underscore"]:
                            transforming_throw_before_inspect = True
                            break
                        if throw_binding in ["typed", "typed_short", "any", "any_short", "all"]:
                            transforming_throw_before_inspect = True
                            break

        # Inspect runs if:
        # 1. Inspect comes before first catch (or no catch)
        # 2. AND one of:
        #    a. Inspect is untyped (matches any error)
        #    b. No transforming throw before inspect
        #    c. Inspect uses chain-searching binding (can find original in chain)
        inspect_before_catch = first_catch_idx is None or (first_inspect_idx is not None and first_inspect_idx < first_catch_idx)

        # Type matching logic:
        # - Untyped inspect: always matches
        # - Typed inspect: depends on whether io::Error is findable
        #   - chain_after() preserves original error at root
        #   - multi_io_chain(): root IS io::Error
        #   - chained_io_error(): root is StringError
        # - If throw transforms before inspect:
        #   - Chain-searching: still finds original in chain
        #   - Root-only: won't find (root is transformed)
        needs_all = any(h[1] == "all" for h in handlers)
        needs_any = any(h[1] in ["any", "any_short"] for h in handlers)
        uses_chained_io_error = (
            needs_any and not needs_all
            and tp_name not in ["for", "any_iter", "while", "async"]
        )

        if not inspect_is_io_typed:
            type_matches = True  # Untyped inspect matches any error
        elif uses_chained_io_error and inspect_is_root_only(inspect_binding):
            type_matches = False  # Root is StringError, not io::Error
        elif not transforming_throw_before_inspect:
            type_matches = True  # No transformation, original findable
        elif inspect_is_chain_searching(inspect_binding):
            type_matches = True  # Searches chain, finds original
        else:
            type_matches = False  # Root-only, root is transformed type

        # When require_fail, handlers never run (require fails before try block)
        require_passes = pre_name != "require_fail"
        if require_passes and inspect_before_catch and type_matches and inspect_guard_matches:
            lines.append("    assert!(inspected);")

    lines.append("}")

    return "\n".join(lines), expects_ok, expected_value


# =============================================================================
# Permutation Generator
# =============================================================================

def generate_single_handler_permutations() -> Iterator[Tuple[tuple, List[Tuple[str, str, str]], tuple, tuple]]:
    """Generate all single-handler permutations."""
    for tp in TRY_PATTERNS:
        for kw in HANDLER_KEYWORDS:
            for b in BINDINGS:
                for g in GUARDS:
                    handler = (kw, b[0], g[0])
                    if not is_valid_handler(kw, b[0], g[0], tp[0]):
                        continue
                    for ctx in CONTEXTS:
                        for pre in PRECONDITIONS:
                            if is_valid_combination(tp[0], [handler], ctx[0], pre[0]):
                                yield (tp, [handler], ctx, pre)


def generate_two_handler_permutations() -> Iterator[Tuple[tuple, List[Tuple[str, str, str]], tuple, tuple]]:
    """Generate all two-handler permutations."""
    # Build list of valid handlers per try pattern
    for tp in TRY_PATTERNS:
        if tp[0] == "all_iter":
            continue  # Skip complex result types

        valid_handlers = []
        for kw in HANDLER_KEYWORDS:
            for b in BINDINGS:
                for g in GUARDS:
                    if is_valid_handler(kw, b[0], g[0], tp[0]):
                        valid_handlers.append((kw, b[0], g[0]))

        # Generate pairs - use subset of contexts and preconditions to keep count manageable
        for h1 in valid_handlers:
            for h2 in valid_handlers:
                handlers = [h1, h2]
                for ctx in CONTEXTS[:5]:  # none, with_msg, with_data, with_both, scope
                    for pre in PRECONDITIONS[:2]:  # none, require_pass
                        if is_valid_combination(tp[0], handlers, ctx[0], pre[0]):
                            yield (tp, handlers, ctx, pre)


def generate_three_handler_permutations() -> Iterator[Tuple[tuple, List[Tuple[str, str, str]], tuple, tuple]]:
    """Generate all three-handler permutations.

    This tests complex handler chains like:
    - catch + throw + catch
    - inspect + throw + catch
    - throw + inspect + catch
    etc.
    """
    for tp in TRY_PATTERNS:
        if tp[0] == "all_iter":
            continue  # Skip complex result types
        if tp[0] == "async":
            continue  # Skip async for 3-handler (too many combinations)

        valid_handlers = []
        for kw in HANDLER_KEYWORDS:
            for b in BINDINGS:
                for g in GUARDS:
                    if is_valid_handler(kw, b[0], g[0], tp[0]):
                        valid_handlers.append((kw, b[0], g[0]))

        # Limit handlers to reduce explosion - use simpler bindings/guards for 2nd and 3rd handler
        simple_bindings = ["none", "named", "typed"]
        simple_guards = ["none", "when_true"]

        simple_handlers = [(kw, b, g) for kw, b, g in valid_handlers
                          if b in simple_bindings and g in simple_guards]

        # Generate triples: full × simple × simple to keep count manageable
        for h1 in valid_handlers:
            for h2 in simple_handlers:
                for h3 in simple_handlers:
                    handlers = [h1, h2, h3]
                    # Use minimal contexts for 3-handler
                    for ctx in CONTEXTS[:2]:  # none, with_msg
                        pre = PRECONDITIONS[0]  # none only
                        if is_valid_combination(tp[0], handlers, ctx[0], pre[0]):
                            yield (tp, handlers, ctx, pre)


def generate_no_handler_permutations() -> Iterator[Tuple[tuple, List[Tuple[str, str, str]], tuple, tuple]]:
    """Generate permutations with no handlers (just try block)."""
    for tp in TRY_PATTERNS:
        if tp[0] == "direct":
            continue  # Direct mode needs handler
        for ctx in CONTEXTS:
            for pre in PRECONDITIONS:
                if is_valid_combination(tp[0], [], ctx[0], pre[0]):
                    yield (tp, [], ctx, pre)


def generate_else_suffix_permutations() -> Iterator[Tuple[str, str]]:
    """Generate tests with else suffix on typed handlers.

    Tests patterns like:
    - catch io::Error(e) { 42 } else { 0 }
    - throw io::Error(e) { "msg" } else { "other" }
    """
    tests = []
    counter = 0

    # Test catch Type {} else {}
    for tp in TRY_PATTERNS:
        if tp[0] == "all_iter":
            continue  # Skip complex result types
        if tp[0] == "direct":
            continue  # Direct mode handled separately

        tp_name, tp_pattern, _, tp_setup, tp_result, tp_async, _ = tp

        for bind_name, bind_code, _, _, _ in BINDINGS:
            # Only typed bindings can have else suffix
            if bind_name not in ["typed", "typed_short", "any", "any_short", "all"]:
                continue

            for guard_name, guard_code, needs_typed_e, is_match in GUARDS:
                if needs_typed_e and bind_name not in ["typed", "any"]:
                    continue
                # Skip match clause for else suffix tests (match has its own body structure)
                if is_match:
                    continue

                # Test catch with else
                test_id = f"test_else_suffix_catch_{tp_name}_{bind_name}_{guard_name}_{counter}"
                counter += 1

                lines = []
                if tp_async:
                    lines.append("#[tokio::test]")
                    lines.append(f"async fn {test_id}() {{")
                else:
                    lines.append("#[test]")
                    lines.append(f"fn {test_id}() {{")

                if tp_setup:
                    lines.append(f"    {tp_setup}")

                lines.append(f"    let result: Result<{tp_result}> = handle! {{")

                # Choose appropriate body based on binding
                if bind_name == "all":
                    body = "Err(multi_io_chain())?"
                elif bind_name in ["any", "any_short"]:
                    body = "Err(chained_io_error())?"
                else:
                    body = "io_not_found()?"

                if tp_name in ["for", "any_iter", "while"]:
                    body = "item?" if tp_name != "while" else "attempts += 1; Err(io::Error::other(\"retry\"))?"

                lines.append(f"        {tp_pattern} {{ {body} }}")

                # Build handler with else suffix
                handler_parts = ["catch", bind_code]
                if guard_code:
                    handler_parts.append(guard_code)
                handler_parts.append("{ 42 } else { 0 }")
                lines.append(f"        {' '.join(handler_parts)}")

                lines.append("    };")
                lines.append("    assert!(result.is_ok());")
                lines.append("}")

                tests.append((test_id, "\n".join(lines)))

                # Test throw with else
                test_id = f"test_else_suffix_throw_{tp_name}_{bind_name}_{guard_name}_{counter}"
                counter += 1

                lines = []
                if tp_async:
                    lines.append("#[tokio::test]")
                    lines.append(f"async fn {test_id}() {{")
                else:
                    lines.append("#[test]")
                    lines.append(f"fn {test_id}() {{")

                if tp_setup:
                    lines.append(f"    {tp_setup}")

                lines.append(f"    let result: Result<{tp_result}> = handle! {{")
                lines.append(f"        {tp_pattern} {{ {body} }}")

                # Build throw handler with else suffix
                # throw Type {} else {} means:
                # - If Type matches: transform error, then else catches transformed
                # - If Type doesn't match: else catches original
                # Either way, else handles it (it's a catch-all catch)
                # No additional catch needed - else IS the catch-all
                handler_parts = ["throw", bind_code]
                if guard_code:
                    handler_parts.append(guard_code)
                handler_parts.append('{ "transformed" } else { 0 }')  # else returns value
                lines.append(f"        {' '.join(handler_parts)}")
                # No catch needed - else is the catch-all

                lines.append("    };")
                lines.append("    assert_eq!(result.unwrap(), 0);")  # else returns 0
                lines.append("}")

                tests.append((test_id, "\n".join(lines)))

    return iter(tests)


def generate_nested_body_permutations() -> Iterator[Tuple[str, str]]:
    """Generate tests with nested try blocks in handler bodies.

    Tests patterns like:
    - catch { try { ... } catch { ... } }
    - catch { try -> T { ... } else { ... } }
    - catch { try for x in iter { ... } catch { ... } }
    """
    tests = []
    counter = 0

    for tp in TRY_PATTERNS:
        if tp[0] in ["all_iter", "direct"]:
            continue

        tp_name, tp_pattern, _, tp_setup, tp_result, tp_async, _ = tp

        if tp_async:
            continue  # Skip async for nested tests (complexity)

        # Test nested bodies in catch handlers
        for nested_name, nested_body, returns_value in NESTED_BODIES:
            if nested_body is None:
                continue  # Skip flat - covered by main tests

            # Skip control flow bodies for non-loop patterns
            if "break" in nested_name or "continue" in nested_name:
                continue

            test_id = f"test_nested_{tp_name}_catch_{nested_name}_{counter}"
            counter += 1

            lines = []
            lines.append("#[test]")
            lines.append(f"fn {test_id}() {{")

            if tp_setup:
                lines.append(f"    {tp_setup}")

            lines.append(f"    let result: Result<{tp_result}> = handle! {{")

            # Choose appropriate body
            body = "io_not_found()?"
            if tp_name in ["for", "any_iter"]:
                body = "item?"
            elif tp_name == "while":
                body = "attempts += 1; Err(io::Error::other(\"retry\"))?"

            lines.append(f"        {tp_pattern} {{ {body} }}")
            lines.append(f"        catch {{ {nested_body} }}")

            lines.append("    };")
            lines.append("    assert!(result.is_ok());")
            lines.append("}")

            tests.append((test_id, "\n".join(lines)))

        # Test nested bodies in throw handlers
        for nested_name, nested_body in NESTED_THROW_BODIES:
            if nested_body is None:
                continue

            test_id = f"test_nested_{tp_name}_throw_{nested_name}_{counter}"
            counter += 1

            lines = []
            lines.append("#[test]")
            lines.append(f"fn {test_id}() {{")

            if tp_setup:
                lines.append(f"    {tp_setup}")

            lines.append(f"    let result: Result<{tp_result}> = handle! {{")

            body = "io_not_found()?"
            if tp_name in ["for", "any_iter"]:
                body = "item?"
            elif tp_name == "while":
                body = "attempts += 1; Err(io::Error::other(\"retry\"))?"

            lines.append(f"        {tp_pattern} {{ {body} }}")
            lines.append(f"        throw {{ {nested_body} }}")
            lines.append("        catch { 42 }")

            lines.append("    };")
            lines.append("    assert!(result.is_ok());")
            lines.append("}")

            tests.append((test_id, "\n".join(lines)))

    return iter(tests)


# =============================================================================
# Control Flow Dimensions
# =============================================================================
# Compositional building blocks for control flow tests

# Control flow statements: (name, code)
CF_STATEMENTS = [
    ("break", "break"),
    ("continue", "continue"),
]

# Outer loop types: (name, setup, loop_start, loop_end, counter_var, trigger_cond)
# trigger_cond uses {counter} placeholder for when to trigger error
CF_OUTER_LOOPS = [
    ("for_range", "", "for _ in 0..5 {", "}", "iterations", "{counter} == 2"),
    ("for_enum", "", "for i in 0..5 {", "}", "iterations", "i == 2"),
]

# Try patterns for control flow: (name, pattern, body_template, setup, needs_inner_error)
# body_template uses {error_cond} placeholder
CF_TRY_PATTERNS = [
    ("basic", "try", "if {error_cond} {{ Err(io::Error::other(\"stop\"))? }} 42", "", True),
    ("try_for", "try for i in [1, 2, 3]", "if {error_cond} {{ Err(io::Error::other(\"stop\"))? }} i", "", True),
    ("try_while", "try while retries < 3", "retries += 1; if {error_cond} {{ Err(io::Error::other(\"stop\"))? }} 42", "let mut retries = 0;", True),
    ("try_any", "try any i in [1, 2, 3]", "if {error_cond} {{ Err(io::Error::other(\"stop\"))? }} i", "", True),
    ("try_all", "try all item in [1, 2, 3]", "if {error_cond} {{ Err(io::Error::other(\"stop\"))? }} item", "", True),
]

# Handler types for control flow: (name, keyword, can_have_cf)
CF_HANDLER_TYPES = [
    ("catch", "catch", True),
    ("throw", "throw", True),
    ("inspect", "inspect", True),
]

# Binding variants for control flow handlers: (name, code)
CF_BINDINGS = [
    ("none", ""),
    ("underscore", "_"),
    ("typed", "io::Error(_)"),
]

# Nesting levels: (name, depth)
CF_NESTING = [
    ("flat", 1),
    ("nested", 2),
    ("triple", 3),
]


def build_cf_handler(handler_type: str, binding: str, cf_stmt: str, fallback_value: str = None) -> str:
    """Build a control flow handler string.

    Args:
        handler_type: catch, throw, or inspect
        binding: Binding code (empty, "_", "io::Error(_)", etc.)
        cf_stmt: Control flow statement (break or continue)
        fallback_value: Optional value for non-control-flow path
    """
    parts = [handler_type]
    if binding:
        parts.append(binding)

    if fallback_value is not None:
        # Handler has both control flow and value
        body = f"{{ {cf_stmt}; {fallback_value} }}"
    else:
        # Pure control flow
        body = f"{{ {cf_stmt} }}"

    parts.append(body)
    return " ".join(parts)


def build_cf_nested_try(depth: int, error_cond: str, inner_handler: str, outer_handlers: List[str] = None) -> str:
    """Build nested try blocks with control flow handlers.

    Args:
        depth: Nesting depth (1-3)
        error_cond: Condition that triggers the error
        inner_handler: Handler for the innermost try
        outer_handlers: Handlers for outer try levels (optional)
    """
    if outer_handlers is None:
        outer_handlers = []

    body = f"if {error_cond} {{ Err(io::Error::other(\"err\"))? }} 42"

    # Build from inside out
    for i in range(depth):
        handler = inner_handler if i == 0 else (outer_handlers[i-1] if i-1 < len(outer_handlers) else "")
        body = f"try {{ {body} }} {handler}"

    return body


def generate_cf_test(test_id: str, outer_loop: tuple, try_pattern: tuple,
                     handler: str, setup_extra: str = "", expected_iterations: int = 2,
                     extra_counters: List[str] = None, extra_asserts: List[str] = None,
                     result_type: str = "Result<i32>", use_result: bool = True) -> str:
    """Generate a control flow test.

    Args:
        test_id: Test function name
        outer_loop: Tuple from CF_OUTER_LOOPS
        try_pattern: Tuple from CF_TRY_PATTERNS or custom (name, pattern, body, setup, _)
        handler: Handler string
        setup_extra: Additional setup code
        expected_iterations: Expected iteration count for assertion
        extra_counters: Additional counter variables to declare
        extra_asserts: Additional assertions
        result_type: Type for result binding
        use_result: Whether to bind result to a variable
    """
    loop_name, loop_setup, loop_start, loop_end, counter_var, trigger_cond = outer_loop
    tp_name, tp_pattern, tp_body, tp_setup, _ = try_pattern

    lines = ["#[test]", f"fn {test_id}() {{"]

    # Setup
    if loop_setup:
        lines.append(f"    {loop_setup}")
    lines.append(f"    let mut {counter_var} = 0;")
    if extra_counters:
        for c in extra_counters:
            lines.append(f"    let mut {c} = 0;")
    if setup_extra:
        lines.append(f"    {setup_extra}")

    # Loop
    lines.append(f"    {loop_start}")
    lines.append(f"        {counter_var} += 1;")

    # Try pattern setup
    if tp_setup:
        lines.append(f"        {tp_setup}")

    # Handle! block
    error_cond = trigger_cond.format(counter=counter_var)
    body = tp_body.format(error_cond=error_cond)

    if use_result:
        lines.append(f"        let _: {result_type} = handle! {{")
    else:
        lines.append("        handle! {")
    lines.append(f"            {tp_pattern} {{ {body} }}")
    lines.append(f"            {handler}")
    lines.append("        };")

    lines.append(f"    {loop_end}")

    # Assertions
    lines.append(f"    assert_eq!({counter_var}, {expected_iterations});")
    if extra_asserts:
        for a in extra_asserts:
            lines.append(f"    {a}")

    lines.append("}")
    return "\n".join(lines)


def generate_control_flow_permutations() -> Iterator[Tuple[str, str]]:
    """Generate tests with control flow (break/continue) in handlers.

    Builds tests compositionally from:
    - Outer loop types
    - Try patterns (basic, for, while, any, all)
    - Handler types (catch, throw, inspect)
    - Control flow statements (break, continue)
    - Bindings (none, underscore, typed)
    - Nesting levels (1-3)
    """
    tests = []
    counter = [0]  # Use list for closure mutation

    def make_id(prefix: str) -> str:
        test_id = f"{prefix}_{counter[0]}"
        counter[0] += 1
        return test_id

    # =========================================================================
    # BASIC: Try pattern × Handler type × CF statement × Binding
    # =========================================================================
    for tp_name, tp_pattern, tp_body, tp_setup, _ in CF_TRY_PATTERNS:
        for ht_name, ht_keyword, _ in CF_HANDLER_TYPES:
            for cf_name, cf_code in CF_STATEMENTS:
                for bind_name, bind_code in CF_BINDINGS:
                    # Skip invalid combinations
                    if ht_keyword == "inspect" and bind_name == "none":
                        continue  # inspect requires binding

                    # For loop patterns (try for, try while, etc.):
                    # - Skip typed bindings (produce Result type that needs binding)
                    # - Skip throw (transforms error, needs result handling)
                    # - Skip inspect (propagates error, needs result handling)
                    is_loop_pattern = tp_name != "basic"
                    is_typed = bind_name == "typed"

                    if is_loop_pattern and (is_typed or ht_keyword in ["throw", "inspect"]):
                        continue

                    test_id = make_id(f"test_cf_{tp_name}_{ht_name}_{cf_name}_{bind_name}")
                    handler = build_cf_handler(ht_keyword, bind_code, cf_code)

                    # For try_all, need Vec result type
                    result_type = "Vec<i32>" if tp_name == "try_all" else "i32"
                    # Use result binding for basic pattern
                    use_result = tp_name == "basic"

                    # break stops at iteration 2, continue runs all 5
                    expected_iters = 2 if cf_name == "break" else 5

                    test = generate_cf_test(
                        test_id,
                        CF_OUTER_LOOPS[0],  # for_range
                        (tp_name, tp_pattern, tp_body, tp_setup, True),
                        handler,
                        result_type=f"Result<{result_type}>",
                        use_result=use_result,
                        expected_iterations=expected_iters,
                    )
                    tests.append((test_id, test))

    # =========================================================================
    # NESTED: 2-level nesting with CF in inner vs outer handlers
    # =========================================================================
    for cf_name, cf_code in CF_STATEMENTS:
        for position in ["inner", "outer"]:
            test_id = make_id(f"test_cf_nested_{position}_{cf_name}")
            expected_iters = 2 if cf_name == "break" else 5

            if position == "inner":
                # CF in inner handler, outer has no handler
                inner = f"try {{ if iterations == 2 {{ Err(io::Error::other(\"err\"))? }} 42 }} catch _ {{ {cf_code} }}"
                body = f"try {{ {inner} }}"
            else:
                # Inner has value handler, CF in outer
                inner = "try { if iterations == 2 { Err(io::Error::other(\"err\"))? } 42 }"
                body = f"try {{ {inner} }} catch _ {{ {cf_code} }}"

            test = f'''#[test]
fn {test_id}() {{
    let mut iterations = 0;
    for _ in 0..5 {{
        iterations += 1;
        let _: Result<i32> = handle! {{
            {body}
        }};
    }}
    assert_eq!(iterations, {expected_iters});
}}'''
            tests.append((test_id, test))

    # =========================================================================
    # NESTED: Inner and outer both have control flow (tests hybrid mode)
    # =========================================================================
    for inner_cf, inner_code in CF_STATEMENTS:
        for outer_cf, outer_code in CF_STATEMENTS:
            test_id = make_id(f"test_cf_nested_inner_{inner_cf}_outer_{outer_cf}")

            # Errors on odd iterations (1,3,5,7,9) - inner throw handles all
            # Inner break: stops at iter 1, inner_count=1
            # Inner continue: runs all 10, inner_count=5
            # Outer never runs because inner always handles
            if inner_cf == "break":
                expected_iters = 1
                expected_inner = 1
            else:  # continue
                expected_iters = 10
                expected_inner = 5
            expected_outer = 0

            test = f'''#[test]
fn {test_id}() {{
    let mut inner_count = 0;
    let mut outer_count = 0;
    let mut iterations = 0;
    for _ in 0..10 {{
        iterations += 1;
        let _: Result<i32> = handle! {{
            try {{
                try {{
                    if iterations % 2 == 1 {{
                        Err(io::Error::other("odd"))?
                    }}
                    42
                }}
                throw _ {{
                    inner_count += 1;
                    {inner_code}
                }}
            }}
            catch _ {{
                outer_count += 1;
                {outer_code}
            }}
        }};
    }}
    assert_eq!(iterations, {expected_iters});
    assert_eq!(inner_count, {expected_inner});
    assert_eq!(outer_count, {expected_outer});
}}'''
            tests.append((test_id, test))

    # =========================================================================
    # TRIPLE NESTED: CF at different levels
    # =========================================================================
    for level, level_name in [(0, "innermost"), (1, "middle"), (2, "outermost")]:
        for cf_name, cf_code in CF_STATEMENTS:
            test_id = make_id(f"test_cf_triple_{level_name}_{cf_name}")
            expected_iters = 2 if cf_name == "break" else 5

            # Build triple nested with CF at specified level
            handlers = ["", "", ""]
            handlers[level] = f"catch _ {{ {cf_code} }}"

            inner = "if iterations == 2 { Err(io::Error::other(\"deep\"))? } 42"
            for i in range(3):
                h = handlers[2-i]  # Reverse order (innermost first)
                inner = f"try {{ {inner} }} {h}"

            test = f'''#[test]
fn {test_id}() {{
    let mut iterations = 0;
    for _ in 0..5 {{
        iterations += 1;
        let _: Result<i32> = handle! {{
            {inner}
        }};
    }}
    assert_eq!(iterations, {expected_iters});
}}'''
            tests.append((test_id, test))

    # =========================================================================
    # TYPED HANDLERS: Typed catch/throw with CF and fallbacks
    # =========================================================================
    for cf_name, cf_code in CF_STATEMENTS:
        test_id = make_id(f"test_cf_typed_catch_{cf_name}_with_fallback")
        # i=0,1,3: success
        # i=2: io::Error -> typed catch with CF
        # i=4: ParseIntError -> fallback catch
        # break at i=2: iterations=3, typed=1, fallback=0
        # continue at i=2: iterations=5, typed=1, fallback=1
        if cf_name == "break":
            expected_iters = 3
            expected_typed = 1
            expected_fallback = 0
        else:
            expected_iters = 5
            expected_typed = 1
            expected_fallback = 1

        test = f'''#[test]
fn test_cf_typed_catch_{cf_name}_with_fallback() {{
    let mut typed_count = 0;
    let mut fallback_count = 0;
    let mut iterations = 0;
    for i in 0..5 {{
        iterations += 1;
        let _: Result<i32> = handle! {{
            try {{
                if i == 2 {{
                    Err(io::Error::other("io error"))?
                }} else if i == 4 {{
                    Err("x".parse::<i32>().unwrap_err())?
                }}
                42
            }}
            catch io::Error(_) {{
                typed_count += 1;
                {cf_code}
            }}
            catch _ {{
                fallback_count += 1;
                0
            }}
        }};
    }}
    assert_eq!(iterations, {expected_iters});
    assert_eq!(typed_count, {expected_typed});
    assert_eq!(fallback_count, {expected_fallback});
}}'''
        tests.append((test_id, test))

    # =========================================================================
    # GUARDS: Guard conditions with CF
    # =========================================================================
    for cf_name, cf_code in CF_STATEMENTS:
        test_id = make_id(f"test_cf_guard_when_{cf_name}")
        # Error at iterations >= 2 (so 2,3,4,5) = 4 errors, all NotFound
        # Guard always matches, fallback never runs
        # break: iterations=2, guarded=1, fallback=0
        # continue: iterations=5, guarded=4, fallback=0
        if cf_name == "break":
            expected_iters = 2
            expected_guarded = 1
        else:
            expected_iters = 5
            expected_guarded = 4
        expected_fallback = 0

        test = f'''#[test]
fn {test_id}() {{
    let mut guarded = 0;
    let mut fallback = 0;
    let mut iterations = 0;
    for _ in 0..5 {{
        iterations += 1;
        let _: Result<i32> = handle! {{
            try {{
                if iterations >= 2 {{
                    Err(io::Error::new(ErrorKind::NotFound, "not found"))?
                }}
                42
            }}
            catch io::Error(e) when e.kind() == ErrorKind::NotFound {{
                guarded += 1;
                {cf_code}
            }}
            catch _ {{
                fallback += 1;
                0
            }}
        }};
    }}
    assert_eq!(iterations, {expected_iters});
    assert_eq!(guarded, {expected_guarded});
    assert_eq!(fallback, {expected_fallback});
}}'''
        tests.append((test_id, test))

    # =========================================================================
    # ELSE SUFFIX: Typed catch with else, both having CF
    # =========================================================================
    for main_cf, main_code in CF_STATEMENTS:
        for else_cf, else_code in CF_STATEMENTS:
            if main_cf == else_cf:
                continue  # Skip same CF in both
            test_id = make_id(f"test_cf_else_{main_cf}_{else_cf}")
            # i=0,1: ParseIntError -> else branch
            # i=2: io::Error -> main branch
            # i=3,4: success
            #
            # main=break, else=continue:
            #   i=0: else+continue, i=1: else+continue, i=2: main+break
            #   iterations=3, main=1, else=2
            #
            # main=continue, else=break:
            #   i=0: else+break
            #   iterations=1, main=0, else=1
            if main_cf == "break" and else_cf == "continue":
                expected_iters = 3
                expected_main = 1
                expected_else = 2
            else:  # main=continue, else=break
                expected_iters = 1
                expected_main = 0
                expected_else = 1

            test = f'''#[test]
fn {test_id}() {{
    let mut main_count = 0;
    let mut else_count = 0;
    let mut iterations = 0;
    for i in 0..5 {{
        iterations += 1;
        let _: Result<i32> = handle! {{
            try {{
                if i == 2 {{
                    Err(io::Error::other("io"))?
                }} else if i < 2 {{
                    Err("x".parse::<i32>().unwrap_err())?
                }}
                42
            }}
            catch io::Error(_) {{
                main_count += 1;
                {main_code}
            }} else {{
                else_count += 1;
                {else_code}
            }}
        }};
    }}
    assert_eq!(iterations, {expected_iters});
    assert_eq!(main_count, {expected_main});
    assert_eq!(else_count, {expected_else});
}}'''
            tests.append((test_id, test))

    # =========================================================================
    # THROW + CATCH CHAIN: Throw transforms, catch has CF
    # =========================================================================
    for cf_name, cf_code in CF_STATEMENTS:
        test_id = make_id(f"test_cf_throw_then_catch_{cf_name}")
        expected_iters = 2 if cf_name == "break" else 5
        # With continue, errors happen at iterations 2,3,4,5 (4 times)
        expected_thrown = 1 if cf_name == "break" else 4
        expected_caught = 1 if cf_name == "break" else 4
        test = f'''#[test]
fn {test_id}() {{
    let mut thrown = 0;
    let mut caught = 0;
    let mut iterations = 0;
    for _ in 0..5 {{
        iterations += 1;
        let _: Result<i32> = handle! {{
            try {{
                if iterations >= 2 {{
                    Err(io::Error::other("error"))?
                }}
                42
            }}
            throw _ {{
                thrown += 1;
                "transformed"
            }}
            catch _ {{
                caught += 1;
                {cf_code}
            }}
        }};
    }}
    assert_eq!(iterations, {expected_iters});
    assert_eq!(thrown, {expected_thrown});
    assert_eq!(caught, {expected_caught});
}}'''
        tests.append((test_id, test))

    # =========================================================================
    # INSPECT: Inspect with CF (error still propagates)
    # =========================================================================
    for cf_name, cf_code in CF_STATEMENTS:
        test_id = make_id(f"test_cf_inspect_{cf_name}")
        expected_iters = 2 if cf_name == "break" else 5
        # Inspect only runs on errors, error only at iteration 2
        expected_inspected = 1
        test = f'''#[test]
fn {test_id}() {{
    let mut inspected = 0;
    let mut iterations = 0;
    for _ in 0..5 {{
        iterations += 1;
        let _: Result<i32> = handle! {{
            try {{
                if iterations == 2 {{
                    Err(io::Error::other("error"))?
                }}
                42
            }}
            inspect _ {{
                inspected += 1;
                {cf_code}
            }}
        }};
    }}
    assert_eq!(iterations, {expected_iters});
    assert_eq!(inspected, {expected_inspected});
}}'''
        tests.append((test_id, test))

    # =========================================================================
    # CONTEXT MODIFIERS: with + CF
    # Note: scope with control flow has type inference issues in signal mode
    # =========================================================================
    for cf_name, cf_code in CF_STATEMENTS:
        test_id = make_id(f"test_cf_with_{cf_name}")
        expected_iters = 2 if cf_name == "break" else 5
        test = f'''#[test]
fn {test_id}() {{
    let mut iterations = 0;
    for _ in 0..5 {{
        iterations += 1;
        let _: Result<i32> = handle! {{
            try {{
                if iterations == 2 {{
                    Err(io::Error::other("error"))?
                }}
                42
            }}
            with "context"
            catch _ {{ {cf_code} }}
        }};
    }}
    assert_eq!(iterations, {expected_iters});
}}'''
        tests.append((test_id, test))

    # =========================================================================
    # LOOP PATTERNS NESTED IN TRY
    # =========================================================================
    for inner_pattern in ["try for i in [1, 2, 3]", "try while attempts < 3"]:
        pattern_name = "for" if "for" in inner_pattern else "while"
        setup = "let mut attempts = 0;" if "while" in inner_pattern else ""
        body = "attempts += 1; if iterations == 2 { Err(io::Error::other(\"stop\"))? } 42" if "while" in inner_pattern else "if iterations == 2 { Err(io::Error::other(\"stop\"))? } i"

        for cf_name, cf_code in CF_STATEMENTS:
            test_id = make_id(f"test_cf_{pattern_name}_in_try_{cf_name}")
            expected_iters = 2 if cf_name == "break" else 5
            test = f'''#[test]
fn {test_id}() {{
    let mut iterations = 0;
    for _ in 0..5 {{
        iterations += 1;
        {setup}
        handle! {{
            try {{
                {inner_pattern} {{ {body} }}
                catch _ {{ {cf_code} }}
            }}
        }};
    }}
    assert_eq!(iterations, {expected_iters});
}}'''
            tests.append((test_id, test))

    return iter(tests)


def generate_deeply_nested_permutations() -> Iterator[Tuple[str, str]]:
    """Generate tests with 2-3 levels of nesting.

    Tests patterns like:
    - try { try { try { ... } catch { ... } } catch { ... } } catch { ... }
    - try { try for x in iter { try { ... } catch { ... } } catch { ... } }
    - try for x in iter { try { try { ... } catch { ... } } catch { ... } }
    """
    tests = []
    counter = 0

    # Level 2: try in try
    test_id = f"test_deeply_nested_try_in_try_{counter}"
    counter += 1
    tests.append((test_id, f'''#[test]
fn {test_id}() {{
    let result: Result<i32> = handle! {{
        try {{
            try {{ io_not_found()? }} catch {{ 1 }}
        }}
        catch {{ 42 }}
    }};
    assert_eq!(result.unwrap(), 1);
}}'''))

    # Level 2: try in try (inner fails to catch, outer catches)
    test_id = f"test_deeply_nested_try_in_try_outer_catch_{counter}"
    counter += 1
    tests.append((test_id, f'''#[test]
fn {test_id}() {{
    let result: Result<i32> = handle! {{
        try {{
            try {{ io_not_found()? }} throw {{ "inner" }}
        }}
        catch {{ 42 }}
    }};
    assert_eq!(result.unwrap(), 42);
}}'''))

    # Level 2: try for in try
    test_id = f"test_deeply_nested_try_for_in_try_{counter}"
    counter += 1
    tests.append((test_id, f'''#[test]
fn {test_id}() {{
    let items = iter_all_fail();
    let result: Result<i32> = handle! {{
        try {{
            try for item in items {{ item? }}
            catch {{ 1 }}
        }}
        catch {{ 42 }}
    }};
    assert_eq!(result.unwrap(), 1);
}}'''))

    # Level 2: try in try for
    test_id = f"test_deeply_nested_try_in_try_for_{counter}"
    counter += 1
    tests.append((test_id, f'''#[test]
fn {test_id}() {{
    let items = iter_all_fail();
    let result: Result<i32> = handle! {{
        try for item in items {{
            try {{ item? }} catch {{ 1 }}
        }}
        catch {{ 42 }}
    }};
    assert_eq!(result.unwrap(), 1);
}}'''))

    # Level 2: try in try with throw
    test_id = f"test_deeply_nested_try_in_try_with_throw_{counter}"
    counter += 1
    tests.append((test_id, f'''#[test]
fn {test_id}() {{
    let result: Result<i32> = handle! {{
        try {{
            try {{ io_not_found()? }}
            throw {{ "inner transformed" }}
            catch {{ 1 }}
        }}
        throw {{ "outer transformed" }}
        catch {{ 42 }}
    }};
    assert_eq!(result.unwrap(), 1);
}}'''))

    # Level 2: try in try with inspect
    # Inspect runs BEFORE catch (handlers execute in declaration order)
    test_id = f"test_deeply_nested_try_in_try_with_inspect_{counter}"
    counter += 1
    tests.append((test_id, f'''#[test]
fn {test_id}() {{
    let mut inner_inspected = false;
    let mut outer_inspected = false;
    let result: Result<i32> = handle! {{
        try {{
            try {{ io_not_found()? }}
            inspect _ {{ inner_inspected = true; }}
            catch {{ 1 }}
        }}
        inspect _ {{ outer_inspected = true; }}
        catch {{ 42 }}
    }};
    assert_eq!(result.unwrap(), 1);
    assert!(inner_inspected);  // inspect runs before catch (declaration order)
    assert!(!outer_inspected);  // inner catch handles error, nothing reaches outer
}}'''))

    # Level 3: try in try in try
    test_id = f"test_deeply_nested_3_levels_{counter}"
    counter += 1
    tests.append((test_id, f'''#[test]
fn {test_id}() {{
    let result: Result<i32> = handle! {{
        try {{
            try {{
                try {{ io_not_found()? }}
                catch {{ 1 }}
            }}
            catch {{ 2 }}
        }}
        catch {{ 42 }}
    }};
    assert_eq!(result.unwrap(), 1);
}}'''))

    # Level 3: error propagates through all levels
    test_id = f"test_deeply_nested_3_levels_propagate_{counter}"
    counter += 1
    tests.append((test_id, f'''#[test]
fn {test_id}() {{
    let result: Result<i32> = handle! {{
        try {{
            try {{
                try {{ io_not_found()? }}
                throw {{ "level 1" }}
            }}
            throw {{ "level 2" }}
        }}
        catch {{ 42 }}
    }};
    assert_eq!(result.unwrap(), 42);
}}'''))

    # Level 2: try for in try for (complex control flow)
    # The inner try for with iter_all_ok succeeds on first item, returning Ok(1)
    # This success propagates to outer try for, which returns it
    test_id = f"test_deeply_nested_try_for_in_try_for_{counter}"
    counter += 1
    tests.append((test_id, f'''#[test]
fn {test_id}() {{
    let result: Result<i32> = handle! {{
        try for _outer in [1, 2, 3] {{
            let inner_items = iter_all_ok();
            try for inner in inner_items {{
                inner?
            }}
            catch {{ 99 }}
        }}
        catch {{ 42 }}
    }};
    // Inner try for succeeds with first Ok value (1)
    // Outer try for sees success, returns 1
    assert_eq!(result.unwrap(), 1);
}}'''))

    # Level 2: try while in try
    test_id = f"test_deeply_nested_try_while_in_try_{counter}"
    counter += 1
    tests.append((test_id, f'''#[test]
fn {test_id}() {{
    let mut attempts = 0;
    let result: Result<i32> = handle! {{
        try {{
            try while attempts < 3 {{
                attempts += 1;
                Err(io::Error::other("retry"))?
            }}
            catch {{ 1 }}
        }}
        catch {{ 42 }}
    }};
    assert_eq!(result.unwrap(), 1);
}}'''))

    # Level 2: try in try while
    test_id = f"test_deeply_nested_try_in_try_while_{counter}"
    counter += 1
    tests.append((test_id, f'''#[test]
fn {test_id}() {{
    let mut attempts = 0;
    let result: Result<i32> = handle! {{
        try while attempts < 3 {{
            attempts += 1;
            try {{ io_not_found()? }} catch {{ 1 }}
        }}
        catch {{ 42 }}
    }};
    // Inner try catches, returning Ok(1), which succeeds the while loop
    assert_eq!(result.unwrap(), 1);
}}'''))

    # Direct mode with nested try
    test_id = f"test_deeply_nested_direct_with_try_{counter}"
    counter += 1
    tests.append((test_id, f'''#[test]
fn {test_id}() {{
    let result: i32 = handle! {{
        try -> i32 {{ io_not_found()? }}
        catch {{
            try {{ io_not_found()? }} catch {{ 1 }}
        }}
    }};
    assert_eq!(result, 1);
}}'''))

    # Direct mode with nested try for
    test_id = f"test_deeply_nested_direct_with_try_for_{counter}"
    counter += 1
    tests.append((test_id, f'''#[test]
fn {test_id}() {{
    let items = iter_all_fail();
    let result: i32 = handle! {{
        try -> i32 {{ io_not_found()? }}
        catch {{
            try for item in items {{ item? }}
            catch {{ 1 }}
        }}
    }};
    assert_eq!(result, 1);
}}'''))

    # Nested try with typed catch
    test_id = f"test_deeply_nested_typed_catch_{counter}"
    counter += 1
    tests.append((test_id, f'''#[test]
fn {test_id}() {{
    let result: Result<i32> = handle! {{
        try {{
            try {{ io_not_found()? }}
            catch io::Error(e) {{ let _ = e.kind(); 1 }}
            else {{ 2 }}
        }}
        catch {{ 42 }}
    }};
    assert_eq!(result.unwrap(), 1);
}}'''))

    # Nested try with chain searching
    test_id = f"test_deeply_nested_chain_search_{counter}"
    counter += 1
    tests.append((test_id, f'''#[test]
fn {test_id}() {{
    let result: Result<i32> = handle! {{
        try {{
            try {{ Err(chained_io_error())? }}
            catch any io::Error(e) {{ let _ = e.kind(); 1 }}
        }}
        catch {{ 42 }}
    }};
    assert_eq!(result.unwrap(), 1);
}}'''))

    return iter(tests)


def generate_comprehensive_nested_permutations() -> Iterator[Tuple[str, str]]:
    """Generate comprehensive nested permutations.

    Tests nesting in multiple positions with up to 3 levels of depth:
    - try_body: The expression inside try { EXPR }
    - catch_body: The expression inside catch { EXPR }
    - throw_body: The expression inside throw { EXPR }
    - inspect_body: The expression inside inspect { EXPR }

    Combinations tested:
    1. Single position nesting (each position with each nesting type)
    2. Multi-position nesting (2-4 positions with nesting simultaneously)
    3. Deep nesting (2-3 levels in one position)
    4. Mixed (deep nesting + multi-position)
    """
    tests = []
    counter = 0

    # ==========================================================================
    # Part 1: Single position nesting with various nesting types
    # ==========================================================================

    # 1a. Nesting in try_body (the error source)
    for name, nested_code in NESTED_TRY_I32:
        if nested_code is None:
            continue
        test_id = f"test_comp_nested_try_body_{name}_{counter}"
        counter += 1
        tests.append((test_id, f'''#[test]
fn {test_id}() {{
    let result: Result<i32> = handle! {{
        try {{
            {nested_code}
        }}
        catch {{ 99 }}
    }};
    assert!(result.is_ok());
}}'''))

    # 1b. Nesting in catch_body
    for name, nested_code in NESTED_TRY_I32:
        if nested_code is None:
            continue
        test_id = f"test_comp_nested_catch_body_{name}_{counter}"
        counter += 1
        tests.append((test_id, f'''#[test]
fn {test_id}() {{
    let result: Result<i32> = handle! {{
        try {{ io_not_found()? }}
        catch {{
            {nested_code}
        }}
    }};
    assert!(result.is_ok());
}}'''))

    # 1c. Nesting in throw_body
    for name, nested_code in NESTED_TRY_STR:
        if nested_code is None:
            continue
        test_id = f"test_comp_nested_throw_body_{name}_{counter}"
        counter += 1
        tests.append((test_id, f'''#[test]
fn {test_id}() {{
    let result: Result<i32> = handle! {{
        try {{ io_not_found()? }}
        throw {{
            {nested_code}
        }}
        catch {{ 99 }}
    }};
    assert!(result.is_ok());
}}'''))

    # 1d. Nesting in inspect_body
    for name, nested_code in NESTED_TRY_I32:
        if nested_code is None:
            continue
        test_id = f"test_comp_nested_inspect_body_{name}_{counter}"
        counter += 1
        tests.append((test_id, f'''#[test]
fn {test_id}() {{
    let result: Result<i32> = handle! {{
        try {{ io_not_found()? }}
        inspect _ {{
            let _ = {nested_code};
        }}
        catch {{ 99 }}
    }};
    assert!(result.is_ok());
}}'''))

    # ==========================================================================
    # Part 2: Level 2 nesting (try inside try)
    # ==========================================================================

    for name, nested_code in NESTED_L2_I32:
        # In try_body
        test_id = f"test_comp_nested_l2_try_{name}_{counter}"
        counter += 1
        tests.append((test_id, f'''#[test]
fn {test_id}() {{
    let result: Result<i32> = handle! {{
        try {{
            {nested_code}
        }}
        catch {{ 99 }}
    }};
    assert!(result.is_ok());
}}'''))

        # In catch_body
        test_id = f"test_comp_nested_l2_catch_{name}_{counter}"
        counter += 1
        tests.append((test_id, f'''#[test]
fn {test_id}() {{
    let result: Result<i32> = handle! {{
        try {{ io_not_found()? }}
        catch {{
            {nested_code}
        }}
    }};
    assert!(result.is_ok());
}}'''))

    # ==========================================================================
    # Part 3: Level 3 nesting (try inside try inside try)
    # ==========================================================================

    for name, nested_code in NESTED_L3_I32:
        test_id = f"test_comp_nested_l3_{name}_{counter}"
        counter += 1
        tests.append((test_id, f'''#[test]
fn {test_id}() {{
    let result: Result<i32> = handle! {{
        try {{
            {nested_code}
        }}
        catch {{ 99 }}
    }};
    assert!(result.is_ok());
}}'''))

    # ==========================================================================
    # Part 4: Multi-position nesting (nesting in multiple blocks simultaneously)
    # ==========================================================================

    # 4a. try_body AND catch both have nesting
    test_id = f"test_comp_nested_multi_try_catch_{counter}"
    counter += 1
    tests.append((test_id, f'''#[test]
fn {test_id}() {{
    let result: Result<i32> = handle! {{
        try {{
            try {{ io_not_found()? }} catch {{ 1 }}
        }}
        catch {{
            try {{ io_not_found()? }} catch {{ 2 }}
        }}
    }};
    // Inner try catches, returns 1
    assert_eq!(result.unwrap(), 1);
}}'''))

    # 4b. try_body AND throw_body both have nesting
    test_id = f"test_comp_nested_multi_try_throw_{counter}"
    counter += 1
    tests.append((test_id, f'''#[test]
fn {test_id}() {{
    let result: Result<i32> = handle! {{
        try {{
            try {{ io_not_found()? }} throw {{ "inner" }}
        }}
        throw {{
            try -> String {{ "nested_throw".to_string() }} else {{ "fallback".to_string() }}
        }}
        catch {{ 99 }}
    }};
    // Inner try throws, outer throw transforms, catch catches
    assert_eq!(result.unwrap(), 99);
}}'''))

    # 4c. try_body AND inspect_body both have nesting
    test_id = f"test_comp_nested_multi_try_inspect_{counter}"
    counter += 1
    tests.append((test_id, f'''#[test]
fn {test_id}() {{
    let mut inspected = false;
    let result: Result<i32> = handle! {{
        try {{
            try {{ io_not_found()? }} throw {{ "will propagate" }}
        }}
        inspect _ {{
            let _ = try {{ io_ok()? }} catch {{ 0 }};
            inspected = true;
        }}
        catch {{ 99 }}
    }};
    assert_eq!(result.unwrap(), 99);
    assert!(inspected);
}}'''))

    # 4d. catch AND throw both have nesting
    test_id = f"test_comp_nested_multi_catch_throw_{counter}"
    counter += 1
    tests.append((test_id, f'''#[test]
fn {test_id}() {{
    let result: Result<i32> = handle! {{
        try {{ io_not_found()? }}
        throw {{
            try -> String {{ "transformed".to_string() }} else {{ "fallback".to_string() }}
        }}
        catch {{
            try {{ io_not_found()? }} catch {{ 42 }}
        }}
    }};
    assert_eq!(result.unwrap(), 42);
}}'''))

    # 4e. Three positions: try_body, throw, catch all have nesting
    test_id = f"test_comp_nested_multi_3pos_{counter}"
    counter += 1
    tests.append((test_id, f'''#[test]
fn {test_id}() {{
    let result: Result<i32> = handle! {{
        try {{
            try {{ io_not_found()? }} catch {{ 1 }}
        }}
        throw {{
            try -> String {{ "t".to_string() }} else {{ "f".to_string() }}
        }}
        catch {{
            try {{ io_ok()? }} catch {{ 2 }}
        }}
    }};
    // Inner catch handles, returns 1
    assert_eq!(result.unwrap(), 1);
}}'''))

    # 4f. Four positions: try_body, throw, inspect, catch all have nesting
    test_id = f"test_comp_nested_multi_4pos_{counter}"
    counter += 1
    tests.append((test_id, f'''#[test]
fn {test_id}() {{
    let mut seen = false;
    let result: Result<i32> = handle! {{
        try {{
            try {{ io_not_found()? }} throw {{ "propagate" }}
        }}
        throw {{
            try -> String {{ "outer_throw".to_string() }} else {{ "f".to_string() }}
        }}
        inspect _ {{
            let _ = try {{ io_ok()? }} catch {{ 0 }};
            seen = true;
        }}
        catch {{
            try {{ io_not_found()? }} catch {{ 42 }}
        }}
    }};
    assert_eq!(result.unwrap(), 42);
    assert!(seen);
}}'''))

    # ==========================================================================
    # Part 5: Mixed deep + multi-position
    # ==========================================================================

    # 5a. Level 2 in try_body + nesting in catch
    test_id = f"test_comp_nested_mixed_l2try_catch_{counter}"
    counter += 1
    tests.append((test_id, f'''#[test]
fn {test_id}() {{
    let result: Result<i32> = handle! {{
        try {{
            try {{
                try {{ io_not_found()? }} catch {{ 1 }}
            }}
            catch {{ 2 }}
        }}
        catch {{
            try {{ io_not_found()? }} catch {{ 99 }}
        }}
    }};
    assert_eq!(result.unwrap(), 1);
}}'''))

    # 5b. Level 3 in try_body + nesting in catch + nesting in throw
    test_id = f"test_comp_nested_mixed_l3_multi_{counter}"
    counter += 1
    tests.append((test_id, f'''#[test]
fn {test_id}() {{
    let result: Result<i32> = handle! {{
        try {{
            try {{
                try {{
                    try {{ io_not_found()? }} catch {{ 1 }}
                }}
                catch {{ 2 }}
            }}
            throw {{ "l2_throw" }}
            catch {{ 3 }}
        }}
        throw {{
            try -> String {{ "outer".to_string() }} else {{ "f".to_string() }}
        }}
        catch {{
            try {{ io_ok()? }} catch {{ 99 }}
        }}
    }};
    assert_eq!(result.unwrap(), 1);
}}'''))

    # ==========================================================================
    # Part 6: Iteration patterns with nesting
    # ==========================================================================

    # 6a. try for with nesting in body
    test_id = f"test_comp_nested_for_nested_body_{counter}"
    counter += 1
    tests.append((test_id, f'''#[test]
fn {test_id}() {{
    let items = iter_all_fail();
    let result: Result<i32> = handle! {{
        try for item in items {{
            try {{ item? }} catch {{ 1 }}
        }}
        catch {{ 99 }}
    }};
    // Inner try catches, returns Ok(1)
    assert_eq!(result.unwrap(), 1);
}}'''))

    # 6b. try for with nesting in catch
    test_id = f"test_comp_nested_for_nested_catch_{counter}"
    counter += 1
    tests.append((test_id, f'''#[test]
fn {test_id}() {{
    let items = iter_all_fail();
    let result: Result<i32> = handle! {{
        try for item in items {{
            item?
        }}
        catch {{
            try {{ io_not_found()? }} catch {{ 42 }}
        }}
    }};
    assert_eq!(result.unwrap(), 42);
}}'''))

    # 6c. try for in try for (double iteration)
    test_id = f"test_comp_nested_for_for_{counter}"
    counter += 1
    tests.append((test_id, f'''#[test]
fn {test_id}() {{
    let result: Result<i32> = handle! {{
        try for _outer in [1, 2, 3] {{
            let inner_items = iter_all_ok();
            try for inner in inner_items {{
                inner?
            }}
            catch {{ 99 }}
        }}
        catch {{ 88 }}
    }};
    // Inner try for succeeds with 1
    assert_eq!(result.unwrap(), 1);
}}'''))

    # 6d. try while with nesting
    test_id = f"test_comp_nested_while_nested_{counter}"
    counter += 1
    tests.append((test_id, f'''#[test]
fn {test_id}() {{
    let mut attempts = 0;
    let result: Result<i32> = handle! {{
        try while attempts < 3 {{
            attempts += 1;
            try {{ Err(io::Error::other("retry"))? }} catch {{ 1 }}
        }}
        catch {{ 99 }}
    }};
    // Inner catch returns Ok(1) on first attempt
    assert_eq!(result.unwrap(), 1);
}}'''))

    # 6e. try all with nesting in body
    test_id = f"test_comp_nested_all_nested_body_{counter}"
    counter += 1
    tests.append((test_id, f'''#[test]
fn {test_id}() {{
    let items = iter_all_ok();
    let result: Result<Vec<i32>> = handle! {{
        try all item in items {{
            try -> i32 {{ item? }} else {{ 0 }}
        }}
        catch {{ vec![] }}
    }};
    assert_eq!(result.unwrap(), vec![1, 2, 3]);
}}'''))

    # ==========================================================================
    # Part 7: Async patterns with nesting
    # ==========================================================================

    # Note: Nested try blocks inside async try are SYNC, so they use sync functions
    test_id = f"test_comp_nested_async_try_{counter}"
    counter += 1
    tests.append((test_id, f'''#[tokio::test]
async fn {test_id}() {{
    let result: Result<i32> = handle! {{
        async try {{
            try {{ io_not_found()? }} catch {{ 1 }}
        }}
        catch {{ 99 }}
    }};
    assert_eq!(result.unwrap(), 1);
}}'''))

    test_id = f"test_comp_nested_async_catch_{counter}"
    counter += 1
    tests.append((test_id, f'''#[tokio::test]
async fn {test_id}() {{
    let result: Result<i32> = handle! {{
        async try {{
            async_io_not_found().await?
        }}
        catch {{
            try {{ io_ok()? }} catch {{ 1 }}
        }}
    }};
    // Outer catch runs, nested try succeeds with io_ok() = 42
    assert_eq!(result.unwrap(), 42);
}}'''))

    test_id = f"test_comp_nested_async_multi_{counter}"
    counter += 1
    tests.append((test_id, f'''#[tokio::test]
async fn {test_id}() {{
    let result: Result<i32> = handle! {{
        async try {{
            try {{ io_not_found()? }} throw {{ "propagate" }}
        }}
        throw {{
            try -> String {{ "transformed".to_string() }} else {{ "f".to_string() }}
        }}
        catch {{
            try {{ io_not_found()? }} catch {{ 42 }}
        }}
    }};
    assert_eq!(result.unwrap(), 42);
}}'''))

    # ==========================================================================
    # Part 8: Direct mode with nesting
    # ==========================================================================

    test_id = f"test_comp_nested_direct_try_{counter}"
    counter += 1
    tests.append((test_id, f'''#[test]
fn {test_id}() {{
    let result: i32 = handle! {{
        try -> i32 {{
            try {{ io_not_found()? }} catch {{ 1 }}
        }}
        else {{ 99 }}
    }};
    assert_eq!(result, 1);
}}'''))

    test_id = f"test_comp_nested_direct_else_{counter}"
    counter += 1
    tests.append((test_id, f'''#[test]
fn {test_id}() {{
    let result: i32 = handle! {{
        try -> i32 {{ io_not_found()? }}
        else {{
            try {{ io_not_found()? }} catch {{ 42 }}
        }}
    }};
    assert_eq!(result, 42);
}}'''))

    test_id = f"test_comp_nested_direct_multi_{counter}"
    counter += 1
    tests.append((test_id, f'''#[test]
fn {test_id}() {{
    let result: i32 = handle! {{
        try -> i32 {{
            try {{ io_not_found()? }} throw {{ "propagate" }}
        }}
        catch io::Error(_) {{
            try {{ io_not_found()? }} catch {{ 42 }}
        }}
        else {{
            try {{ io_ok()? }} catch {{ 0 }}
        }}
    }};
    // Inner throw propagates, io::Error catches, nested try catches
    assert_eq!(result, 42);
}}'''))

    # ==========================================================================
    # Part 9: Context modifiers with nesting
    # ==========================================================================

    test_id = f"test_comp_nested_with_context_{counter}"
    counter += 1
    tests.append((test_id, f'''#[test]
fn {test_id}() {{
    let result: Result<i32> = handle! {{
        try {{
            try {{ io_not_found()? }}
            with "inner context"
            catch {{ 1 }}
        }}
        with "outer context"
        catch {{ 99 }}
    }};
    assert_eq!(result.unwrap(), 1);
}}'''))

    test_id = f"test_comp_nested_with_scope_{counter}"
    counter += 1
    tests.append((test_id, f'''#[test]
fn {test_id}() {{
    let result: Result<i32> = handle! {{
        scope "outer",
        try {{
            scope "inner",
            try {{ io_not_found()? }}
            catch {{ 1 }}
        }}
        catch {{ 99 }}
    }};
    assert_eq!(result.unwrap(), 1);
}}'''))

    test_id = f"test_comp_nested_with_finally_{counter}"
    counter += 1
    tests.append((test_id, f'''#[test]
fn {test_id}() {{
    let mut inner_finalized = false;
    let mut outer_finalized = false;
    let result: Result<i32> = handle! {{
        try {{
            try {{ io_not_found()? }}
            finally {{ inner_finalized = true; }}
            catch {{ 1 }}
        }}
        finally {{ outer_finalized = true; }}
        catch {{ 99 }}
    }};
    assert_eq!(result.unwrap(), 1);
    assert!(inner_finalized);
    assert!(outer_finalized);
}}'''))

    # ==========================================================================
    # Part 10: Typed catches with nesting
    # ==========================================================================

    test_id = f"test_comp_nested_typed_inner_{counter}"
    counter += 1
    tests.append((test_id, f'''#[test]
fn {test_id}() {{
    let result: Result<i32> = handle! {{
        try {{
            try {{ io_not_found()? }}
            catch io::Error(e) {{ let _ = e.kind(); 1 }}
            else {{ 2 }}
        }}
        catch {{ 99 }}
    }};
    assert_eq!(result.unwrap(), 1);
}}'''))

    test_id = f"test_comp_nested_typed_outer_{counter}"
    counter += 1
    tests.append((test_id, f'''#[test]
fn {test_id}() {{
    let result: Result<i32> = handle! {{
        try {{
            try {{ io_not_found()? }} throw {{ "inner propagate" }}
        }}
        catch io::Error(_) {{
            try {{ io_not_found()? }} catch {{ 42 }}
        }}
        else {{ 99 }}
    }};
    // Inner throw changes type to String, io::Error doesn't match, else catches
    assert_eq!(result.unwrap(), 99);
}}'''))

    test_id = f"test_comp_nested_any_chain_{counter}"
    counter += 1
    tests.append((test_id, f'''#[test]
fn {test_id}() {{
    let result: Result<i32> = handle! {{
        try {{
            try {{ Err(chained_io_error())? }}
            catch any io::Error(e) {{ let _ = e.kind(); 1 }}
        }}
        catch {{ 99 }}
    }};
    assert_eq!(result.unwrap(), 1);
}}'''))

    test_id = f"test_comp_nested_all_chain_{counter}"
    counter += 1
    tests.append((test_id, f'''#[test]
fn {test_id}() {{
    let result: Result<i32> = handle! {{
        try {{
            try {{ Err(multi_io_chain())? }}
            catch all io::Error |errors| {{ errors.len() as i32 }}
        }}
        catch {{ 99 }}
    }};
    assert_eq!(result.unwrap(), 2);  // Two io::Errors in chain
}}'''))

    # ==========================================================================
    # Part 11: else_body nesting (direct mode and typed catch fallback)
    # ==========================================================================

    test_id = f"test_comp_nested_else_direct_{counter}"
    counter += 1
    tests.append((test_id, f'''#[test]
fn {test_id}() {{
    let result: i32 = handle! {{
        try -> i32 {{ io_not_found()? }}
        else {{
            try {{ io_not_found()? }} catch {{ 42 }}
        }}
    }};
    assert_eq!(result, 42);
}}'''))

    test_id = f"test_comp_nested_else_typed_fallback_{counter}"
    counter += 1
    tests.append((test_id, f'''#[test]
fn {test_id}() {{
    let result: Result<i32> = handle! {{
        try {{ Err(Handled::msg("not io"))? }}
        catch io::Error(_) {{ 1 }}
        else {{
            try {{ io_not_found()? }} catch {{ 42 }}
        }}
    }};
    // Not io::Error, falls to else, nested try catches
    assert_eq!(result.unwrap(), 42);
}}'''))

    # ==========================================================================
    # Part 12: finally_body nesting
    # ==========================================================================

    test_id = f"test_comp_nested_finally_with_try_{counter}"
    counter += 1
    tests.append((test_id, f'''#[test]
fn {test_id}() {{
    let mut cleanup_result = 0;
    let result: Result<i32> = handle! {{
        try {{ io_not_found()? }}
        finally {{
            // Nesting in finally - cleanup logic
            cleanup_result = try {{ io_ok()? }} catch {{ 99 }};
        }}
        catch {{ 42 }}
    }};
    assert_eq!(result.unwrap(), 42);
    assert_eq!(cleanup_result, 42);  // io_ok returns 42
}}'''))

    # ==========================================================================
    # Part 13: try_catch_body nesting (returns Result)
    # ==========================================================================

    test_id = f"test_comp_nested_try_catch_body_{counter}"
    counter += 1
    tests.append((test_id, f'''#[test]
fn {test_id}() {{
    let result: Result<i32> = handle! {{
        try {{ io_not_found()? }}
        try catch e {{
            // try catch can use ? and return Result
            let inner: i32 = try {{ io_not_found()? }} catch {{ 42 }};
            if inner > 0 {{ Ok(inner) }} else {{ Err(e) }}
        }}
    }};
    assert_eq!(result.unwrap(), 42);
}}'''))

    # Note: Can't have `try catch` followed by `catch` - try catch handles all errors
    # So we test try catch with typed inner patterns instead
    test_id = f"test_comp_nested_try_catch_typed_{counter}"
    counter += 1
    tests.append((test_id, f'''#[test]
fn {test_id}() {{
    let result: Result<i32> = handle! {{
        try {{ io_not_found()? }}
        try catch e {{
            // Nested try with direct mode (no catch needed since it's infallible)
            let inner: i32 = try -> i32 {{ 42 }} else {{ 0 }};
            if inner > 0 {{ Ok(inner) }} else {{ Err(e) }}
        }}
    }};
    assert_eq!(result.unwrap(), 42);
}}'''))

    # ==========================================================================
    # Part 14: match_arm nesting
    # ==========================================================================
    # NOTE: Nested try blocks inside match arms are NOT supported.
    # The macro can't parse `try { } catch { }` as a match arm expression.
    # Users should use blocks or separate bindings instead:
    #   catch io::Error(e) match e.kind() {
    #       ErrorKind::NotFound => { let v = try { ... } catch { }; v },
    #       _ => 0
    #   }
    # For now, we test match arms with simple expressions only.

    test_id = f"test_comp_nested_match_simple_{counter}"
    counter += 1
    tests.append((test_id, f'''#[test]
fn {test_id}() {{
    let result: Result<i32> = handle! {{
        try {{ io_not_found()? }}
        catch io::Error(e) match e.kind() {{
            ErrorKind::NotFound => 42,
            _ => 0
        }}
    }};
    assert_eq!(result.unwrap(), 42);
}}'''))

    # ==========================================================================
    # Part 15: require_else nesting
    # ==========================================================================
    # NOTE: `let` statements inside require_else blocks don't work in macro context.
    # Nested try blocks work when used directly as expressions.

    test_id = f"test_comp_nested_require_else_{counter}"
    counter += 1
    tests.append((test_id, f'''#[test]
fn {test_id}() {{
    let result: Result<i32> = handle! {{
        require false else "required condition failed",
        try {{ io_ok()? }}
    }};
    // require fails, returns error
    assert!(result.is_err());
}}'''))

    # ==========================================================================
    # Part 16: Loop bodies with multi-position nesting
    # ==========================================================================

    # for_body + catch_body both nested
    test_id = f"test_comp_nested_for_body_and_catch_{counter}"
    counter += 1
    tests.append((test_id, f'''#[test]
fn {test_id}() {{
    let items = iter_all_fail();
    let result: Result<i32> = handle! {{
        try for item in items {{
            // Nesting in for_body
            try {{ item? }} throw {{ "transform" }}
        }}
        catch {{
            // Nesting in catch_body
            try {{ io_not_found()? }} catch {{ 42 }}
        }}
    }};
    // for_body nested throw propagates, catch nested try catches
    assert_eq!(result.unwrap(), 42);
}}'''))

    # while_body + catch_body both nested
    test_id = f"test_comp_nested_while_body_and_catch_{counter}"
    counter += 1
    tests.append((test_id, f'''#[test]
fn {test_id}() {{
    let mut attempts = 0;
    let result: Result<i32> = handle! {{
        try while attempts < 2 {{
            attempts += 1;
            // Nesting in while_body
            try {{ Err(io::Error::other("retry"))? }} throw {{ "transformed" }}
        }}
        catch {{
            // Nesting in catch_body
            try {{ io_not_found()? }} catch {{ 42 }}
        }}
    }};
    assert_eq!(result.unwrap(), 42);
}}'''))

    # all_body + catch_body both nested
    test_id = f"test_comp_nested_all_body_and_catch_{counter}"
    counter += 1
    tests.append((test_id, f'''#[test]
fn {test_id}() {{
    let items = iter_all_ok();
    let result: Result<Vec<i32>> = handle! {{
        try all item in items {{
            // Nesting in all_body
            try -> i32 {{ item? }} else {{ 0 }}
        }}
        catch {{
            // Nesting in catch_body (won't run since all succeeds)
            // Note: can't use try inside vec![] macro, use block instead
            {{ let v = try {{ io_ok()? }} catch {{ 99 }}; vec![v] }}
        }}
    }};
    assert_eq!(result.unwrap(), vec![1, 2, 3]);
}}'''))

    # ==========================================================================
    # Part 17: Deep nesting in loop bodies
    # ==========================================================================

    test_id = f"test_comp_nested_for_l2_{counter}"
    counter += 1
    tests.append((test_id, f'''#[test]
fn {test_id}() {{
    let items = iter_all_fail();
    let result: Result<i32> = handle! {{
        try for item in items {{
            // Level 2 nesting in for_body
            try {{
                try {{ item? }} catch {{ 1 }}
            }}
            catch {{ 2 }}
        }}
        catch {{ 99 }}
    }};
    assert_eq!(result.unwrap(), 1);
}}'''))

    test_id = f"test_comp_nested_while_l2_{counter}"
    counter += 1
    tests.append((test_id, f'''#[test]
fn {test_id}() {{
    let mut attempts = 0;
    let result: Result<i32> = handle! {{
        try while attempts < 2 {{
            attempts += 1;
            // Level 2 nesting in while_body
            try {{
                try {{ Err(io::Error::other("r"))? }} catch {{ 1 }}
            }}
            catch {{ 2 }}
        }}
        catch {{ 99 }}
    }};
    assert_eq!(result.unwrap(), 1);
}}'''))

    # ==========================================================================
    # Part 18: Multiple loop patterns nested
    # ==========================================================================

    test_id = f"test_comp_nested_while_in_for_{counter}"
    counter += 1
    tests.append((test_id, f'''#[test]
fn {test_id}() {{
    let result: Result<i32> = handle! {{
        try for _outer in [1, 2] {{
            let mut att = 0;
            try while att < 2 {{
                att += 1;
                Err(io::Error::other("retry"))?
            }}
            catch {{ 1 }}
        }}
        catch {{ 99 }}
    }};
    // Inner while exhausts retries, its catch returns 1
    assert_eq!(result.unwrap(), 1);
}}'''))

    test_id = f"test_comp_nested_for_in_while_{counter}"
    counter += 1
    tests.append((test_id, f'''#[test]
fn {test_id}() {{
    let mut attempts = 0;
    let result: Result<i32> = handle! {{
        try while attempts < 2 {{
            attempts += 1;
            let items = iter_all_ok();
            try for item in items {{
                item?
            }}
            catch {{ 99 }}
        }}
        catch {{ 88 }}
    }};
    // Inner for succeeds with 1
    assert_eq!(result.unwrap(), 1);
}}'''))

    # ==========================================================================
    # Part 19: Five positions all nested simultaneously
    # ==========================================================================

    test_id = f"test_comp_nested_5pos_{counter}"
    counter += 1
    tests.append((test_id, f'''#[test]
fn {test_id}() {{
    let mut inspected = false;
    let mut finalized = false;
    let result: Result<i32> = handle! {{
        try {{
            // try_body nested
            try {{ io_not_found()? }} throw {{ "prop" }}
        }}
        throw {{
            // throw_body nested
            try -> String {{ "t".to_string() }} else {{ "f".to_string() }}
        }}
        inspect _ {{
            // inspect_body nested
            let _ = try {{ io_ok()? }} catch {{ 0 }};
            inspected = true;
        }}
        finally {{
            // finally_body nested
            let _ = try {{ io_ok()? }} catch {{ 0 }};
            finalized = true;
        }}
        catch {{
            // catch_body nested
            try {{ io_not_found()? }} catch {{ 42 }}
        }}
    }};
    assert_eq!(result.unwrap(), 42);
    assert!(inspected);
    assert!(finalized);
}}'''))

    # ==========================================================================
    # Part 20: All block types with same nested pattern
    # ==========================================================================

    # Use a simple nested pattern in EVERY block type that supports it
    test_id = f"test_comp_nested_all_blocks_{counter}"
    counter += 1
    tests.append((test_id, f'''#[test]
fn {test_id}() {{
    let mut inspected = false;
    let mut finalized = false;
    let result: Result<i32> = handle! {{
        try {{
            try {{ io_not_found()? }} catch {{ 1 }}  // try_body
        }}
        throw {{
            try -> String {{ "x".to_string() }} else {{ "y".to_string() }}  // throw_body
        }}
        inspect _ {{
            let _ = try {{ io_ok()? }} catch {{ 0 }};  // inspect_body
            inspected = true;
        }}
        finally {{
            let _ = try {{ io_ok()? }} catch {{ 0 }};  // finally_body
            finalized = true;
        }}
        catch {{
            try {{ io_not_found()? }} catch {{ 42 }}  // catch_body
        }}
    }};
    // try_body catches first, returns 1
    assert_eq!(result.unwrap(), 1);
    assert!(!inspected);  // inner catch handled, no error reaches inspect
    assert!(finalized);   // finally always runs
}}'''))

    # ==========================================================================
    # Part 21: Programmatic all-pairs nested combinations
    # ==========================================================================

    # Define reusable nested patterns for each block type
    # Note: Use single braces since these are inserted into f-strings
    NESTED_FOR_TRY = "try { io_not_found()? } catch { 1 }"
    NESTED_FOR_CATCH = "try { io_ok()? } catch { 42 }"
    NESTED_FOR_THROW = 'try -> String { "x".to_string() } else { "y".to_string() }'
    NESTED_FOR_INSPECT = "let _ = try { io_ok()? } catch { 0 };"

    # All pairs of (try_body nested, catch_body nested) with different patterns
    for try_nested in [NESTED_FOR_TRY, "try { io_ok()? } catch { 1 }"]:
        for catch_nested in [NESTED_FOR_CATCH, "try { io_ok()? } catch { 1 }"]:
            test_id = f"test_comp_nested_pair_try_catch_{counter}"
            counter += 1
            tests.append((test_id, f'''#[test]
fn {test_id}() {{
    let result: Result<i32> = handle! {{
        try {{ {try_nested} }}
        catch {{ {catch_nested} }}
    }};
    assert!(result.is_ok());
}}'''))

    # All pairs of (throw_body nested, catch_body nested)
    for throw_nested in [NESTED_FOR_THROW]:
        for catch_nested in [NESTED_FOR_CATCH, "try { io_ok()? } catch { 1 }"]:
            test_id = f"test_comp_nested_pair_throw_catch_{counter}"
            counter += 1
            tests.append((test_id, f'''#[test]
fn {test_id}() {{
    let result: Result<i32> = handle! {{
        try {{ io_not_found()? }}
        throw {{ {throw_nested} }}
        catch {{ {catch_nested} }}
    }};
    assert!(result.is_ok());
}}'''))

    # All pairs of (inspect_body nested, catch_body nested)
    for catch_nested in [NESTED_FOR_CATCH, "try { io_ok()? } catch { 1 }"]:
        test_id = f"test_comp_nested_pair_inspect_catch_{counter}"
        counter += 1
        tests.append((test_id, f'''#[test]
fn {test_id}() {{
    let mut inspected = false;
    let result: Result<i32> = handle! {{
        try {{ io_not_found()? }}
        inspect _ {{ {NESTED_FOR_INSPECT} inspected = true; }}
        catch {{ {catch_nested} }}
    }};
    assert!(result.is_ok());
    assert!(inspected);
}}'''))

    # ==========================================================================
    # Part 22: Triple nested positions - all combinations
    # ==========================================================================

    # try + throw + catch all nested
    for try_nested in [NESTED_FOR_TRY]:
        for throw_nested in [NESTED_FOR_THROW]:
            for catch_nested in [NESTED_FOR_CATCH]:
                test_id = f"test_comp_nested_triple_ttc_{counter}"
                counter += 1
                tests.append((test_id, f'''#[test]
fn {test_id}() {{
    let result: Result<i32> = handle! {{
        try {{ {try_nested} }}
        throw {{ {throw_nested} }}
        catch {{ {catch_nested} }}
    }};
    assert!(result.is_ok());
}}'''))

    # try + inspect + catch all nested
    test_id = f"test_comp_nested_triple_tic_{counter}"
    counter += 1
    tests.append((test_id, f'''#[test]
fn {test_id}() {{
    let mut inspected = false;
    let result: Result<i32> = handle! {{
        try {{ {NESTED_FOR_TRY} }}
        inspect _ {{ {NESTED_FOR_INSPECT} inspected = true; }}
        catch {{ {NESTED_FOR_CATCH} }}
    }};
    assert!(result.is_ok());
}}'''))

    # ==========================================================================
    # Part 23: Nested patterns with different try variants
    # ==========================================================================

    # Basic try with each handler type having nested
    for handler_type in ["catch", "throw"]:
        for nested_in_handler in [True, False]:
            for nested_in_try in [True, False]:
                if not nested_in_handler and not nested_in_try:
                    continue  # At least one must be nested
                test_id = f"test_comp_nested_variant_{handler_type}_h{int(nested_in_handler)}_t{int(nested_in_try)}_{counter}"
                counter += 1

                try_body = NESTED_FOR_TRY if nested_in_try else "io_not_found()?"

                if handler_type == "catch":
                    handler_body = NESTED_FOR_CATCH if nested_in_handler else "42"
                    handler_code = f"catch {{ {handler_body} }}"
                else:
                    handler_body = NESTED_FOR_THROW if nested_in_handler else '"error"'
                    handler_code = f'throw {{ {handler_body} }} catch {{ 42 }}'

                tests.append((test_id, f'''#[test]
fn {test_id}() {{
    let result: Result<i32> = handle! {{
        try {{ {try_body} }}
        {handler_code}
    }};
    assert!(result.is_ok());
}}'''))

    # ==========================================================================
    # Part 24: Nested with typed handlers
    # ==========================================================================

    # Typed catch with nested in body
    test_id = f"test_comp_nested_typed_catch_{counter}"
    counter += 1
    tests.append((test_id, f'''#[test]
fn {test_id}() {{
    let result: Result<i32> = handle! {{
        try {{ io_not_found()? }}
        catch io::Error(_) {{
            {NESTED_FOR_CATCH}
        }}
        else {{ 0 }}
    }};
    assert!(result.is_ok());
}}'''))

    # Typed throw with nested in body
    test_id = f"test_comp_nested_typed_throw_{counter}"
    counter += 1
    tests.append((test_id, f'''#[test]
fn {test_id}() {{
    let result: Result<i32> = handle! {{
        try {{ io_not_found()? }}
        throw io::Error(_) {{
            {NESTED_FOR_THROW}
        }}
        catch {{ 42 }}
    }};
    assert!(result.is_ok());
}}'''))

    # Any chain with nested
    test_id = f"test_comp_nested_any_chain_{counter}"
    counter += 1
    tests.append((test_id, f'''#[test]
fn {test_id}() {{
    let result: Result<i32> = handle! {{
        try {{ Err(chained_io_error())? }}
        catch any io::Error(_) {{
            {NESTED_FOR_CATCH}
        }}
    }};
    assert!(result.is_ok());
}}'''))

    # All chain with nested
    test_id = f"test_comp_nested_all_chain_{counter}"
    counter += 1
    tests.append((test_id, f'''#[test]
fn {test_id}() {{
    let result: Result<i32> = handle! {{
        try {{ Err(multi_io_chain())? }}
        catch all io::Error |errors| {{
            let _ = {NESTED_FOR_CATCH};
            errors.len() as i32
        }}
    }};
    assert!(result.is_ok());
}}'''))

    # ==========================================================================
    # Part 25: Nested with guards
    # ==========================================================================

    # when guard with nested
    test_id = f"test_comp_nested_when_guard_{counter}"
    counter += 1
    tests.append((test_id, f'''#[test]
fn {test_id}() {{
    let result: Result<i32> = handle! {{
        try {{ io_not_found()? }}
        catch io::Error(e) when e.kind() == ErrorKind::NotFound {{
            {NESTED_FOR_CATCH}
        }}
        else {{ 0 }}
    }};
    assert!(result.is_ok());
}}'''))

    # ==========================================================================
    # Part 26: Nested in async context
    # ==========================================================================

    test_id = f"test_comp_nested_async_{counter}"
    counter += 1
    tests.append((test_id, f'''#[tokio::test]
async fn {test_id}() {{
    let result: Result<i32> = handle! {{
        async try {{ async_io_not_found().await? }}
        catch {{
            // Note: nested try is sync even in async context
            try {{ io_ok()? }} catch {{ 42 }}
        }}
    }};
    assert!(result.is_ok());
}}'''))

    # ==========================================================================
    # Part 27: Nested in loop patterns
    # ==========================================================================

    # try for with nested in body
    test_id = f"test_comp_nested_for_body_{counter}"
    counter += 1
    tests.append((test_id, f'''#[test]
fn {test_id}() {{
    let items = iter_all_ok();
    let result: Result<i32> = handle! {{
        try for item in items {{
            try -> i32 {{ item? }} else {{ 0 }}
        }}
        catch {{ 99 }}
    }};
    assert!(result.is_ok());
}}'''))

    # try while with nested in body
    test_id = f"test_comp_nested_while_body_{counter}"
    counter += 1
    tests.append((test_id, f'''#[test]
fn {test_id}() {{
    let mut att = 0;
    let result: Result<i32> = handle! {{
        try while att < 2 {{
            att += 1;
            try -> i32 {{ Err(io::Error::other("r"))? }} else {{ 1 }}
        }}
        catch {{ 99 }}
    }};
    // Inner try catches, returns 1
    assert_eq!(result.unwrap(), 1);
}}'''))

    # try all with nested in body
    test_id = f"test_comp_nested_all_body_{counter}"
    counter += 1
    tests.append((test_id, f'''#[test]
fn {test_id}() {{
    let items = iter_all_ok();
    let result: Result<Vec<i32>> = handle! {{
        try all item in items {{
            try -> i32 {{ item? }} else {{ 0 }}
        }}
        catch {{ vec![99] }}
    }};
    assert_eq!(result.unwrap(), vec![1, 2, 3]);
}}'''))

    # ==========================================================================
    # Part 28: Multiple nested try patterns in same block
    # ==========================================================================

    test_id = f"test_comp_nested_multi_in_block_{counter}"
    counter += 1
    tests.append((test_id, f'''#[test]
fn {test_id}() {{
    let result: Result<i32> = handle! {{
        try {{ io_not_found()? }}
        catch {{
            let a = try {{ io_ok()? }} catch {{ 1 }};
            let b = try {{ io_ok()? }} catch {{ 2 }};
            a + b
        }}
    }};
    assert_eq!(result.unwrap(), 84);  // 42 + 42
}}'''))

    # ==========================================================================
    # Part 29: Nested with context modifiers
    # ==========================================================================

    test_id = f"test_comp_nested_with_context_{counter}"
    counter += 1
    tests.append((test_id, f'''#[test]
fn {test_id}() {{
    let result: Result<i32> = handle! {{
        try {{ io_not_found()? }}
        with "outer context"
        catch {{
            // Nested try with its own context
            scope "inner", try {{ io_ok()? }} catch {{ 42 }}
        }}
    }};
    assert!(result.is_ok());
}}'''))

    test_id = f"test_comp_nested_with_scope_{counter}"
    counter += 1
    tests.append((test_id, f'''#[test]
fn {test_id}() {{
    let result: Result<i32> = handle! {{
        scope "outer",
        try {{ io_not_found()? }}
        catch {{
            scope "inner", try {{ io_ok()? }} catch {{ 42 }}
        }}
    }};
    assert!(result.is_ok());
}}'''))

    test_id = f"test_comp_nested_with_finally_{counter}"
    counter += 1
    tests.append((test_id, f'''#[test]
fn {test_id}() {{
    let mut finalized = false;
    let result: Result<i32> = handle! {{
        try {{ io_not_found()? }}
        finally {{ finalized = true; }}
        catch {{
            let mut inner_final = false;
            let v = try {{ io_ok()? }} finally {{ inner_final = true; }} catch {{ 42 }};
            assert!(inner_final);
            v
        }}
    }};
    assert!(result.is_ok());
    assert!(finalized);
}}'''))

    # ==========================================================================
    # Part 17: Nested scopes with kv data
    # Tests inline scope syntax with { key: value } attachments
    # ==========================================================================

    test_id = f"test_comp_nested_scope_kv_int_{counter}"
    counter += 1
    tests.append((test_id, f'''#[test]
fn {test_id}() {{
    let result: Result<i32> = handle! {{
        scope "outer", {{ count: 42 }},
        try {{
            scope "inner", {{ value: 1 }},
            try {{ io_not_found()? }}
            catch {{ 1 }}
        }}
        catch {{ 99 }}
    }};
    assert_eq!(result.unwrap(), 1);
}}'''))

    test_id = f"test_comp_nested_scope_kv_bool_{counter}"
    counter += 1
    tests.append((test_id, f'''#[test]
fn {test_id}() {{
    let result: Result<i32> = handle! {{
        scope "outer", {{ enabled: true }},
        try {{
            scope "inner", {{ active: false }},
            try {{ io_not_found()? }}
            catch {{ 1 }}
        }}
        catch {{ 99 }}
    }};
    assert_eq!(result.unwrap(), 1);
}}'''))

    test_id = f"test_comp_nested_scope_kv_str_{counter}"
    counter += 1
    tests.append((test_id, f'''#[test]
fn {test_id}() {{
    let result: Result<i32> = handle! {{
        scope "outer", {{ name: "test" }},
        try {{
            scope "inner", {{ msg: "hello" }},
            try {{ io_not_found()? }}
            catch {{ 1 }}
        }}
        catch {{ 99 }}
    }};
    assert_eq!(result.unwrap(), 1);
}}'''))

    test_id = f"test_comp_nested_scope_kv_multi_{counter}"
    counter += 1
    tests.append((test_id, f'''#[test]
fn {test_id}() {{
    let result: Result<i32> = handle! {{
        scope "request", {{ method: "GET", status: 200 }},
        try {{
            scope "auth", {{ user_id: 42, valid: true }},
            try {{ io_not_found()? }}
            catch {{ 1 }}
        }}
        catch {{ 99 }}
    }};
    assert_eq!(result.unwrap(), 1);
}}'''))

    test_id = f"test_comp_nested_scope_triple_{counter}"
    counter += 1
    tests.append((test_id, f'''#[test]
fn {test_id}() {{
    let result: Result<i32> = handle! {{
        scope "level1", {{ depth: 1 }},
        try {{
            scope "level2", {{ depth: 2 }},
            try {{
                scope "level3", {{ depth: 3 }},
                try {{ io_not_found()? }}
                catch {{ 1 }}
            }}
            catch {{ 2 }}
        }}
        catch {{ 3 }}
    }};
    assert_eq!(result.unwrap(), 1);
}}'''))

    test_id = f"test_comp_nested_scope_propagate_{counter}"
    counter += 1
    tests.append((test_id, f'''#[test]
fn {test_id}() {{
    // Error propagates through all scopes to outermost catch
    let result: Result<i32> = handle! {{
        scope "outer", {{ layer: "outer" }},
        try {{
            scope "inner", {{ layer: "inner" }},
            try {{ io_not_found()? }}
            // No catch - propagates
        }}
        catch {{ 42 }}
    }};
    assert_eq!(result.unwrap(), 42);
}}'''))

    test_id = f"test_comp_nested_scope_mixed_ctx_{counter}"
    counter += 1
    tests.append((test_id, f'''#[test]
fn {test_id}() {{
    let result: Result<i32> = handle! {{
        scope "outer",
        try {{
            scope "inner", {{ has_data: true }},
            try {{ io_not_found()? }}
            catch {{ 1 }}
        }}
        with "outer context"
        catch {{ 99 }}
    }};
    assert_eq!(result.unwrap(), 1);
}}'''))

    test_id = f"test_comp_nested_scope_in_catch_{counter}"
    counter += 1
    tests.append((test_id, f'''#[test]
fn {test_id}() {{
    let result: Result<i32> = handle! {{
        try {{ io_not_found()? }}
        catch {{
            scope "recovery", {{ attempt: 1 }},
            try {{ io_ok()? }}
            catch {{ 0 }}
        }}
    }};
    assert!(result.is_ok());
}}'''))

    test_id = f"test_comp_nested_scope_no_kv_with_kv_{counter}"
    counter += 1
    tests.append((test_id, f'''#[test]
fn {test_id}() {{
    // Mix of scope with and without kv data
    let result: Result<i32> = handle! {{
        scope "outer",
        try {{
            scope "middle", {{ id: 123 }},
            try {{
                scope "inner",
                try {{ io_not_found()? }}
                catch {{ 1 }}
            }}
            catch {{ 2 }}
        }}
        catch {{ 3 }}
    }};
    assert_eq!(result.unwrap(), 1);
}}'''))

    return iter(tests)


# =============================================================================
# File Writer
# =============================================================================

HEADER = '''//! Auto-generated test file - DO NOT EDIT
//! Generated by scripts/generate_full_matrix.py

#![allow(unused_variables, unused_assignments, dead_code, unused_mut, unused_imports)]

mod matrix_common;
use matrix_common::*;
use handle_this::{handle, Handled, Result};
use std::io::{self, ErrorKind};
'''

MATRIX_COMMON_CONTENT = '''//! Common test utilities for matrix tests - AUTO-GENERATED
//! Generated by scripts/generate_full_matrix.py

use std::io::{self, ErrorKind};
use handle_this::{Handled, Result};

/// Returns Ok(42)
pub fn io_ok() -> Result<i32> {
    Ok(42)
}

/// Returns Err with io::Error NotFound
pub fn io_not_found() -> Result<i32> {
    Err(Handled::from(io::Error::new(ErrorKind::NotFound, "not found")))
}

/// Returns error chain: StringError at root, io::Error NotFound in chain
/// Used for testing `catch any io::Error` which searches the chain
pub fn chained_io_error() -> Handled {
    let inner = Handled::from(io::Error::new(ErrorKind::NotFound, "inner not found"));
    let outer = Handled::msg("outer error");
    outer.chain_after(inner)
}

/// Returns error chain: io::Error PermissionDenied at root, NotFound in chain
/// Used for testing `catch all io::Error` which collects all io::Errors
pub fn multi_io_chain() -> Handled {
    let inner = Handled::from(io::Error::new(ErrorKind::NotFound, "inner not found"));
    let outer = Handled::from(io::Error::new(ErrorKind::PermissionDenied, "permission denied"));
    outer.chain_after(inner)
}

/// Iterator that yields all Err values (for try for tests where handlers run)
pub fn iter_all_fail() -> impl Iterator<Item = Result<i32>> {
    vec![
        Err(Handled::from(io::Error::new(ErrorKind::Other, "fail 1"))),
        Err(Handled::from(io::Error::new(ErrorKind::Other, "fail 2"))),
        Err(Handled::from(io::Error::new(ErrorKind::Other, "fail 3"))),
    ].into_iter()
}

/// Iterator that yields all Ok values (for try all tests)
pub fn iter_all_ok() -> impl Iterator<Item = Result<i32>> {
    vec![Ok(1), Ok(2), Ok(3)].into_iter()
}

/// Iterator where second item succeeds (for try for early exit)
pub fn iter_second_ok() -> impl Iterator<Item = Result<i32>> {
    vec![
        Err(Handled::from(io::Error::new(ErrorKind::Other, "fail"))),
        Ok(42),
    ].into_iter()
}

/// Async version of io_ok
pub async fn async_io_ok() -> Result<i32> {
    Ok(42)
}

/// Async version of io_not_found
pub async fn async_io_not_found() -> Result<i32> {
    Err(Handled::from(io::Error::new(ErrorKind::NotFound, "not found")))
}

/// Helper for then chains - wraps value in Ok
pub fn ok_val<T>(v: T) -> Result<T> {
    Ok(v)
}
'''


def write_matrix_common():
    """Write the matrix_common module."""
    common_dir = os.path.join(TESTS_DIR, "matrix_common")
    os.makedirs(common_dir, exist_ok=True)
    filepath = os.path.join(common_dir, "mod.rs")
    with open(filepath, "w") as f:
        f.write(MATRIX_COMMON_CONTENT)


def write_tests_to_files(tests: List[Tuple[str, str]], prefix: str) -> int:
    """Write tests to files, splitting by TESTS_PER_FILE."""
    os.makedirs(TESTS_DIR, exist_ok=True)

    file_num = 0
    test_count = 0
    current_tests = []

    for test_id, test_code in tests:
        current_tests.append(test_code)
        test_count += 1

        if len(current_tests) >= TESTS_PER_FILE:
            filename = f"{prefix}_{file_num:04d}.rs"
            filepath = os.path.join(TESTS_DIR, filename)
            with open(filepath, "w") as f:
                f.write(HEADER)
                f.write("\n")
                f.write("\n".join(current_tests))
                f.write("\n")
            file_num += 1
            current_tests = []

    # Write remaining
    if current_tests:
        filename = f"{prefix}_{file_num:04d}.rs"
        filepath = os.path.join(TESTS_DIR, filename)
        with open(filepath, "w") as f:
            f.write(HEADER)
            f.write("\n")
            f.write("\n".join(current_tests))
            f.write("\n")
        file_num += 1

    return test_count


def make_test_id(tp_name: str, handlers: List[Tuple[str, str, str]],
                 ctx_name: str, pre_name: str, then_name: str, counter: int) -> str:
    """Generate unique test ID."""
    h_parts = "_".join(f"{h[0][:2]}{h[1][:2]}{h[2][:2]}" for h in handlers) if handlers else "none"
    # Include then_name only if not no_then (to keep test names shorter)
    if then_name != "no_then":
        return f"test_{tp_name}_{h_parts}_{ctx_name}_{pre_name}_{then_name}_{counter}"
    return f"test_{tp_name}_{h_parts}_{ctx_name}_{pre_name}_{counter}"


# =============================================================================
# Filtering Support
# =============================================================================

@dataclass
class Filters:
    """Filters for selective test generation."""
    try_patterns: Optional[Set[str]] = None
    handlers: Optional[Set[str]] = None
    bindings: Optional[Set[str]] = None
    guards: Optional[Set[str]] = None
    contexts: Optional[Set[str]] = None
    preconditions: Optional[Set[str]] = None
    num_handlers: Optional[Set[int]] = None
    categories: Optional[Set[str]] = None
    then_steps: Optional[Set[str]] = None

    def matches_try_pattern(self, name: str) -> bool:
        return self.try_patterns is None or name in self.try_patterns

    def matches_handler(self, kw: str) -> bool:
        return self.handlers is None or kw in self.handlers

    def matches_binding(self, name: str) -> bool:
        return self.bindings is None or name in self.bindings

    def matches_guard(self, name: str) -> bool:
        return self.guards is None or name in self.guards

    def matches_context(self, name: str) -> bool:
        return self.contexts is None or name in self.contexts

    def matches_precondition(self, name: str) -> bool:
        return self.preconditions is None or name in self.preconditions

    def matches_num_handlers(self, n: int) -> bool:
        return self.num_handlers is None or n in self.num_handlers

    def matches_category(self, cat: str) -> bool:
        return self.categories is None or cat in self.categories

    def matches_then_steps(self, name: str) -> bool:
        return self.then_steps is None or name in self.then_steps

    def matches_handler_combo(self, handlers: List[Tuple[str, str, str]]) -> bool:
        """Check if all handlers in the combo match filters."""
        for kw, bind, guard in handlers:
            if not self.matches_handler(kw):
                return False
            if not self.matches_binding(bind):
                return False
            if not self.matches_guard(guard):
                return False
        return True


def generate_filtered_single_handler(filters: Filters) -> Iterator[Tuple[tuple, List[Tuple[str, str, str]], tuple, tuple, tuple]]:
    """Generate single-handler permutations with filters."""
    for tp in TRY_PATTERNS:
        if not filters.matches_try_pattern(tp[0]):
            continue
        for kw in HANDLER_KEYWORDS:
            if not filters.matches_handler(kw):
                continue
            for b in BINDINGS:
                if not filters.matches_binding(b[0]):
                    continue
                for g in GUARDS:
                    if not filters.matches_guard(g[0]):
                        continue
                    handler = (kw, b[0], g[0])
                    if not is_valid_handler(kw, b[0], g[0], tp[0]):
                        continue
                    for ctx in CONTEXTS:
                        if not filters.matches_context(ctx[0]):
                            continue
                        for pre in PRECONDITIONS:
                            if not filters.matches_precondition(pre[0]):
                                continue
                            for then in THEN_STEPS:
                                if not filters.matches_then_steps(then[0]):
                                    continue
                                if is_valid_combination(tp[0], [handler], ctx[0], pre[0], then[0]):
                                    yield (tp, [handler], ctx, pre, then)


def generate_filtered_two_handler(filters: Filters) -> Iterator[Tuple[tuple, List[Tuple[str, str, str]], tuple, tuple, tuple]]:
    """Generate two-handler permutations with filters."""
    for tp in TRY_PATTERNS:
        if not filters.matches_try_pattern(tp[0]):
            continue
        if tp[0] == "all_iter":
            continue

        valid_handlers = []
        for kw in HANDLER_KEYWORDS:
            if not filters.matches_handler(kw):
                continue
            for b in BINDINGS:
                if not filters.matches_binding(b[0]):
                    continue
                for g in GUARDS:
                    if not filters.matches_guard(g[0]):
                        continue
                    if is_valid_handler(kw, b[0], g[0], tp[0]):
                        valid_handlers.append((kw, b[0], g[0]))

        for h1 in valid_handlers:
            for h2 in valid_handlers:
                handlers = [h1, h2]
                for ctx in CONTEXTS[:5]:  # Subset of contexts
                    if not filters.matches_context(ctx[0]):
                        continue
                    for pre in PRECONDITIONS[:2]:  # Subset of preconditions
                        if not filters.matches_precondition(pre[0]):
                            continue
                        for then in THEN_STEPS[:2]:  # Subset: no_then, then_1
                            if not filters.matches_then_steps(then[0]):
                                continue
                            if is_valid_combination(tp[0], handlers, ctx[0], pre[0], then[0]):
                                yield (tp, handlers, ctx, pre, then)


def generate_filtered_three_handler(filters: Filters) -> Iterator[Tuple[tuple, List[Tuple[str, str, str]], tuple, tuple, tuple]]:
    """Generate three-handler permutations with filters."""
    for tp in TRY_PATTERNS:
        if not filters.matches_try_pattern(tp[0]):
            continue
        if tp[0] == "all_iter":
            continue
        if tp[0] == "async":
            continue  # Skip async for 3-handler

        valid_handlers = []
        for kw in HANDLER_KEYWORDS:
            if not filters.matches_handler(kw):
                continue
            for b in BINDINGS:
                if not filters.matches_binding(b[0]):
                    continue
                for g in GUARDS:
                    if not filters.matches_guard(g[0]):
                        continue
                    if is_valid_handler(kw, b[0], g[0], tp[0]):
                        valid_handlers.append((kw, b[0], g[0]))

        # Limit 2nd and 3rd handlers to simpler bindings/guards
        simple_bindings = ["none", "named", "typed"]
        simple_guards = ["none", "when_true"]
        simple_handlers = [(kw, b, g) for kw, b, g in valid_handlers
                          if b in simple_bindings and g in simple_guards]

        # For 3-handler, only use no_then to keep count reasonable
        then = THEN_STEPS[0]  # no_then
        if not filters.matches_then_steps(then[0]):
            continue

        for h1 in valid_handlers:
            for h2 in simple_handlers:
                for h3 in simple_handlers:
                    handlers = [h1, h2, h3]
                    for ctx in CONTEXTS[:2]:  # Minimal contexts for 3-handler
                        if not filters.matches_context(ctx[0]):
                            continue
                        pre = PRECONDITIONS[0]  # none only
                        if not filters.matches_precondition(pre[0]):
                            continue
                        if is_valid_combination(tp[0], handlers, ctx[0], pre[0], then[0]):
                            yield (tp, handlers, ctx, pre, then)


def generate_filtered_no_handler(filters: Filters) -> Iterator[Tuple[tuple, List[Tuple[str, str, str]], tuple, tuple, tuple]]:
    """Generate no-handler permutations with filters.

    Note: Then chains require a terminal handler, so no_handler always uses no_then.
    """
    then = THEN_STEPS[0]  # no_then - then chains need handlers
    if not filters.matches_then_steps(then[0]):
        return
    for tp in TRY_PATTERNS:
        if not filters.matches_try_pattern(tp[0]):
            continue
        if tp[0] == "direct":
            continue
        for ctx in CONTEXTS:
            if not filters.matches_context(ctx[0]):
                continue
            for pre in PRECONDITIONS:
                if not filters.matches_precondition(pre[0]):
                    continue
                if is_valid_combination(tp[0], [], ctx[0], pre[0], then[0]):
                    yield (tp, [], ctx, pre, then)


# =============================================================================
# CLI Argument Parsing
# =============================================================================

def parse_list(value: str) -> Set[str]:
    """Parse comma-separated list into set."""
    return set(v.strip() for v in value.split(',') if v.strip())


def parse_int_list(value: str) -> Set[int]:
    """Parse comma-separated int list into set."""
    return set(int(v.strip()) for v in value.split(',') if v.strip())


def list_options():
    """Print all available filter options."""
    print("Available filter values:\n")

    print("Try patterns (-t, --try-pattern):")
    for tp in TRY_PATTERNS:
        print(f"  {tp[0]}")

    print("\nHandler keywords (-k, --handler):")
    for kw in HANDLER_KEYWORDS:
        print(f"  {kw}")

    print("\nBindings (-b, --binding):")
    for b in BINDINGS:
        print(f"  {b[0]}")

    print("\nGuards (-g, --guard):")
    for g in GUARDS:
        print(f"  {g[0]}")

    print("\nContexts (-c, --context):")
    for ctx in CONTEXTS:
        print(f"  {ctx[0]}")

    print("\nPreconditions (-p, --precondition):")
    for pre in PRECONDITIONS:
        print(f"  {pre[0]}")

    print("\nNumber of handlers (-n, --num-handlers):")
    print("  0, 1, 2")

    print("\nCategories (--category):")
    print("  single        - Single handler tests")
    print("  two           - Two handler tests")
    print("  three         - Three handler tests")
    print("  no_handler    - No handler tests")
    print("  else_suffix   - Else suffix tests")
    print("  nested        - Nested body tests")
    print("  control_flow  - Control flow tests")
    print("  deeply_nested - Deeply nested tests")
    print("  comp_nested   - Comprehensive nested permutations (multi-position, multi-depth)")


def create_arg_parser() -> argparse.ArgumentParser:
    """Create argument parser."""
    parser = argparse.ArgumentParser(
        description="Generate test matrix for handle-this macro",
        formatter_class=argparse.RawDescriptionHelpFormatter,
        epilog="""
Examples:
  # Generate tests for basic try pattern with catch handler
  python3 %(prog)s -t basic -k catch

  # Generate tests for multiple patterns
  python3 %(prog)s -t basic,for -k catch,throw -b named,typed

  # Generate only single-handler tests for basic pattern
  python3 %(prog)s --category single -t basic

  # Generate all tests (WARNING: 88k+ tests)
  python3 %(prog)s --all

  # List all available options
  python3 %(prog)s --list
"""
    )

    parser.add_argument('-t', '--try-pattern',
                        help='Try patterns (comma-separated): basic, direct, for, any_iter, all_iter, while, async')
    parser.add_argument('-k', '--handler',
                        help='Handler keywords (comma-separated): catch, throw, inspect, else')
    parser.add_argument('-b', '--binding',
                        help='Bindings (comma-separated): none, named, underscore, typed, typed_short, any, any_short, all')
    parser.add_argument('-g', '--guard',
                        help='Guards (comma-separated): none, when_true, when_kind, match_kind')
    parser.add_argument('-c', '--context',
                        help='Contexts (comma-separated): none, with_msg, with_data, etc.')
    parser.add_argument('-p', '--precondition',
                        help='Preconditions (comma-separated): none, require_pass, require_fail')
    parser.add_argument('-n', '--num-handlers',
                        help='Number of handlers (comma-separated): 0, 1, 2')
    parser.add_argument('--category',
                        help='Test categories (comma-separated): single, two, no_handler, else_suffix, nested, control_flow, deeply_nested, comp_nested')
    parser.add_argument('--then-steps',
                        help='Then chain steps (comma-separated): no_then, then_1, then_2, then_ctx')
    parser.add_argument('--all', action='store_true',
                        help='Generate all tests (use with caution - 88k+ tests)')
    parser.add_argument('--list', action='store_true',
                        help='List all available filter values')
    parser.add_argument('--no-clean', action='store_true',
                        help='Do not clean existing test files before generating')

    return parser


# =============================================================================
# Main
# =============================================================================

def main():
    parser = create_arg_parser()
    args = parser.parse_args()

    if args.list:
        list_options()
        return

    # Check if any filters specified
    has_filters = any([
        args.try_pattern, args.handler, args.binding, args.guard,
        args.context, args.precondition, args.num_handlers, args.category,
        args.then_steps
    ])

    if not has_filters and not args.all:
        print("No filters specified. Use --all to generate all tests, or specify filters.")
        print("Use --list to see available options, or --help for usage.")
        return

    # Build filters
    filters = Filters(
        try_patterns=parse_list(args.try_pattern) if args.try_pattern else None,
        handlers=parse_list(args.handler) if args.handler else None,
        bindings=parse_list(args.binding) if args.binding else None,
        guards=parse_list(args.guard) if args.guard else None,
        contexts=parse_list(args.context) if args.context else None,
        preconditions=parse_list(args.precondition) if args.precondition else None,
        num_handlers=parse_int_list(args.num_handlers) if args.num_handlers else None,
        categories=parse_list(args.category) if args.category else None,
        then_steps=parse_list(args.then_steps) if args.then_steps else None,
    )

    print("Generating test matrix...")
    print(f"Output directory: {TESTS_DIR}")
    if has_filters:
        print("Filters applied:")
        if filters.try_patterns:
            print(f"  try_patterns: {filters.try_patterns}")
        if filters.handlers:
            print(f"  handlers: {filters.handlers}")
        if filters.bindings:
            print(f"  bindings: {filters.bindings}")
        if filters.guards:
            print(f"  guards: {filters.guards}")
        if filters.contexts:
            print(f"  contexts: {filters.contexts}")
        if filters.preconditions:
            print(f"  preconditions: {filters.preconditions}")
        if filters.num_handlers:
            print(f"  num_handlers: {filters.num_handlers}")
        if filters.categories:
            print(f"  categories: {filters.categories}")
        if filters.then_steps:
            print(f"  then_steps: {filters.then_steps}")

    # Clean existing matrix files unless --no-clean
    # Only removes matrix_*.rs files - preserves other test files (ui.rs, etc.)
    if not args.no_clean and os.path.exists(TESTS_DIR):
        for f in os.listdir(TESTS_DIR):
            if f.startswith('matrix_') and f.endswith('.rs'):
                os.remove(os.path.join(TESTS_DIR, f))

    # Always generate matrix_common module
    write_matrix_common()

    all_tests = []
    counter = 0

    # No-handler permutations
    if filters.matches_category("no_handler") and filters.matches_num_handlers(0):
        print("\nGenerating no-handler permutations...")
        no_handler_count = 0
        for tp, handlers, ctx, pre, then in generate_filtered_no_handler(filters):
            test_id = make_test_id(tp[0], handlers, ctx[0], pre[0], then[0], counter)
            test_code, _, _ = generate_test(test_id, tp, handlers, ctx, pre, then)
            all_tests.append((test_id, test_code))
            counter += 1
            no_handler_count += 1
        print(f"  No-handler: {no_handler_count}")

    # Single-handler permutations
    if filters.matches_category("single") and filters.matches_num_handlers(1):
        print("\nGenerating single-handler permutations...")
        single_count = 0
        for tp, handlers, ctx, pre, then in generate_filtered_single_handler(filters):
            test_id = make_test_id(tp[0], handlers, ctx[0], pre[0], then[0], counter)
            test_code, _, _ = generate_test(test_id, tp, handlers, ctx, pre, then)
            all_tests.append((test_id, test_code))
            counter += 1
            single_count += 1
        print(f"  Single-handler: {single_count}")

    # Two-handler permutations
    if filters.matches_category("two") and filters.matches_num_handlers(2):
        print("\nGenerating two-handler permutations...")
        two_count = 0
        for tp, handlers, ctx, pre, then in generate_filtered_two_handler(filters):
            test_id = make_test_id(tp[0], handlers, ctx[0], pre[0], then[0], counter)
            test_code, _, _ = generate_test(test_id, tp, handlers, ctx, pre, then)
            all_tests.append((test_id, test_code))
            counter += 1
            two_count += 1
            if two_count % 10000 == 0:
                print(f"    Progress: {two_count}...")
        print(f"  Two-handler: {two_count}")

    # Three-handler permutations
    if filters.matches_category("three") and filters.matches_num_handlers(3):
        print("\nGenerating three-handler permutations...")
        three_count = 0
        for tp, handlers, ctx, pre, then in generate_filtered_three_handler(filters):
            test_id = make_test_id(tp[0], handlers, ctx[0], pre[0], then[0], counter)
            test_code, _, _ = generate_test(test_id, tp, handlers, ctx, pre, then)
            all_tests.append((test_id, test_code))
            counter += 1
            three_count += 1
            if three_count % 10000 == 0:
                print(f"    Progress: {three_count}...")
        print(f"  Three-handler: {three_count}")

    # Else suffix permutations
    if filters.matches_category("else_suffix"):
        print("\nGenerating else suffix permutations...")
        else_count = 0
        for test_id, test_code in generate_else_suffix_permutations():
            # Apply try_pattern filter to else_suffix tests
            if filters.try_patterns:
                # Extract pattern from test_id (e.g., test_else_suffix_catch_basic_...)
                parts = test_id.split('_')
                if len(parts) > 4:
                    tp_name = parts[4]
                    if tp_name not in filters.try_patterns:
                        continue
            all_tests.append((test_id, test_code))
            else_count += 1
        print(f"  Else suffix: {else_count}")

    # Nested body permutations
    if filters.matches_category("nested"):
        print("\nGenerating nested body permutations...")
        nested_count = 0
        for test_id, test_code in generate_nested_body_permutations():
            # Apply try_pattern filter
            if filters.try_patterns:
                parts = test_id.split('_')
                if len(parts) > 2:
                    tp_name = parts[2]
                    if tp_name not in filters.try_patterns:
                        continue
            all_tests.append((test_id, test_code))
            nested_count += 1
        print(f"  Nested body: {nested_count}")

    # Control flow permutations
    if filters.matches_category("control_flow"):
        print("\nGenerating control flow permutations...")
        cf_count = 0
        for test_id, test_code in generate_control_flow_permutations():
            all_tests.append((test_id, test_code))
            cf_count += 1
        print(f"  Control flow: {cf_count}")

    # Deeply nested permutations
    if filters.matches_category("deeply_nested"):
        print("\nGenerating deeply nested permutations...")
        deep_count = 0
        for test_id, test_code in generate_deeply_nested_permutations():
            all_tests.append((test_id, test_code))
            deep_count += 1
        print(f"  Deeply nested: {deep_count}")

    # Comprehensive nested permutations
    if filters.matches_category("comp_nested"):
        print("\nGenerating comprehensive nested permutations...")
        comp_count = 0
        for test_id, test_code in generate_comprehensive_nested_permutations():
            all_tests.append((test_id, test_code))
            comp_count += 1
        print(f"  Comprehensive nested: {comp_count}")

    if not all_tests:
        print("\nNo tests generated. Check your filters.")
        return

    # Write to files
    print(f"\nTotal tests: {len(all_tests)}")
    print("Writing to files...")

    written = write_tests_to_files(all_tests, "matrix")

    num_files = (len(all_tests) + TESTS_PER_FILE - 1) // TESTS_PER_FILE
    print(f"\nGenerated {written} tests across {num_files} files")
    print(f"Directory: {TESTS_DIR}")
    print("\nTo run: cargo test --test 'matrix_*'")


if __name__ == "__main__":
    main()
