//! Test nested try blocks with signal mode (break/continue in fallible mode)

use handle_this::{handle, Result};

fn main() {
    println!("=== Nested Signal Mode Tests ===\n");

    // Test 1: Nested try in loop with break/continue
    println!("1. Nested try with break:");
    let mut outer_count = 0;
    for i in 0..5 {
        outer_count += 1;
        let result: Result<i32> = handle! {
            try {
                try {
                    if i < 3 {
                        Err(std::io::Error::other("inner fail"))?
                    }
                    i * 10
                }
                with "inner"
            }
            catch _ {
                if i >= 2 {
                    break
                }
                continue
            }
            with "outer"
        };
        println!("   i={}: {:?}", i, result);
    }
    println!("   outer_count = {} (expected 3: 0, 1, 2)\n", outer_count);

    // Test 2: Deeply nested with context at each level
    println!("2. Deeply nested (3 levels):");
    let result: Result<i32> = handle! {
        try {
            try {
                try {
                    Err::<i32, _>(std::io::Error::other("deep"))?
                }
                with "level 2"
            }
            with "level 1"
        }
        with "level 0"
    };
    match &result {
        Ok(v) => println!("   Ok({})", v),
        Err(e) => {
            println!("   Err: {}", e.message());
            println!("   Trace depth: {}", e.depth());
        }
    }
    println!();

    // Test 3: Control flow in nested catch
    println!("3. Control flow in nested catch:");
    let mut count = 0;
    for i in 0..5 {
        count += 1;
        let result: Result<i32> = handle! {
            try {
                try {
                    if i < 3 {
                        Err(std::io::Error::other("inner"))?
                    }
                    i * 10
                }
                catch _ {
                    if i >= 2 {
                        break  // break from inner catch
                    }
                    continue  // continue from inner catch
                }
            }
            with "outer"
        };
        println!("   i={}: {:?}", i, result);
    }
    println!("   count = {} (expected 3: 0, 1, 2)\n", count);

    // Test 4: Return from function via signal mode
    println!("4. Return value test:");
    fn compute() -> Result<i32> {
        for i in 0..10 {
            let result: Result<i32> = handle! {
                try {
                    if i < 5 {
                        Err(std::io::Error::other("not yet"))?
                    }
                    i * 100
                }
                catch _ { continue }
            };
            return result;
        }
        Ok(-1)
    }
    println!("   compute() = {:?}", compute());
    println!();

    // Test 5: Mixed typed catches with break
    println!("5. Mixed typed catches:");
    for i in 0..5 {
        let result: Result<i32> = handle! {
            try {
                match i {
                    0 => Err(std::io::Error::other("io"))?,
                    1 => Err("parse".parse::<i32>().unwrap_err())?,
                    _ => i,
                }
            }
            catch std::io::Error(_) {
                println!("   i={}: caught io::Error, continuing", i);
                continue
            }
            catch std::num::ParseIntError(_) {
                println!("   i={}: caught ParseIntError, breaking", i);
                break
            }
        };
        println!("   i={}: result = {:?}", i, result);
    }

    // Test 6: Control flow in nested throw
    println!("6. Control flow in nested throw:");
    let mut count = 0;
    for i in 0..5 {
        count += 1;
        let result: Result<i32> = handle! {
            try {
                try {
                    Err::<i32, _>(std::io::Error::other("inner"))?
                }
                throw _ {
                    if i >= 2 {
                        break
                    }
                    continue
                }
            }
            catch _ { -999 }  // shouldn't reach - throw has control flow
        };
        println!("   i={}: {:?}", i, result);
    }
    println!("   count = {} (expected 3)\n", count);

    // Test 7: Nested try-catch-throw chain
    println!("7. Nested try-catch-throw chain:");
    for i in 0..6 {
        let result: Result<i32> = handle! {
            try {
                try {
                    try {
                        match i {
                            0 => Err(std::io::Error::other("deep"))?,
                            1 => Err("x".parse::<i32>().unwrap_err())?,
                            _ => i * 10,
                        }
                    }
                    throw std::io::Error(_) { continue }  // io errors continue
                }
                catch std::num::ParseIntError(_) { break }  // parse errors break
            }
            with "outer"
        };
        println!("   i={}: {:?}", i, result);
    }
    println!();

    // Test 8: Control flow in inspect (inspect runs, then propagates)
    println!("8. Control flow after inspect:");
    let mut inspected = Vec::new();
    for i in 0..4 {
        let result: Result<i32> = handle! {
            try {
                try {
                    if i < 2 {
                        Err(std::io::Error::other("fail"))?
                    }
                    i * 10
                }
                inspect e { inspected.push(i); }
            }
            catch _ { continue }
        };
        println!("   i={}: {:?}", i, result);
    }
    println!("   inspected: {:?} (expected [0, 1])\n", inspected);

    // Test 9: Multiple nested levels with mixed handlers
    println!("9. Triple-nested with mixed handlers:");
    let mut trace = Vec::new();
    for i in 0..5 {
        trace.push(format!("start-{}", i));
        let result: Result<i32> = handle! {
            try {
                try {
                    try {
                        if i == 0 { Err(std::io::Error::other("L3"))? }
                        if i == 1 { Err("x".parse::<i32>().unwrap_err())? }
                        if i == 2 { Err(std::io::Error::other("L3-break"))? }
                        i * 100
                    }
                    throw std::io::Error(e) {
                        trace.push(format!("throw-{}", i));
                        std::io::Error::other(format!("wrapped: {}", e))
                    }
                }
                catch std::num::ParseIntError(_) {
                    trace.push(format!("catch-parse-{}", i));
                    continue
                }
            }
            catch std::io::Error(_e) {
                trace.push(format!("catch-io-{}", i));
                if i >= 2 { break }
                continue
            }
        };
        trace.push(format!("result-{}: {:?}", i, result));
    }
    println!("   trace: {:?}\n", trace);

    // Test 10: Deeply nested with finally
    println!("10. Nested with finally:");
    let mut cleanup = Vec::new();
    for i in 0..3 {
        let result: Result<i32> = handle! {
            try {
                try {
                    if i < 2 {
                        Err(std::io::Error::other("fail"))?
                    }
                    i * 10
                }
                finally { cleanup.push(format!("inner-{}", i)); }
            }
            catch _ { continue }
            finally { cleanup.push(format!("outer-{}", i)); }
        };
        println!("   i={}: {:?}", i, result);
    }
    println!("   cleanup: {:?}\n", cleanup);

    println!("=== All tests complete ===");
}
