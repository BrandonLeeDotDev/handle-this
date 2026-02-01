//! Error: catch all requires |binding| not (binding)

use handle_this::handle;

fn main() {
    let _ = handle! {
        try { Err::<i32, String>("error".into())? }
        catch all String(e) { 0 }  // should be |e| not (e)
    };
}
