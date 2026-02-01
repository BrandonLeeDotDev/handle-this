//! Error: multiple with clauses

use handle_this::handle;

fn main() {
    let _ = handle! {
        try { Err::<i32, &str>("error")? }
        catch { 0 }
        with { key: "first" }
        with { key: "second" }
    };
}
