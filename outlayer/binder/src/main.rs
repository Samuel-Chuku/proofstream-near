//! Reads `{gist_id, contract, milestone_id, nonce}` on stdin. A public gist naming that
//! contract, milestone and nonce proves its owner's GitHub id chose the payout `account`.
//! Any failure prints `{"error": ...}`, so the contract binds nothing.

// On the host, only the tests use these.
#![cfg_attr(not(target_os = "wasi"), allow(dead_code))]

mod rules;

#[cfg(target_os = "wasi")]
fn run(stdin: &str) -> Result<serde_json::Value, &'static str> {
    use std::time::Duration;

    let input: rules::Input = serde_json::from_str(stdin).map_err(|_| "bad_input")?;
    if !rules::valid_gist_id(&input.gist_id) {
        return Err("bad_input");
    }
    let token = std::env::var("GITHUB_TOKEN").map_err(|_| "missing_github_token")?;
    let response = wasi_http_client::Client::new()
        .get(&format!("https://api.github.com/gists/{}", input.gist_id))
        .header("Authorization", format!("Bearer {token}"))
        .header("Accept", "application/vnd.github+json")
        .header("X-GitHub-Api-Version", "2022-11-28")
        .header("User-Agent", "proofstream-binder")
        .connect_timeout(Duration::from_secs(10))
        .send()
        .map_err(|_| "github_unreachable")?;
    let body = match response.status() {
        200 => response.body().map_err(|_| "github_bad_body")?,
        401 => return Err("github_bad_token"),
        403 | 429 => return Err("github_forbidden_or_rate_limited"),
        404 => return Err("gist_not_found"),
        _ => return Err("github_unexpected_status"),
    };
    let proof = rules::read_proof(&body, &input)?;
    serde_json::to_value(proof).map_err(|_| "encode_failed")
}

#[cfg(target_os = "wasi")]
fn main() {
    use std::io::{Read, Write};

    let mut stdin = String::new();
    let output = match std::io::stdin().read_to_string(&mut stdin) {
        Ok(_) => run(&stdin).unwrap_or_else(|code| serde_json::json!({ "error": code })),
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
