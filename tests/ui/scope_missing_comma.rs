//! Error: scope requires comma after name

use handle_this::handle;

fn main() {
    let _ = handle! {
        scope "test"  // missing comma
        try { Ok::<i32, &str>(42)? }
    };
}
