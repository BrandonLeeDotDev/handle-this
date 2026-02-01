//! Error: scope must come before try, not after handlers

use handle_this::handle;

fn main() {
    let _ = handle! {
        try { Err::<i32, &str>("error")? }
        catch { 0 }
        scope "test",  // scope after handlers is wrong
        try { Ok(1)? }
    };
}
