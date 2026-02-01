use handle_this::{handle, Handled};
use std::io;
type Result<T> = std::result::Result<T, Handled>;

fn main() {
    // Test 1: throw + untyped catch -> Ok
    let result: Result<i32> = handle! {
        try { Err(io::Error::other("test"))? }
        throw e { format!("transformed: {}", e) }
        catch _e { 42 }
    };
    assert_eq!(result.unwrap(), 42, "throw + untyped catch should be Ok(42)");
    println!("Test 1 passed: throw + untyped catch = Ok(42)");

    // Test 2: throw only -> Err
    let result: Result<i32> = handle! {
        try { Err(io::Error::other("test"))? }
        throw e { format!("transformed: {}", e) }
    };
    assert!(result.is_err(), "throw only should be Err");
    println!("Test 2 passed: throw only = Err");

    // Test 3 removed: catch before throw is now a compile error
    // (untyped catch handles all errors, making throw unreachable)

    // Test 3: throw + typed catch (wrong type) -> Err
    let result: Result<i32> = handle! {
        try { Err(io::Error::other("test"))? }
        throw e { format!("transformed: {}", e) }
        catch io::Error(_e) { 42 }
    };
    assert!(result.is_err(), "throw + typed catch (wrong type) should be Err");
    println!("Test 4 passed: throw + typed catch (wrong type) = Err");

    println!("\nAll tests passed!");
}
