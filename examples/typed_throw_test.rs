use handle_this::{handle, Handled};
use std::io::{self, ErrorKind};
type Result<T> = std::result::Result<T, Handled>;

/// Creates a Handled error with multiple io::Errors in the chain
fn multi_io_chain() -> Handled {
    let e1 = Handled::wrap(io::Error::new(ErrorKind::NotFound, "first"));
    let e2 = Handled::wrap(io::Error::new(ErrorKind::PermissionDenied, "second"));
    e2.chain_after(e1)
}

fn main() {
    // Test: typed throw + typed catch - same type
    // throw any io::Error transforms matching errors to String
    // catch all io::Error should NOT match (error is now String)
    let result: Result<i32> = handle! {
        try { Err(multi_io_chain())? }
        throw any io::Error { "transformed" }
        catch all io::Error |errors| { errors.len() as i32 }
    };
    println!("Typed throw + typed catch (same type): {:?}", result);
    println!("Expected: Err (throw transforms io::Error to String, catch can't find io::Error)");

    // Test: typed throw + untyped catch
    let result2: Result<i32> = handle! {
        try { Err(multi_io_chain())? }
        throw any io::Error { "transformed" }
        catch _e { 42 }
    };
    println!("\nTyped throw + untyped catch: {:?}", result2);
    println!("Expected: Ok(42) (untyped catch catches anything)");

    // Test: typed throw that doesn't match + typed catch
    let result3: Result<i32> = handle! {
        try { Err(multi_io_chain())? }
        throw any std::num::ParseIntError { "transformed" }  // won't match io::Error
        catch all io::Error |errors| { errors.len() as i32 }
    };
    println!("\nTyped throw (no match) + typed catch: {:?}", result3);
    println!("Expected: Ok(2) (throw doesn't transform, catch finds original io::Errors)");
}
