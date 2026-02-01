//! Error: guard must use 'when' keyword

use handle_this::handle;

fn main() {
    let _ = handle! {
        try { Ok::<i32, &str>(42)? }
        catch e if true { 0 }  // 'if' instead of 'when'
    };
}
