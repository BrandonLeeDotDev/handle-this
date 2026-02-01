//! Error: try body cannot be empty

use handle_this::handle;

fn main() {
    let _ = handle! {
        try { }  // empty body
        catch { 0 }
    };
}
