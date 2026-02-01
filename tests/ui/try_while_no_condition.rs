//! Error: try while requires a condition

use handle_this::handle;

fn main() {
    let _ = handle! {
        try while { Ok::<i32, &str>(1) }
    };
}
