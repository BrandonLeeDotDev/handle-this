//! Example: Using handle-this with thiserror
//!
//! This shows how handle-this integrates naturally with thiserror-defined errors.

use handle_this::{handle, Result};
use thiserror::Error;

// Define your domain errors with thiserror
#[derive(Error, Debug)]
pub enum ConfigError {
    #[error("Config file not found: {path}")]
    NotFound { path: String },

    #[error("Invalid config format: {0}")]
    ParseError(String),

    #[error("Missing required field: {0}")]
    MissingField(String),
}

#[derive(Error, Debug)]
pub enum DbError {
    #[error("Connection failed: {0}")]
    Connection(String),

    #[error("Query failed: {0}")]
    Query(String),
}

#[derive(Error, Debug)]
pub enum AppError {
    #[error(transparent)]
    Config(#[from] ConfigError),

    #[error(transparent)]
    Database(#[from] DbError),

    #[error("Unexpected: {0}")]
    Internal(String),
}

// Simulated operations that return thiserror types
fn read_config(path: &str) -> std::result::Result<String, ConfigError> {
    if path == "missing.toml" {
        Err(ConfigError::NotFound { path: path.into() })
    } else if path == "invalid.toml" {
        Err(ConfigError::ParseError("unexpected token".into()))
    } else {
        Ok("config_data".into())
    }
}

fn connect_db(url: &str) -> std::result::Result<(), DbError> {
    if url.contains("bad") {
        Err(DbError::Connection("refused".into()))
    } else {
        Ok(())
    }
}

fn query_user(id: u32) -> std::result::Result<String, DbError> {
    if id == 0 {
        Err(DbError::Query("user not found".into()))
    } else {
        Ok(format!("user_{}", id))
    }
}

// Example 1: Simple typed catch
fn load_config_with_default(path: &str) -> String {
    handle! {
        try -> String { read_config(path)? }
        catch ConfigError(e) when matches!(e, ConfigError::NotFound { .. }) {
            "default_config".into()
        }
        else { "fallback".into() }
    }
}

// Example 2: Transform errors with context
fn init_database(url: &str) -> Result<()> {
    handle! {
        try { connect_db(url)? }
        throw DbError(e) {
            format!("Failed to initialize database at {}: {}", url, e)
        }
        with "database initialization"
    }
}

// Example 3: Multiple error types with different handlers
fn get_user_or_default(config_path: &str, db_url: &str, user_id: u32) -> String {
    handle! {
        try -> String {
            let _ = read_config(config_path)?;
            connect_db(db_url)?;
            query_user(user_id)?
        }
        // Config errors: use default config
        catch ConfigError(_) { "anonymous".into() }
        // DB connection errors: log and use guest
        inspect DbError(e) when matches!(e, DbError::Connection(_)) {
            eprintln!("DB unavailable: {}", e);
        }
        catch DbError(e) when matches!(e, DbError::Connection(_)) {
            "guest".into()
        }
        // DB query errors: user not found
        catch DbError(_) { "unknown_user".into() }
        // Everything else
        catch { "error".into() }
    }
}

// Example 4: Using AppError wrapper with chain_any
fn process_request(config: &str, db: &str, user_id: u32) -> Result<String> {
    handle! {
        try {
            let cfg = read_config(config).map_err(AppError::from)?;
            connect_db(db).map_err(AppError::from)?;
            let user = query_user(user_id).map_err(AppError::from)?;
            format!("{}:{}", cfg, user)
        }
        with { msg: "processing request", user_id: user_id }
    }
}

fn main() {
    println!("=== thiserror + handle-this examples ===\n");

    // Example 1: Default on missing config
    println!("1. Load missing config:");
    let cfg = load_config_with_default("missing.toml");
    println!("   Result: {}\n", cfg);

    // Example 2: Transform DB error
    println!("2. Init bad database:");
    let result = init_database("bad://localhost");
    if let Err(e) = result {
        println!("   Error: {}", e.message());
        println!("   Trace depth: {}\n", e.depth());
    }

    // Example 3: Multiple handlers
    println!("3. Get user with various failures:");
    println!("   Missing config: {}", get_user_or_default("missing.toml", "good://db", 1));
    println!("   Bad DB: {}", get_user_or_default("config.toml", "bad://db", 1));
    println!("   Missing user: {}", get_user_or_default("config.toml", "good://db", 0));
    println!("   Success: {}\n", get_user_or_default("config.toml", "good://db", 42));

    // Example 4: Chain traversal
    println!("4. Process request with error chain:");
    let result = process_request("config.toml", "good://db", 0);
    if let Err(e) = result {
        println!("   Message: {}", e.message());

        // chain_any finds errors wrapped via #[source] or Error::source()
        // AppError wraps DbError via #[from], so we can find it:
        if let Some(app_err) = e.chain_any::<AppError>() {
            println!("   Found AppError: {}", app_err);
        }
    }

    // Example 5: Direct error propagation (chain_any works better)
    println!("\n5. Direct error with chain_any:");
    fn direct_db_error() -> Result<String> {
        handle! {
            try {
                // Propagate DbError directly
                query_user(0)?
            }
        }
    }

    if let Err(e) = direct_db_error() {
        println!("   Message: {}", e.message());
        if let Some(db_err) = e.chain_any::<DbError>() {
            println!("   Found DbError via chain_any: {}", db_err);
        }
    }
}
