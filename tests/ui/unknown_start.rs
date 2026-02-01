//! Error: unknown pattern at start of handle! block

use handle_this::handle;

fn main() {
    let _ = handle! {
        foo  // not a valid keyword
    };
}
