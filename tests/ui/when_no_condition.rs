//! Error: when requires a condition

use handle_this::handle;

fn main() {
    let _ = handle! {
        try { Err::<i32, &str>("error")? }
        catch e when { 0 }
    };
}
