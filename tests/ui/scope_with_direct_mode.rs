//! Error: scope cannot be used with direct mode

use handle_this::handle;

fn main() {
    let _: i32 = handle! {
        scope "test",
        try -> i32 { Ok::<i32, &str>(42)? }
        else { 0 }
    };
}
