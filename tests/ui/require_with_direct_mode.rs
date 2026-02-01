//! Error: require cannot be used with direct mode

use handle_this::handle;

fn main() {
    let _: i32 = handle! {
        require true else "error",
        try -> i32 { Ok::<i32, &str>(42)? }
        else { 0 }
    };
}
