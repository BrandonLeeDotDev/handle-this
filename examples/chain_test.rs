use handle_this::{handle, Handled};
use std::io;
type Result<T> = std::result::Result<T, Handled>;

fn main() {
    // Test 1: Basic while without throw
    println!("=== Test 1: While without throw ===");
    let mut attempts = 0;
    let result: Result<i32> = handle! {
        try while attempts < 3 { attempts += 1; Err(io::Error::other("retry"))? }
        catch all io::Error |errors| {
            println!("Found {} io::Errors (no throw)", errors.len());
            errors.len() as i32
        }
    };
    println!("Result: {:?}\n", result);

    // Test 2: While with throw
    println!("=== Test 2: While with throw ===");
    let mut attempts = 0;
    let result: Result<i32> = handle! {
        try while attempts < 3 { attempts += 1; Err(io::Error::other("retry"))? }
        throw any io::Error(e) { format!("transformed: {:?}", e.kind()) }
        catch all io::Error |errors| {
            println!("Found {} io::Errors (with throw)", errors.len());
            errors.len() as i32
        }
    };
    println!("Result: {:?}", result);
}
