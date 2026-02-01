//! Error: catch requires a body

use handle_this::handle;

fn main() {
    let _ = handle! {
        try { Ok::<i32, &str>(42)? }
        catch e  // missing body
    };
}
