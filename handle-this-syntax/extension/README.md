# handle-this Syntax

Syntax highlighting extension for the [handle-this](https://crates.io/crates/handle-this) Rust crate.

## About handle-this

`handle-this` is a Rust procedural macro that provides ergonomic try/then/catch error handling with automatic context chaining. This extension provides syntax highlighting for the macro's custom keywords.

## Features

- **Keyword Highlighting** - `try`, `async`, `require`, `then`, `catch`, `throw`, `finally`, `with`, `when`, `else`, `inspect`, `any`, `all` keywords
- **Rainbow `handle!`** - Each letter in a unique color (toggleable)
- **Theme Presets** - Dark+, Monokai, Dracula, One Dark, Nord, Gruvbox, Catppuccin
- **Custom Themes** - Create and save your own color schemes
- **Full Syntax Colors** - Strings, numbers, comments, functions, types, variables, operators, attributes
- **Live Preview** - See changes before applying

## Installation

### From VSIX
```bash
code --install-extension handle-this-syntax.vsix
# or for VSCodium:
codium --install-extension handle-this-syntax.vsix
```

### From Source
Clone and package with `vsce package`, then install the generated `.vsix`.

## Usage

1. Install the [handle-this](https://crates.io/crates/handle-this) crate in your Rust project
2. Install this extension
3. Open any `.rs` file using the `handle!` macro
4. Configure colors: `Ctrl+Shift+P` → `handle-this: Configure Colors`

## Example

```rust
use handle_this::{handle, Result};
use std::io::{self, ErrorKind};

fn load_config(path: &str) -> Result<Config> {
    handle! {
        require !path.is_empty() else "path cannot be empty",
        try { read_file(path)? } with "reading config",
        then |data| { parse(&data)? } with "parsing"
            catch io::Error(e) when e.kind() == ErrorKind::NotFound {
                Config::default()  // Use defaults if not found
            }
            catch any ParseError(e) {
                log::warn!("Parse error: {e}");
                Config::default()
            }
    }
}

async fn fetch_data(url: &str) -> Result<Data> {
    handle! {
        async try { client.get(url).await? } with "fetching",
        then |resp| { resp.json().await? } with "parsing"
    }.await
}

fn conditional_load(use_cache: bool) -> Result<Data> {
    handle! {
        try when use_cache { load_from_cache()? }
        else { fetch_fresh()? }
        finally { cleanup(); }
    }
}
```

## Configuration

Open the color picker via:
- Command Palette: `Ctrl+Shift+P` → `handle-this: Configure Colors`
- Settings: Extensions → handle-this → "Configure Colors"

### Options
- **Theme Presets** - Quick color schemes matching popular editor themes
- **Rainbow Mode** - Colorful `handle!` macro name
- **Custom Colors** - Fine-tune every syntax element
- **Save Themes** - Save your customizations as named themes

## Links

- [handle-this on crates.io](https://crates.io/crates/handle-this)
- [handle-this documentation](https://docs.rs/handle-this)

## License

MIT
