//! The network side: fetch the diff, call the model, fetch and check the signature.

use crate::rules::{self, Config, Input, MAX_DIFF_BYTES};
use crate::verify::{self, ResponseSignature};
use serde_json::{json, Value};
use std::time::{Duration, Instant};
use wasi::http::outgoing_handler;
use wasi::http::types::{Fields, Method, OutgoingBody, OutgoingRequest, RequestOptions, Scheme};
use wasi::io::streams::StreamError;
use wasi_http_client::Client;

const GITHUB_API: &str = "https://api.github.com";
const GITHUB_API_VERSION: &str = "2022-11-28";
const NEAR_AI_API: &str = "https://cloud-api.near.ai/v1";
const NEAR_AI_AUTHORITY: &str = "cloud-api.near.ai";
const CHAT_COMPLETIONS_PATH: &str = "/v1/chat/completions";
const READ_CHUNK_BYTES: u64 = 65_536;
const CONNECT_TIMEOUT_NANOS: u64 = 10_000_000_000;
/// Under the measured 85 s window, so a slow model fails with a code, not a lost result.
const FIRST_BYTE_TIMEOUT_NANOS: u64 = 70_000_000_000;
const CONNECT_TIMEOUT: Duration = Duration::from_secs(10);
const SIGNATURE_ATTEMPTS: u32 = 5;
const SIGNATURE_RETRY_DELAY: Duration = Duration::from_secs(2);

fn fetch_diff(cfg: &Config, token: &str, input: &Input, base_sha: &str, head_sha: &str) -> Result<Vec<u8>, &'static str> {
    let url = format!("{GITHUB_API}/repos/{}/{}/compare/{base_sha}...{head_sha}", input.owner, input.repo);
    let response = Client::new()
        .get(&url)
        .header("Authorization", format!("Bearer {token}"))
        .header("Accept", "application/vnd.github.diff")
        .header("X-GitHub-Api-Version", GITHUB_API_VERSION)
        .header("User-Agent", cfg.user_agent)
        .connect_timeout(CONNECT_TIMEOUT)
        .send()
        .map_err(|_| "github_unreachable")?;
    match response.status() {
        200 => response.body().map_err(|_| "github_bad_body"),
        // GitHub answers 406 when a diff is too big for it to render.
        406 => Err("too_large"),
        401 => Err("github_bad_token"),
        403 | 429 => Err("github_forbidden_or_rate_limited"),
        404 => Err("github_not_found"),
        _ => Err("github_unexpected_status"),
    }
}

/// Written on the raw WASI interface: `wasi-http-client` writes a body with one
/// `blocking-write-and-flush`, which traps above 4096 bytes, so a real diff can't use it.
fn post(cfg: &Config, url_path: &str, key: &str, body: &[u8]) -> Result<(u16, Vec<u8>), &'static str> {
    let headers = Fields::new();
    let authorization = format!("Bearer {key}");
    let length = body.len().to_string();
    for (name, value) in [
        ("authorization", authorization.as_str()),
        ("content-type", "application/json"),
        ("content-length", length.as_str()),
        // The signature covers uncompressed bytes.
        ("accept-encoding", "identity"),
        // Fails rather than rewriting to another model, which would change who signs.
        ("x-no-aliasing", "true"),
        ("user-agent", cfg.user_agent),
    ] {
        headers
            .append(&name.to_string(), &value.as_bytes().to_vec())
            .map_err(|_| "model_bad_header")?;
    }

    let request = OutgoingRequest::new(headers);
    let configured = request.set_method(&Method::Post).is_ok()
        && request.set_scheme(Some(&Scheme::Https)).is_ok()
        && request.set_authority(Some(NEAR_AI_AUTHORITY)).is_ok()
        && request.set_path_with_query(Some(url_path)).is_ok();
    if !configured {
        return Err("model_bad_request");
    }

    let outgoing = request.body().map_err(|_| "model_bad_request")?;
    {
        let stream = outgoing.write().map_err(|_| "model_write_failed")?;
        let ready = stream.subscribe();
        let mut rest = body;
        while !rest.is_empty() {
            ready.block();
            let allowed = stream.check_write().map_err(|_| "model_write_failed")? as usize;
            let count = allowed.min(rest.len());
            if count > 0 {
                stream.write(&rest[..count]).map_err(|_| "model_write_failed")?;
                rest = &rest[count..];
            }
        }
        stream.flush().map_err(|_| "model_write_failed")?;
        ready.block();
        stream.check_write().map_err(|_| "model_write_failed")?;
        drop(ready);
    }
    OutgoingBody::finish(outgoing, None).map_err(|_| "model_write_failed")?;

    let options = RequestOptions::new();
    let timeouts = options.set_connect_timeout(Some(CONNECT_TIMEOUT_NANOS)).is_ok()
        && options.set_first_byte_timeout(Some(FIRST_BYTE_TIMEOUT_NANOS)).is_ok();
    if !timeouts {
        return Err("model_bad_request");
    }

    let pending = outgoing_handler::handle(request, Some(options)).map_err(|_| "model_unreachable")?;
    pending.subscribe().block();
    let response = pending
        .get()
        .ok_or("model_unreachable")?
        .map_err(|_| "model_unreachable")?
        .map_err(|_| "model_unreachable")?;

    let status = response.status();
    let incoming = response.consume().map_err(|_| "model_bad_body")?;
    let mut bytes = Vec::new();
    {
        let stream = incoming.stream().map_err(|_| "model_bad_body")?;
        loop {
            match stream.blocking_read(READ_CHUNK_BYTES) {
                Ok(chunk) => bytes.extend_from_slice(&chunk),
                Err(StreamError::Closed) => break,
                Err(_) => return Err("model_bad_body"),
            }
        }
    }
    drop(incoming);
    Ok((status, bytes))
}

fn complete(cfg: &Config, key: &str, body: &[u8]) -> Result<Vec<u8>, &'static str> {
    match post(cfg, CHAT_COMPLETIONS_PATH, key, body)? {
        (200, bytes) => Ok(bytes),
        (401, _) => Err("model_bad_key"),
        (402, _) => Err("model_out_of_credit"),
        (429, _) => Err("model_rate_limited"),
        _ => Err("model_unexpected_status"),
    }
}

/// The signature can take a moment to be recorded, so a 404 is retried a bounded number of times.
fn fetch_signature(cfg: &Config, key: &str, chat_id: &str) -> Result<ResponseSignature, &'static str> {
    let url = format!("{NEAR_AI_API}/signature/{chat_id}?signing_algo=ecdsa");
    for attempt in 1..=SIGNATURE_ATTEMPTS {
        let response = Client::new()
            .get(&url)
            .header("Authorization", format!("Bearer {key}"))
            .header("Accept", "application/json")
            .header("User-Agent", cfg.user_agent)
            .connect_timeout(CONNECT_TIMEOUT)
            .send()
            .map_err(|_| "signature_unreachable")?;
        let status = response.status();
        if status == 200 {
            let body = response.body().map_err(|_| "signature_bad_body")?;
            // A 2xx with `error_code` means unavailable. It lacks these fields, so it fails here.
            if let Ok(signature) = serde_json::from_slice::<ResponseSignature>(&body) {
                return Ok(signature);
            }
        } else if status != 404 {
            return Err("signature_unexpected_status");
        }
        if attempt < SIGNATURE_ATTEMPTS {
            std::thread::sleep(SIGNATURE_RETRY_DELAY);
        }
    }
    Err("signature_unavailable")
}

#[cfg(feature = "tamper-test")]
fn tamper(response: Vec<u8>) -> Vec<u8> {
    const WANTED: &[u8] = b"does_not_meet";
    const FORGED: &[u8] = b"meets";
    match response.windows(WANTED.len()).position(|w| w == WANTED) {
        Some(at) => [&response[..at], FORGED, &response[at + WANTED.len()..]].concat(),
        None => {
            let mut changed = response;
            changed[0] ^= 1;
            changed
        }
    }
}

#[cfg(not(feature = "tamper-test"))]
fn tamper(response: Vec<u8>) -> Vec<u8> {
    response
}

pub fn run(stdin: &str) -> Result<Value, &'static str> {
    let input: Input = serde_json::from_str(stdin).map_err(|_| "bad_input")?;
    if !rules::valid_input(&input) {
        return Err("bad_input");
    }
    let cfg = rules::config_for_project(std::env::var("OUTLAYER_PROJECT_NAME").ok().as_deref())?;
    let github_token = std::env::var("GITHUB_TOKEN").map_err(|_| "missing_github_token")?;
    let near_ai_key = std::env::var("NEAR_AI_API_KEY").map_err(|_| "missing_near_ai_key")?;

    let started = Instant::now();
    let too_large = |bytes: Option<usize>| json!({ "status": "too_large", "role": cfg.role, "head_sha": input.head_sha, "diff_bytes": bytes });
    let diff = match fetch_diff(cfg, &github_token, &input, &input.base_sha, &input.head_sha) {
        Ok(diff) => diff,
        Err("too_large") => return Ok(too_large(None)),
        Err(code) => return Err(code),
    };
    // D45: earlier paid PRs, as context. Their size counts toward the same cap.
    let mut paid = Vec::new();
    for part in &input.prior {
        match fetch_diff(cfg, &github_token, &input, &part.base_sha, &part.head_sha) {
            Ok(bytes) => paid.extend_from_slice(&bytes),
            Err("too_large") => return Ok(too_large(None)),
            Err(code) => return Err(code),
        }
    }
    if paid.len() + diff.len() > MAX_DIFF_BYTES {
        return Ok(too_large(Some(paid.len() + diff.len())));
    }
    let paid_text = std::str::from_utf8(&paid).map_err(|_| "diff_not_utf8")?;
    let diff_text = std::str::from_utf8(&diff).map_err(|_| "diff_not_utf8")?;
    if !rules::diff_complete(diff_text, input.changed_files) {
        return Ok(json!({
            "status": "incomplete_diff",
            "role": cfg.role,
            "head_sha": input.head_sha,
            "diff_files": rules::diff_file_count(diff_text),
            "changed_files": input.changed_files,
        }));
    }
    let fetch_ms = started.elapsed().as_millis() as u64;

    let request = rules::request_body(cfg, &input, paid_text, diff_text);
    let model_started = Instant::now();
    let response = complete(cfg, &near_ai_key, &request)?;
    let model_ms = model_started.elapsed().as_millis() as u64;
    let chat_id = rules::chat_id(&response).ok_or("model_no_id")?;
    let signature_started = Instant::now();
    let signature = fetch_signature(cfg, &near_ai_key, &chat_id)?;
    let signature_ms = signature_started.elapsed().as_millis() as u64;

    let response = tamper(response);
    verify::check(&signature, cfg.model, &request, &response, cfg.signer).map_err(verify::Rejected::code)?;
    let answer = rules::read_answer(&response, &input.head_sha)?;

    Ok(rules::verified_output(&rules::Verified {
        cfg,
        head_sha: &input.head_sha,
        answer: &answer,
        chat_id: &chat_id,
        request_sha256: verify::sha256_hex(&request),
        response_sha256: verify::sha256_hex(&response),
        diff_bytes: paid.len() + diff.len(),
        ms: [fetch_ms, model_ms, signature_ms],
    }))
}
