//! Error: inspect any requires a type

use handle_this::handle;

fn main() {
    let _ = handle! {
        try { Err::<i32, &str>("error")? }
        inspect any e { println!("log"); }
        catch { 0 }
    };
}
