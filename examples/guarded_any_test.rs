use handle_this::{handle, Handled};
use std::io::{self, ErrorKind};
type Result<T> = std::result::Result<T, Handled>;

/// Creates chain: PermissionDenied (root) -> NotFound (cause)
fn multi_io_chain() -> Handled {
    let e1 = Handled::wrap(io::Error::new(ErrorKind::NotFound, "first"));
    let e2 = Handled::wrap(io::Error::new(ErrorKind::PermissionDenied, "second"));
    e2.chain_after(e1)
}

fn main() {
    // Test: throw any io::Error with guard that only matches NotFound
    // The chain has PermissionDenied (root) and NotFound (cause)
    // Throw should find NotFound in the chain and transform
    let result: Result<i32> = handle! {
        try { Err(multi_io_chain())? }
        throw any io::Error(e) when e.kind() == ErrorKind::NotFound { format!("found: {:?}", e.kind()) }
        catch all io::Error |errors| { errors.len() as i32 }
    };
    println!("Guarded throw any + catch all: {:?}", result);
    println!("Expected: Err (throw finds NotFound in chain, transforms, catch can't find io::Error)");

    // Test: catch any with guard
    let result2: Result<i32> = handle! {
        try { Err(multi_io_chain())? }
        catch any io::Error(e) when e.kind() == ErrorKind::NotFound { 42 }
    };
    println!("\nGuarded catch any: {:?}", result2);
    println!("Expected: Ok(42) (catch finds NotFound in chain, guard passes)");

    // Test: catch any with guard that won't match
    let result3: Result<i32> = handle! {
        try { Err(multi_io_chain())? }
        catch any io::Error(e) when e.kind() == ErrorKind::Other { 42 }
    };
    println!("\nGuarded catch any (no match): {:?}", result3);
    println!("Expected: Err (no io::Error with Other kind in chain)");
}
