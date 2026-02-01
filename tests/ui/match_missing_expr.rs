//! Error: match missing expression

use handle_this::handle;

fn main() {
    let _ = handle! {
        try { Err::<i32, &str>("error")? }
        catch e match { _ => 0 }  // missing expression before { }
    };
}
