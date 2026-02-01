use handle_this::{handle, Result};
use std::io;

fn io_err(msg: &str) -> io::Error {
    io::Error::new(io::ErrorKind::Other, msg)
}

// Pattern 1: Simple try with catch-all
fn simple_catchall() -> i32 {
    handle! {
        try -> i32 {
            Err(io_err("fail"))?
        } else {
            0
        }
    }
}

// Pattern 2: Try with typed catch
fn typed_catch() -> Result<i32> {
    handle! {
        try {
            Err(io_err("fail"))?
        }
        catch io::Error(_) {
            42
        }
    }
}

// Pattern 3: Try for loop
fn try_for_loop() -> Result<i32> {
    handle! {
        try for i in 0..3 {
            if i == 2 { return Ok(i) }
            Err(io_err("not yet"))?
        }
        catch { -1 }
    }
}

// Pattern 4: Try while (retry)
fn try_while_retry() -> Result<i32> {
    let mut count = 0;
    handle! {
        try while count < 3 {
            count += 1;
            if count < 3 { Err(io_err("retry"))? }
            count
        }
        catch { -1 }
    }
}

// Pattern 5: Chain any
fn chain_any() -> Result<i32> {
    handle! {
        try {
            Err(io_err("inner"))?
        }
        catch any io::Error(_) {
            42
        }
        catch { 0 }
    }
}

// Pattern 6: Inspect + catch
fn inspect_catch() -> Result<i32> {
    handle! {
        try {
            Err(io_err("fail"))?
        }
        inspect io::Error(e) {
            println!("saw: {}", e);
        }
        catch { 0 }
    }
}

// Pattern 7: Throw (rethrow)
fn throw_pattern() -> i32 {
    handle! {
        try -> i32 {
            Err(io_err("original"))?
        }
        throw io::Error(e) {
            io::Error::new(io::ErrorKind::Other, format!("wrapped: {}", e))
        }
        else { 0 }
    }
}

// Pattern 8: With context
fn with_context() -> Result<i32> {
    handle! {
        try {
            Err(io_err("fail"))?
        }
        with "operation context"
    }
}

fn main() {
    println!("simple_catchall: {}", simple_catchall());
    println!("typed_catch: {:?}", typed_catch());
    println!("try_for_loop: {:?}", try_for_loop());
    println!("try_while_retry: {:?}", try_while_retry());
    println!("chain_any: {:?}", chain_any());
    println!("inspect_catch: {:?}", inspect_catch());
    println!("throw_pattern: {:?}", throw_pattern());
    println!("with_context: {:?}", with_context());
}
