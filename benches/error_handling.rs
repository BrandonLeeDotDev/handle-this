//! Benchmarks for error handling performance.
//!
//! Compares handle_this macro against idiomatic Rust patterns.
//! Each benchmark pair does EQUIVALENT work - same allocations, same operations.
//!
//! Run with: cargo bench

use criterion::{black_box, criterion_group, criterion_main, Criterion};
use handle_this::{handle, Result, Handled};
use std::io;
use std::error::Error;

// ============================================================
// Test helpers
// ============================================================

#[inline(never)]
fn io_err(msg: &str) -> io::Error {
    io::Error::new(io::ErrorKind::Other, msg)
}

#[inline(never)]
fn fallible_ok() -> std::result::Result<i32, io::Error> {
    Ok(42)
}

#[inline(never)]
fn fallible_err() -> std::result::Result<i32, io::Error> {
    Err(io_err("fail"))
}

// ============================================================
// 1. BASELINE: Error creation costs
// ============================================================

fn bench_baseline_io_error(c: &mut Criterion) {
    c.bench_function("baseline_io_error", |b| {
        b.iter(|| black_box(io_err("fail")))
    });
}

fn bench_baseline_boxed_error(c: &mut Criterion) {
    c.bench_function("baseline_boxed_error", |b| {
        b.iter(|| {
            let e: Box<dyn Error + Send + Sync> = Box::new(io_err("fail"));
            black_box(e)
        })
    });
}

fn bench_baseline_handled(c: &mut Criterion) {
    c.bench_function("baseline_handled", |b| {
        b.iter(|| {
            let e = Handled::wrap(io_err("fail")).frame("test.rs", 1, 1);
            black_box(e)
        })
    });
}

// ============================================================
// 2. SUCCESS PATH: Fallible operation that succeeds
// ============================================================

fn bench_success_handle(c: &mut Criterion) {
    c.bench_function("success_handle", |b| {
        b.iter(|| {
            let result: i32 = handle! {
                try -> i32 { fallible_ok()? }
                else { 0 }
            };
            black_box(result)
        })
    });
}

fn bench_success_rust(c: &mut Criterion) {
    c.bench_function("success_rust", |b| {
        b.iter(|| {
            let result: i32 = fallible_ok().unwrap_or(0);
            black_box(result)
        })
    });
}

// ============================================================
// 3. RECOVER UNUSED: Catch error, ignore it, return default
// ============================================================

fn bench_recover_unused_handle(c: &mut Criterion) {
    c.bench_function("recover_unused_handle", |b| {
        b.iter(|| {
            let result: i32 = handle! {
                try -> i32 { fallible_err()? }
                else { black_box(0) }
            };
            black_box(result)
        })
    });
}

fn bench_recover_unused_rust(c: &mut Criterion) {
    c.bench_function("recover_unused_rust", |b| {
        b.iter(|| {
            let result: i32 = fallible_err().unwrap_or(black_box(0));
            black_box(result)
        })
    });
}

// ============================================================
// 4. RECOVER USED: Catch error, use it, return derived value
//    Fair comparison: both must box for type erasure
// ============================================================

fn bench_recover_used_handle(c: &mut Criterion) {
    c.bench_function("recover_used_handle", |b| {
        b.iter(|| {
            let result: i32 = handle! {
                try -> i32 { fallible_err()? }
                catch e {
                    // Access just the message (not full trace)
                    black_box(e.message().len() as i32)
                }
            };
            black_box(result)
        })
    });
}

fn bench_recover_used_handle_full(c: &mut Criterion) {
    c.bench_function("recover_used_handle_full", |b| {
        b.iter(|| {
            let result: i32 = handle! {
                try -> i32 { fallible_err()? }
                catch e {
                    // Full trace format (includes locations, context)
                    black_box(e.to_string().len() as i32)
                }
            };
            black_box(result)
        })
    });
}

fn bench_recover_used_rust(c: &mut Criterion) {
    c.bench_function("recover_used_rust", |b| {
        b.iter(|| {
            // Fair comparison: box the error like handle_this does
            let result: i32 = fallible_err()
                .map_err(|e| -> Box<dyn Error + Send + Sync> { Box::new(e) })
                .unwrap_or_else(|e| black_box(e.to_string().len() as i32));
            black_box(result)
        })
    });
}

// Unfair comparison for reference - shows boxing overhead
fn bench_recover_used_rust_nobox(c: &mut Criterion) {
    c.bench_function("recover_used_rust_nobox", |b| {
        b.iter(|| {
            let result: i32 = fallible_err()
                .unwrap_or_else(|e| black_box(e.to_string().len() as i32));
            black_box(result)
        })
    });
}

// ============================================================
// 5. TYPED CATCH MATCH: Type-specific error handling
//    Both must box + downcast
// ============================================================

fn bench_typed_match_handle(c: &mut Criterion) {
    c.bench_function("typed_match_handle", |b| {
        b.iter(|| {
            let result: Result<i32> = handle! {
                try { fallible_err()? }
                catch io::Error(_) { black_box(42) }
            };
            black_box(result)
        })
    });
}

fn bench_typed_match_rust(c: &mut Criterion) {
    c.bench_function("typed_match_rust", |b| {
        b.iter(|| {
            let result: std::result::Result<i32, Box<dyn Error + Send + Sync>> =
                fallible_err()
                    .map_err(|e| -> Box<dyn Error + Send + Sync> { Box::new(e) })
                    .or_else(|boxed| {
                        if boxed.downcast_ref::<io::Error>().is_some() {
                            Ok(black_box(42))
                        } else {
                            Err(boxed)
                        }
                    });
            black_box(result)
        })
    });
}

// ============================================================
// 6. TYPED CATCH MISS: Type doesn't match, error propagates
// ============================================================

fn bench_typed_miss_handle(c: &mut Criterion) {
    c.bench_function("typed_miss_handle", |b| {
        b.iter(|| {
            let result: Result<i32> = handle! {
                try { fallible_err()? }
                catch std::fmt::Error(_) { black_box(42) }
            };
            black_box(result)
        })
    });
}

fn bench_typed_miss_rust(c: &mut Criterion) {
    c.bench_function("typed_miss_rust", |b| {
        b.iter(|| {
            let result: std::result::Result<i32, Box<dyn Error + Send + Sync>> =
                fallible_err()
                    .map_err(|e| -> Box<dyn Error + Send + Sync> { Box::new(e) })
                    .or_else(|boxed| {
                        if boxed.downcast_ref::<std::fmt::Error>().is_some() {
                            Ok(black_box(42))
                        } else {
                            Err(boxed)
                        }
                    });
            black_box(result)
        })
    });
}

// ============================================================
// 7. TYPED + FALLBACK: Type check with catch-all
//    Tests early type check optimization
// ============================================================

fn bench_typed_fallback_handle(c: &mut Criterion) {
    c.bench_function("typed_fallback_handle", |b| {
        b.iter(|| {
            let result: i32 = handle! {
                try -> i32 { fallible_err()? }
                catch io::Error(_) { black_box(42) }
                else { black_box(0) }
            };
            black_box(result)
        })
    });
}

fn bench_typed_fallback_rust(c: &mut Criterion) {
    c.bench_function("typed_fallback_rust", |b| {
        b.iter(|| {
            let result: i32 = fallible_err()
                .map_err(|e| -> Box<dyn Error + Send + Sync> { Box::new(e) })
                .map_or_else(
                    |boxed| {
                        if boxed.downcast_ref::<io::Error>().is_some() {
                            black_box(42)
                        } else {
                            black_box(0)
                        }
                    },
                    |v| v,
                );
            black_box(result)
        })
    });
}

// ============================================================
// 8. MULTI-TYPED: Multiple type-specific handlers
//    This is where handle_this ergonomics shine
// ============================================================

fn bench_multi_typed_handle(c: &mut Criterion) {
    c.bench_function("multi_typed_handle", |b| {
        b.iter(|| {
            let result: i32 = handle! {
                try -> i32 { fallible_err()? }
                catch std::fmt::Error(_) { black_box(1) }
                catch std::num::ParseIntError(_) { black_box(2) }
                catch io::Error(_) { black_box(3) }
                else { black_box(0) }
            };
            black_box(result)
        })
    });
}

fn bench_multi_typed_rust(c: &mut Criterion) {
    c.bench_function("multi_typed_rust", |b| {
        b.iter(|| {
            let result: i32 = fallible_err()
                .map_err(|e| -> Box<dyn Error + Send + Sync> { Box::new(e) })
                .map_or_else(
                    |boxed| {
                        if boxed.downcast_ref::<std::fmt::Error>().is_some() {
                            black_box(1)
                        } else if boxed.downcast_ref::<std::num::ParseIntError>().is_some() {
                            black_box(2)
                        } else if boxed.downcast_ref::<io::Error>().is_some() {
                            black_box(3)
                        } else {
                            black_box(0)
                        }
                    },
                    |v| v,
                );
            black_box(result)
        })
    });
}

// ============================================================
// 9. NESTED: Multiple levels of error handling
// ============================================================

fn bench_nested_handle(c: &mut Criterion) {
    c.bench_function("nested_handle", |b| {
        b.iter(|| {
            let result: i32 = handle! {
                try -> i32 {
                    try -> i32 {
                        try -> i32 {
                            fallible_err()?
                        } else { black_box(1) }
                    } else { black_box(2) }
                } else { black_box(3) }
            };
            black_box(result)
        })
    });
}

fn bench_nested_rust(c: &mut Criterion) {
    c.bench_function("nested_rust", |b| {
        b.iter(|| {
            // Equivalent: each level catches and recovers
            let result: i32 = fallible_err()
                .map_err(|e| -> Box<dyn Error + Send + Sync> { Box::new(e) })
                .or_else(|_| -> std::result::Result<i32, Box<dyn Error + Send + Sync>> {
                    Ok(black_box(1))
                })
                .or_else(|_| -> std::result::Result<i32, Box<dyn Error + Send + Sync>> {
                    Ok(black_box(2))
                })
                .unwrap_or(black_box(3));
            black_box(result)
        })
    });
}

// ============================================================
// 10. STACK TRACE: The feature handle_this provides
//     No idiomatic equivalent - this is the value proposition
// ============================================================

fn bench_stack_trace_collect(c: &mut Criterion) {
    c.bench_function("stack_trace_collect", |b| {
        b.iter(|| {
            let result: Result<i32> = handle! {
                try { fallible_err()? }
                with "context"
            };
            // Access the trace to ensure it's computed
            if let Err(e) = &result {
                black_box(e.frames().count());
            }
            black_box(result)
        })
    });
}

fn bench_stack_trace_deep(c: &mut Criterion) {
    c.bench_function("stack_trace_deep", |b| {
        b.iter(|| {
            fn level3() -> Result<i32> {
                handle! { try { fallible_err()? } with "level3" }
            }
            fn level2() -> Result<i32> {
                handle! { try { level3()? } with "level2" }
            }
            fn level1() -> Result<i32> {
                handle! { try { level2()? } with "level1" }
            }
            let result = level1();
            if let Err(e) = &result {
                black_box(e.frames().count());
            }
            black_box(result)
        })
    });
}

// ============================================================
// 11. REAL WORLD: Simulated realistic error handling
// ============================================================

#[inline(never)]
fn parse_config(s: &str) -> std::result::Result<i32, std::num::ParseIntError> {
    s.parse()
}

#[inline(never)]
fn load_file(_path: &str) -> std::result::Result<String, io::Error> {
    Err(io_err("file not found"))
}

fn bench_realistic_handle(c: &mut Criterion) {
    c.bench_function("realistic_handle", |b| {
        b.iter(|| {
            let result: i32 = handle! {
                try -> i32 {
                    let content = load_file("config.txt")?;
                    parse_config(&content)?
                }
                catch io::Error(_) { black_box(-1) }
                catch std::num::ParseIntError(_) { black_box(-2) }
                else { black_box(-3) }
            };
            black_box(result)
        })
    });
}

fn bench_realistic_rust(c: &mut Criterion) {
    c.bench_function("realistic_rust", |b| {
        b.iter(|| {
            let result: i32 = (|| -> std::result::Result<i32, Box<dyn Error + Send + Sync>> {
                let content = load_file("config.txt")?;
                let value = parse_config(&content)?;
                Ok(value)
            })()
            .map_or_else(
                |e| {
                    if e.downcast_ref::<io::Error>().is_some() {
                        black_box(-1)
                    } else if e.downcast_ref::<std::num::ParseIntError>().is_some() {
                        black_box(-2)
                    } else {
                        black_box(-3)
                    }
                },
                |v| v,
            );
            black_box(result)
        })
    });
}

// ============================================================
// Benchmark groups
// ============================================================

criterion_group!(
    baseline,
    bench_baseline_io_error,
    bench_baseline_boxed_error,
    bench_baseline_handled,
);

criterion_group!(
    success_path,
    bench_success_handle,
    bench_success_rust,
);

criterion_group!(
    recover,
    bench_recover_unused_handle,
    bench_recover_unused_rust,
    bench_recover_used_handle,
    bench_recover_used_handle_full,
    bench_recover_used_rust,
    bench_recover_used_rust_nobox,
);

criterion_group!(
    typed_catch,
    bench_typed_match_handle,
    bench_typed_match_rust,
    bench_typed_miss_handle,
    bench_typed_miss_rust,
    bench_typed_fallback_handle,
    bench_typed_fallback_rust,
    bench_multi_typed_handle,
    bench_multi_typed_rust,
);

criterion_group!(
    nested,
    bench_nested_handle,
    bench_nested_rust,
);

criterion_group!(
    stack_trace,
    bench_stack_trace_collect,
    bench_stack_trace_deep,
);

criterion_group!(
    realistic,
    bench_realistic_handle,
    bench_realistic_rust,
);

criterion_main!(baseline, success_path, recover, typed_catch, nested, stack_trace, realistic);
