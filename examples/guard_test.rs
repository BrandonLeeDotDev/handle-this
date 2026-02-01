use handle_this::{handle, Result};
use std::io;

fn io_err(msg: &str) -> io::Error {
    io::Error::new(io::ErrorKind::Other, msg)
}

fn main() {
    // Test 1: Guard matches - should catch
    println!("=== Test 1: Guard matches ===");
    let result: Result<i32> = handle! {
        try {
            Err(io_err("error code 42"))?
        }
        catch io::Error(e) when e.to_string().contains("42") {
            println!("Caught with guard: {}", e);
            100
        }
    };
    println!("Result: {:?}\n", result);

    // Test 2: Type matches but guard fails - what happens?
    println!("=== Test 2: Type matches, guard FAILS ===");
    let result: Result<i32> = handle! {
        try {
            Err(io_err("error code 99"))?  // doesn't contain "42"
        }
        catch io::Error(e) when e.to_string().contains("42") {
            println!("Caught with guard: {}", e);
            100
        }
    };
    println!("Result: {:?}\n", result);

    // Test 3: With catch-all fallback
    println!("=== Test 3: Guard fails, catch-all handles ===");
    let result: Result<i32> = handle! {
        try {
            Err(io_err("error code 99"))?
        }
        catch io::Error(e) when e.to_string().contains("42") {
            println!("Caught io::Error with 42");
            100
        }
        catch e {
            println!("Catch-all got: {}", e);
            200
        }
    };
    println!("Result: {:?}\n", result);

    // Test 4: Control flow with guard - uses signal mode since handler has break
    // Signal mode returns Result, not T
    println!("=== Test 4: Control flow + guard (signal mode) ===");
    for i in 0..3 {
        print!("i={}: ", i);
        let result: Result<i32> = handle! {
            try {
                if i == 1 {
                    Err(io_err("error code 42"))?  // guard will match
                } else if i == 2 {
                    Err(io_err("error code 99"))?  // guard will FAIL
                }
                i * 10
            }
            catch io::Error(error) when error.to_string().contains("42") {
                println!("guard matched, breaking");
                break;
            }
            catch { 0 }  // catch-all for remaining errors
        };
        println!("result = {:?}", result);
    }

    // Test 5: Explicit direct mode with `try -> T`
    println!("\n=== Test 5: Explicit direct mode (try -> T) ===");
    // No type annotation needed at binding site!
    // Direct mode requires catch-all for clear error handling
    let val = handle! {
        try -> i32 {
            Err(io_err("explicit direct"))?
        }
        catch io::Error(e) {
            println!("Caught in explicit direct mode: {}", e);
            42
        }
        catch { unreachable!() }  // catch-all required
    };
    println!("val = {}", val);

    // Test 6: Explicit direct mode with `else` (cleaner catch-all syntax)
    println!("\n=== Test 6: Explicit direct mode with else ===");
    let val = handle! {
        try -> i32 {
            Err(io_err("will hit else"))?
        }
        else {
            println!("else block executed");
            -1
        }
    };
    println!("val = {}", val);
}
