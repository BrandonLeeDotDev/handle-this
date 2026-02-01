//! Error: inspect requires a binding

use handle_this::handle;

fn main() {
    let _ = handle! {
        try { Err::<i32, &str>("error")? }
        inspect { println!("no binding"); }
        catch { 0 }
    };
}
