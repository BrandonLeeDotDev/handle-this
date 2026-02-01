//! Error: catch all requires a type

use handle_this::handle;

fn main() {
    let _ = handle! {
        try { Err::<i32, &str>("error")? }
        catch all e { 0 }  // all without type path
    };
}
