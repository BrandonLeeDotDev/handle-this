//! Error: try for requires an iterator

use handle_this::handle;

fn main() {
    let _ = handle! {
        try for x in { Ok::<i32, &str>(42)? }  // missing iterator
        catch { 0 }
    };
}
