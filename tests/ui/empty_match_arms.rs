//! Error: empty match arms

use handle_this::handle;

fn main() {
    let _ = handle! {
        try { Err::<i32, &str>("error")? }
        catch e match e { }  // empty match arms
    };
}
