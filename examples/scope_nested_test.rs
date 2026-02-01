//! Test nested scope functionality

use handle_this::{handle, Result, Handled};

fn main() {
    println!("=== Testing Nested Scope ===\n");

    // Test 1: Simple scope at top level
    println!("Test 1: Top level scope");
    let result: Result<i32> = handle! {
        scope "outer",
        try {
            Err(Handled::from(std::io::Error::other("test")))?
        }
        catch { 42 }
    };
    println!("  Result: {:?}\n", result);

    // Test 2: Nested scope inside try body
    println!("Test 2: Scope nested in try body");
    let result: Result<i32> = handle! {
        try {
            scope "inner", try {
                Err(Handled::from(std::io::Error::other("inner error")))?
            }
            catch { 1 }
        }
        catch { 99 }
    };
    println!("  Result: {:?}\n", result);

    // Test 3: Double nested scopes
    println!("Test 3: Double nested scopes");
    let result: Result<i32> = handle! {
        scope "level1",
        try {
            scope "level2", try {
                Err(Handled::from(std::io::Error::other("deep error")))?
            }
            catch { 1 }
        }
        catch { 99 }
    };
    println!("  Result: {:?}\n", result);

    // Test 4: Scope in catch body
    println!("Test 4: Scope in catch body");
    let result: Result<i32> = handle! {
        try {
            Err(Handled::from(std::io::Error::other("outer error")))?
        }
        catch {
            scope "in_catch", try {
                Ok::<_, Handled>(42)?
            }
            catch { 0 }
        }
    };
    println!("  Result: {:?}\n", result);
}
