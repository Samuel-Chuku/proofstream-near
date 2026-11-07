//! Reads `{owner, repo, base_sha, head_sha, milestone_text}` on stdin, asks a TEE-hosted
//! model whether the diff meets the milestone, and checks the model's signature here in
//! the enclave. Any failure prints `{"error": ...}` and no verdict: it can't release money.

// On the host, only the tests use these.
#![cfg_attr(not(target_os = "wasi"), allow(dead_code))]

mod rules;
mod verify;

#[cfg(target_os = "wasi")]
mod run;

#[cfg(target_os = "wasi")]
fn main() {
    use std::io::{Read, Write};

    let mut stdin = String::new();
    let output = match std::io::stdin().read_to_string(&mut stdin) {
        Ok(_) => run::run(&stdin).unwrap_or_else(|code| serde_json::json!({ "error": code })),
        Err(_) => serde_json::json!({ "error": "bad_stdin" }),
    };
    print!("{output}");
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
