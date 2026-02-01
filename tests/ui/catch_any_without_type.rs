//! Error: catch any/all requires a type

use handle_this::handle;

fn main() {
    let _ = handle! {
        try { Err::<i32, &str>("error")? }
        catch any e { 0 }  // any without type path
    };
}
