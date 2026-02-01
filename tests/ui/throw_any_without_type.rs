//! Error: throw any requires a type

use handle_this::handle;

fn main() {
    let _ = handle! {
        try { Err::<i32, &str>("error")? }
        throw any e { "transformed" }
    };
}
