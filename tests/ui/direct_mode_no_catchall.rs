//! Error: direct mode requires catch-all handler

use handle_this::handle;

fn main() {
    let _: i32 = handle! {
        try -> i32 { Err::<i32, &str>("error")? }
        catch std::io::Error { 0 }  // typed catch only - not a catch-all
    };
}
