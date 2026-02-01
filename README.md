# handle-this

Ergonomic error handling for Rust with automatic stack traces.

[![Crates.io](https://img.shields.io/crates/v/handle-this.svg)](https://crates.io/crates/handle-this)
[![Documentation](https://docs.rs/handle-this/badge.svg)](https://docs.rs/handle-this)
[![License](https://img.shields.io/crates/l/handle-this.svg)](LICENSE)
[![Tests](https://img.shields.io/badge/tests-210%2C657-brightgreen)](tests/)

## This Is Not Exception Handling

The syntax looks like try/catch, but the semantics are pure Rust. Understanding this upfront will save confusion:

**In exception-based languages:**
```
try { risky() }
catch { handle() }    // catches thrown exception, unwinds stack
throw new Error()     // throws exception, unwinds stack
```

**In handle-this:**
```rust
try { risky()? }
catch { handle() }    // pattern matches Err, returns Ok(handle())
throw { new_error() } // transforms Err, continues to NEXT handler
```

The critical differences:

| | Exceptions | handle-this |
|---|---|---|
| **Mechanism** | Stack unwinding | `Result<T, E>` |
| **`throw`** | Exits immediately | Transforms error, **continues chain** |
| **`catch`** | Catches thrown exception | Matches `Err`, returns `Ok` |
| **Propagation** | Implicit | Explicit with `?` |
| **Performance** | Zero-cost try, expensive throw | Zero-cost success, cheap error path |

**Why this matters:** If you expect `throw` to exit like in Java/Python/JavaScript, you'll be confused. In handle-this, `throw` is a **transformation step**, not an exit point. The error continues through subsequent handlers until a `catch` stops it.

```rust
handle! {
    try { Err("oops")? }
    throw e { format!("wrapped: {}", e) }  // transforms, continues
    inspect e { log::error!("{}", e); }    // observes, continues
    catch { "recovered" }                   // catches, returns Ok("recovered")
}
// Returns: Ok("recovered")
```

## Why handle-this?

Rust's `?` operator is great for simple propagation, but real-world error handling often requires more:

```rust
// Without handle-this: verbose, scattered logic
fn load_config(path: &str) -> Result<Config, Box<dyn Error>> {
    let content = std::fs::read_to_string(path)
        .map_err(|e| {
            if e.kind() == ErrorKind::NotFound {
                return Ok(Config::default());  // Can't early return from closure
            }
            format!("reading {}: {}", path, e)
        })?;
    serde_json::from_str(&content)
        .map_err(|e| format!("parsing {}: {}", path, e).into())
}

// With handle-this: clear, declarative
fn load_config(path: &str) -> Result<Config> {
    handle! {
        try {
            let content = std::fs::read_to_string(path)?;
            serde_json::from_str(&content)?
        }
        catch io::Error(e) when e.kind() == ErrorKind::NotFound {
            Config::default()
        }
        throw serde_json::Error(_) {
            format!("invalid config: {}", path)
        }
        with "loading config", { path: path }
    }
}
```

**What you get:**
- **Automatic stack traces** — Every error captures its origin and propagation path
- **Zero-cost success path** — No overhead when operations succeed
- **Type-safe matching** — Pattern match on error types with guards
- **Composable handlers** — Chain multiple handlers in declaration order
- **Iteration patterns** — First-success, collect-all, retry with backoff
- **Tested exhaustively** — 210k+ generated tests covering all syntax permutations

## Installation

```toml
[dependencies]
handle-this = "0.2"
```

MSRV: 1.70.0

## VS Code Extension

Get syntax highlighting for `handle!` macro keywords (`try`, `catch`, `throw`, `inspect`, `finally`, `when`, `scope`, etc.) with the included VS Code extension.

**Why install it?** Without the extension, VS Code's Rust syntax highlighter doesn't recognize handle-this keywords, making the code harder to read. The extension adds proper highlighting and a color picker to customize the appearance.

**Install from VSIX:**
```bash
# From the repository root
code --install-extension handle-this-syntax/handle-this-syntax.vsix
```

**Or build from source:**
```bash
cd handle-this-syntax/extension
npm install -g vsce
vsce package
code --install-extension handle-this-syntax-*.vsix
```

After installation, use the command palette (`Ctrl+Shift+P`) → "handle-this: Configure Colors" to customize highlighting.

## Quick Start

```rust
use handle_this::{handle, Result};

// Wrap errors with automatic location tracking
fn read_file(path: &str) -> Result<String> {
    handle! {
        try { std::fs::read_to_string(path)? }
        with "reading file", { path: path }
    }
}

// Recover from errors (returns String, not Result)
fn read_or_default(path: &str) -> String {
    handle! {
        try -> String { std::fs::read_to_string(path)? }
        else { String::new() }
    }
}

// Multiple typed handlers
fn fetch_data(url: &str) -> Result<Data> {
    handle! {
        try { http::get(url)?.json()? }
        catch http::Timeout(_) { cached_data() }
        throw http::Error(e) { format!("fetch failed: {}", e) }
        catch { Data::default() }
    }
}
```

## Core Concepts

### The Handler Chain

Handlers form a pipeline that processes errors. Understanding terminal vs non-terminal is key:

| Handler | Terminal? | What it does |
|---------|-----------|--------------|
| `catch` | **Yes** | Matches error → returns `Ok(value)` → **chain stops** |
| `try catch` | **Yes** | Matches error → body returns `Result` → **chain stops** |
| `throw` | No | Matches error → transforms it → **continues to next handler** |
| `inspect` | No | Matches error → runs side effect → **continues to next handler** |

```rust
handle! {
    try { fallible()? }
    throw e { wrap(e) }    // 1. transforms error, continues ↓
    inspect e { log(e); }  // 2. logs error, continues ↓
    catch { recover() }    // 3. catches, returns Ok(recover())
}
```

The error flows through ALL matching non-terminal handlers until a terminal handler catches it.

### Execution Order

Handlers execute top-to-bottom in declaration order:

```rust
handle! {
    try { operation()? }
    inspect e { log::error!("{}", e); }          // 1. runs, continues
    throw io::Error(e) { CustomError::Io(e) }    // 2. transforms, continues
    catch CustomError::Recoverable(_) { fallback() }  // 3. if matches, stops
    catch e { panic!("unhandled: {}", e) }       // 4. catches rest
}
```

### Critical Rules

1. **Untyped catch must be last** — catches everything, makes subsequent handlers unreachable
2. **throw changes the type** — typed catches after throw may not match
3. **inspect never stops** — always propagates after running

```rust
// COMPILE ERROR: catch e {} catches everything
handle! {
    try { op()? }
    catch e { 0 }           // catches ALL errors
    catch io::Error(_) { 1 } // unreachable!
}

// CORRECT: typed catches before untyped
handle! {
    try { op()? }
    catch io::Error(_) { 1 }  // specific first
    catch e { 0 }             // catch-all last
}
```

## Patterns Reference

### Basic Patterns

```rust
// Just wrap with location
try { fallible()? }

// Catch with binding
try { op()? } catch e { recover(e) }

// Catch without binding
try { op()? } catch { default() }

// Explicit discard
try { op()? } catch _ { default() }

// Infallible (returns T, not Result<T>)
try -> i32 { parse(s)? } else { 0 }

// Transform error
try { op()? } throw e { format!("failed: {}", e) }

// Side effect then propagate
try { op()? } inspect e { log::error!("{}", e); }

// Chain operations (pass success values through)
try { a()? }, then |x| { b(x)? }, then |y| { c(y)? }
```

### Typed Handlers

```rust
// Match specific type
catch io::Error(e) { handle_io(e) }
catch io::Error { default() }  // no binding

// Type with fallback
catch io::Error(e) { handle_io(e) }
else { handle_other() }

// Typed throw
throw ParseError(e) { format!("parse: {}", e) }

// Typed inspect
inspect NetworkError(e) { metrics.record(e); }
```

### Guards

```rust
// Catch with condition
catch e when is_retryable(&e) { retry() }

// Typed with guard
catch io::Error(e) when e.kind() == NotFound { None }

// Guard on throw
throw io::Error(e) when e.kind() == TimedOut {
    TimeoutError::new(e)
}
```

### Match Clause

```rust
catch io::Error(e) match e.kind() {
    ErrorKind::NotFound => default_value(),
    ErrorKind::PermissionDenied => Err("access denied")?,
    _ => Err(e)?
}
```

### Error Chain Search

For wrapped errors, search the cause chain:

```rust
// First matching error in chain
catch any io::Error(e) { handle(e) }

// All matching errors as Vec
catch all ValidationError |errors| {
    for e in &errors { log::warn!("{}", e); }
    Err("validation failed")?
}

// Chain search with guard
catch any io::Error(e) when e.kind() == NotFound { None }
```

### Iteration Patterns

```rust
// First success (try servers until one works)
try for server in servers { connect(server)? }
catch { Err("all servers failed")? }

// Collect all successes
try all item in items { process(item)? }
catch { vec![] }  // returns Vec of successes

// Retry with condition
let mut attempts = 0;
try while attempts < 3 {
    attempts += 1;
    fallible_op()?
}
```

### Context and Scope

```rust
// Add context message
try { op()? } with "processing request"

// Context with structured data
try { op()? } with "db query", { table: "users", id: user_id }

// Structured data only
try { op()? } with { request_id: req.id, user: req.user }

// Hierarchical scope
scope "http handler",
try {
    scope "validation",
    try { validate(req)? }
}
```

### Cleanup

```rust
try {
    let file = open(path)?;
    process(&file)?
}
finally {
    cleanup();  // always runs
}
catch e { default() }
```

### Preconditions

```rust
handle! {
    require user.is_authenticated() else "not logged in",
    require user.has_permission(action) else {
        format!("no permission for {}", action)
    },
    try { perform(action)? }
}
```

### Nested Patterns

Try blocks can nest freely—inner handlers catch their own errors:

```rust
handle! {
    try {
        // Inner try handles its own errors
        let data = try { fetch()? } catch { cached() };
        process(data)?
    }
    catch e { fallback() }
}
```

### Then Chains

Chain operations together, passing success values through a pipeline:

```rust
handle! {
    try { read_file(path)? },
    then |content| { parse_json(&content)? },
    then |data| { validate(data)? }
    catch ParseError(_) { Data::default() }
}
```

Each `then` receives the `Ok` value from the previous step. Type annotations are supported:

```rust
handle! {
    try { fetch_bytes(url)? },
    then |bytes: Vec<u8>| { decode(&bytes)? },
    then |text: String| { text.to_uppercase() }
    catch { String::new() }
}
```

Context can be added to any step using all `with` variants:

```rust
handle! {
    try { fetch(url)? } with "fetching",
    then |data| { parse(data)? } with { url: url },
    then |parsed| { validate(parsed)? } with "validating", { stage: 2 }
    catch { Default::default() }
}
```

All handlers (`catch`, `throw`, `inspect`, `finally`) work with chains.

### Async Support

All patterns work with async:

```rust
handle! {
    async try {
        let response = fetch(url).await?;
        response.json().await?
    }
    catch Timeout(_) { cached_data().await }
    finally { cleanup().await; }
}
```

### Control Flow

Loop handlers support break/continue to outer loops:

```rust
'outer: for batch in batches {
    handle! {
        try for item in batch { process(item)? }
        catch FatalError(_) { break 'outer }
        catch RetryableError(_) { continue 'outer }
    };
}
```

## Stack Traces

Every error captures its propagation path automatically:

```rust
fn inner() -> Result<()> {
    handle! { try { Err("root cause")? } with "in inner" }
}

fn outer() -> Result<()> {
    handle! { try { inner()? } with "in outer" }
}

// Error displays:
// root cause
//
// Trace (most recent last):
//   src/lib.rs:2:5
//     → in inner
//   src/lib.rs:6:5
//     → in outer
```

Structured data appears in traces:

```rust
with "processing order", { order_id: 12345, customer: "acme" }

// Trace shows:
//   src/orders.rs:42:5
//     → processing order
//     order_id: 12345
//     customer: "acme"
```

## Performance

Success path has zero overhead. Error path cost depends on what you access.

| Scenario | handle-this | Rust | Overhead |
|----------|-------------|------|----------|
| Success path | 1.5ns | 1.5ns | 1.0x |
| Typed catch + fallback | 62ns | 63ns | 1.0x |
| Multi-handler chain | 60ns | 59ns | 1.0x |
| Nested error handling | 61ns | 61ns | 1.0x |
| Realistic workload | 63ns | 63ns | 1.0x |
| Error message access | 106ns | 88ns | 1.2x |
| Full trace format | 384ns | 88ns | 4.4x |
| Typed catch miss | 86ns | 63ns | 1.4x |

**Key tradeoffs:**
- **Success path**: Zero overhead
- **Error caught**: Zero overhead when using typed catches with fallback
- **Message access**: 1.2x overhead for `e.message()` (lazy computation)
- **Full trace**: 4.4x overhead for `e.to_string()` (formats all locations)
- **Type miss**: 1.4x overhead when error propagates (wrapping cost)

Use `e.message()` instead of `e.to_string()` when you only need the error text.

## Feature Flags

| Feature | Description |
|---------|-------------|
| `std` (default) | Standard library support |
| `serde` | Serialize/deserialize errors |
| `anyhow` | Convert from `anyhow::Error` |
| `eyre` | Convert from `eyre::Report` |

## Comparison

| Feature | handle-this | anyhow | thiserror |
|---------|-------------|--------|-----------|
| Automatic stack traces | Yes | RUST_BACKTRACE | No |
| Typed error matching | Yes | Downcast | Define types |
| Guard conditions | Yes | No | No |
| try/catch syntax | Yes | No | No |
| Error transformation | `throw` | `.context()` | Manual |
| Iteration patterns | Yes | No | No |
| no_std support | Yes | No | Yes |

## Testing

The macro is validated by 210k+ generated tests covering:
- All handler combinations (single, two, three handlers)
- All binding variants (named, underscore, typed, untyped)
- All guard conditions
- All iteration patterns
- Nested try patterns at multiple depths
- Async variants
- Control flow (break/continue) in handlers

```bash
# Run all tests
cargo test

# Run specific matrix
cargo test --test matrix_0042
```

## License

Licensed under either of [Apache License, Version 2.0](LICENSE-APACHE) or [MIT license](LICENSE-MIT) at your option.
