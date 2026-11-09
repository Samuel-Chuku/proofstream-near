//! Reads the contract-built review input on stdin, asks a TEE-hosted model how much of the
//! milestone the diff completes, and checks the model's signature here in the enclave. The
//! same build runs as the judge (GLM) or the verifier (Qwen), chosen by the OutLayer project
//! it runs in. The verifier signs its verdict with a key that exists only in its enclave;
//! the judge then buys that signed second opinion over OutLayer's HTTPS API and passes it
//! on (D48). Any failure prints `{"error": ...}` and no verdict: it can't release money.

// On the host, only the tests use these.
#![cfg_attr(not(target_os = "wasi"), allow(dead_code))]

mod rules;
mod verify;

/// The manifest, in the custom section the worker reads and the wasm hash covers. It
/// declares the verifier's signing key (D48); the judge's project derives its own,
/// unused, since the key belongs to the project.
#[cfg(target_arch = "wasm32")]
#[used]
#[link_section = "outlayer.manifest"]
static OUTLAYER_MANIFEST: [u8; include_bytes!("../manifest.json").len()] = *include_bytes!("../manifest.json");

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
