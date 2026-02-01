//! Error: inspect after untyped catch is unreachable

use handle_this::handle;

fn main() {
    let _ = handle! {
        try { Err::<i32, &str>("error")? }
        catch e { 0 }         // catches ALL errors
        inspect e { println!("{}", e); } // unreachable!
    };
}
