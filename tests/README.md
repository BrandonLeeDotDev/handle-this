# Test Suite

## Overview

Tests are **not included in the repository** to avoid bloating the crate and overwhelming IDEs. The test matrix generator creates 210,000+ tests covering every permutation of `handle!` macro syntax.

To run tests, you must generate them first using `scripts/generate_full_matrix.py`.

## Generating Tests

```bash
# Generate all tests (WARNING: creates 88k+ tests)
python3 scripts/generate_full_matrix.py --all

# List available filter options
python3 scripts/generate_full_matrix.py --list
```

### Filter Options

| Flag | Values |
|------|--------|
| `-t, --try-pattern` | basic, direct, for, any_iter, all_iter, while, async |
| `-k, --handler` | catch, throw, inspect, else |
| `-b, --binding` | none, named, underscore, typed, typed_short, any, any_short, all |
| `-g, --guard` | none, when_true, when_false, when_kind, when_perm, when_other, match_kind, match_perm |
| `-c, --context` | none, with_msg, with_data, with_both, scope, scope_data, finally, scope_finally, with_finally |
| `-p, --precondition` | none, require_pass, require_fail |
| `-n, --num-handlers` | 0, 1, 2, 3 |
| `--category` | single, two, three, no_handler, else_suffix, nested, control_flow, deeply_nested, comp_nested |

### Examples

```bash
# Generate only catch handler tests with named bindings
python3 scripts/generate_full_matrix.py -k catch -b named

# Generate async tests with typed handlers
python3 scripts/generate_full_matrix.py -t async -b typed,typed_short

# Generate all single-handler tests for basic try pattern
python3 scripts/generate_full_matrix.py --category single -t basic

# Generate tests for for-loop pattern with guards
python3 scripts/generate_full_matrix.py -t for -g when_kind,when_true
```

## Running Tests

```bash
# Run a specific matrix file
cargo test --test matrix_0042

# Run tests matching a pattern
cargo test test_async_catch_e

# Run all tests (takes a long time)
cargo test
```

## UI Tests

The `ui/` subdirectory contains compile-fail tests using `trybuild`. These verify that invalid macro usage produces helpful error messages.

```bash
# Run UI tests only
cargo test --test ui
```

## Test Structure

- `matrix_*.rs` — Auto-generated permutation tests
- `ui/*.rs` — Compile-fail tests with expected `.stderr` output
- `helpers.rs` — Shared test utilities (io helpers, iterators)
