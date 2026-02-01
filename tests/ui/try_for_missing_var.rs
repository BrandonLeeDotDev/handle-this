//! Error: try for missing variable

use handle_this::handle;

fn main() {
    let _ = handle! {
        try for in [1, 2, 3] { Ok::<i32, &str>(1) }
    };
}
