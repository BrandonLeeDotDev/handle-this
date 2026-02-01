//! Show nested scope structure as JSON
//!
//! Run with: cargo run --example scope_json --features serde

use handle_this::{handle, Handled, Result, Value};

/// Format a Value as JSON (preserving types)
fn value_to_json(v: &Value) -> String {
    match v {
        Value::String(s) => format!(r#""{}""#, s),
        Value::Int(n) => n.to_string(),
        Value::Uint(n) => n.to_string(),
        Value::Float(n) => n.to_string(),
        Value::Bool(b) => b.to_string(),
        Value::Null => "null".to_string(),
    }
}

fn to_json(e: &Handled) -> String {
    // Manual JSON building since we can't easily serialize Handled<Error>
    let mut frames = Vec::new();
    for frame in e.frames() {
        let mut f = format!(
            r#"{{ "file": "{}", "line": {}, "col": {}"#,
            frame.file, frame.line, frame.col
        );
        if let Some(ctx) = frame.context {
            f.push_str(&format!(r#", "message": "{}""#, ctx));
        }
        let atts: Vec<String> = frame.attachments()
            .map(|(k, v)| format!(r#""{}": {}"#, k, value_to_json(v)))
            .collect();
        if !atts.is_empty() {
            f.push_str(&format!(r#", "attachments": {{ {} }}"#, atts.join(", ")));
        }
        f.push_str(" }");
        frames.push(f);
    }
    format!(
        r#"{{
  "message": "{}",
  "trace": [
    {}
  ]
}}"#,
        e.message(),
        frames.join(",\n    ")
    )
}

fn main() {
    println!("=== Nested Scope Structure as JSON ===\n");

    // Test 1: Single scope with message
    println!("Test 1: Single scope with message");
    let result: Result<i32> = handle! {
        scope "auth_flow",
        try {
            Err(Handled::from(std::io::Error::other("invalid token")))?
        }
        catch e {
            println!("{}\n", to_json(&e));
            42
        }
    };
    println!("Result: {:?}\n", result);

    // Test 2: Scope with key-value data
    println!("Test 2: Scope with key-value data");
    let result: Result<i32> = handle! {
        scope "api_call", { endpoint: "/users", method: "GET" },
        try {
            Err(Handled::from(std::io::Error::other("rate limited")))?
        }
        catch e {
            println!("{}\n", to_json(&e));
            42
        }
    };
    println!("Result: {:?}\n", result);

    // Test 3: Nested scope in try body (direct, not separate handle!)
    println!("Test 3: Nested scope in try body");
    let result: Result<i32> = handle! {
        scope "level1",
        try {
            scope "level2", try {
                Err(Handled::from(std::io::Error::other("deep error")))?
            }
            catch e {
                println!("Level 2 caught:");
                println!("{}\n", to_json(&e));
                42
            }
        }
        catch e {
            println!("Level 1 caught (won't reach):");
            println!("{}\n", to_json(&e));
            99
        }
    };
    println!("Result: {:?}\n", result);

    // Test 4: Nested scopes with separate handle! calls (shows propagation)
    println!("Test 4: Nested handle! calls showing scope accumulation");
    let result: Result<i32> = handle! {
        scope "outer", { outer_data: 1 },
        try {
            let inner_result: Result<i32> = handle! {
                scope "inner", { inner_data: 2 },
                try {
                    Err(Handled::from(std::io::Error::other("inner failure")))?
                }
                try catch e {
                    println!("Inner scope caught:");
                    println!("{}\n", to_json(&e));
                    Err(e)  // Propagate to outer scope
                }
            };
            inner_result?
        }
        try catch e {
            // This receives error if inner propagates
            println!("Outer scope caught (after propagation):");
            println!("{}\n", to_json(&e));
            Ok(0)
        }
    };
    println!("Result: {:?}\n", result);

    // Test 5: Triple nesting with inline scope syntax (no explicit handle! wrappers needed)
    println!("Test 5: Triple nesting demonstration");
    let result: Result<i32> = handle! {
        scope "http_request", { url: "https://api.example.com" },
        try {
            scope "auth", { token_type: "Bearer" },
            try {
                scope "token_validation",
                try {
                    Err(Handled::from(std::io::Error::other("token expired")))?
                }
                catch e {
                    println!("Token validation failed:");
                    println!("{}\n", to_json(&e));
                    42  // Return default
                }
            }
            catch e {
                println!("Auth failed (won't reach):");
                println!("{}", to_json(&e));
                99
            }
        }
        catch e {
            println!("Request failed (won't reach):");
            println!("{}", to_json(&e));
            0
        }
    };
    println!("Result: {:?}\n", result);

    // Test 6: Full propagation through all nested scopes
    println!("Test 6: Error propagating through all scopes");
    let result: Result<i32> = handle! {
        scope "http_request", { url: "https://api.example.com", method: "POST" },
        try {
            scope "auth", { token_type: "Bearer", user_id: 42 },
            try {
                scope "token_validation", { cache_hit: false },
                try {
                    Err(Handled::from(std::io::Error::other("token expired")))?
                }
                // No catch here - error propagates up
            }
            // No catch here - error propagates up
        }
        catch e {
            // Catch at outermost level - see full trace
            println!("{}\n", to_json(&e));
            0
        }
    };
    println!("Result: {:?}\n", result);
}
