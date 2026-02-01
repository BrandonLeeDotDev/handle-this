//! Error: multiple finally blocks

use handle_this::handle;

fn main() {
    let _ = handle! {
        try { Err::<i32, &str>("error")? }
        finally { println!("first"); }
        finally { println!("second"); }
        catch { 0 }
    };
}
