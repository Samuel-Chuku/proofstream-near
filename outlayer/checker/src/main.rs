//! Reads `{owner, repo, pr_number, required_checks}` on stdin and prints the PR's
//! hard facts as compact JSON. Any failure prints `{"error": ...}`, so the contract
//! refuses by default. Response bodies are never echoed.

mod rules;

#[cfg(target_os = "wasi")]
mod github;

#[cfg(target_os = "wasi")]
fn main() {
    use std::io::{Read, Write};

    let mut stdin = String::new();
    let output = match std::io::stdin().read_to_string(&mut stdin) {
        Ok(_) => github::run(&stdin).unwrap_or_else(|code| serde_json::json!({ "error": code })),
        Err(_) => serde_json::json!({ "error": "bad_stdin" }),
    };
    print!("{output}");
    // OutLayer's docs warn that unflushed output can come back empty.
    if std::io::stdout().flush().is_err() {
        std::process::exit(1);
    }
}

// `cargo test` runs on the host, where the WASI HTTP client doesn't exist.
#[cfg(not(target_os = "wasi"))]
fn main() {
    eprintln!("build with --target wasm32-wasip2");
    std::process::exit(1);
}
