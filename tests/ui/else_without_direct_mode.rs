//! Error: else requires direct mode (try -> T)

use handle_this::handle;

fn main() {
    let _ = handle! {
        try { Ok::<i32, &str>(42)? }
        catch { 0 }
        else { -1 }  // else not valid without direct mode
    };
}
