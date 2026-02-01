//! Error: reserved binding name

use handle_this::handle;

fn main() {
    let _ = handle! {
        try { Err::<i32, &str>("error")? }
        catch __err { 0 }  // __err is reserved
    };
}
