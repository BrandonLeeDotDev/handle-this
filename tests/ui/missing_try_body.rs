//! Error: try requires a body

use handle_this::handle;

fn main() {
    let _ = handle! {
        try
        catch { 0 }
    };
}
