//! Error: require needs else clause

use handle_this::handle;

fn main() {
    let _ = handle! {
        require true,  // missing else
        try { Ok::<i32, &str>(42)? }
    };
}
