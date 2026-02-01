//! Error: unknown keyword in handler position

use handle_this::handle;

fn main() {
    let _ = handle! {
        try { Ok::<i32, &str>(42)? }
        handle e { 0 }  // 'handle' is not a valid keyword
    };
}
