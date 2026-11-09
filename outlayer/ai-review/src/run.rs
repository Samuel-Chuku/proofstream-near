//! The network side: fetch the diff, call the model, fetch and check the signature; the
//! verifier signs its verdict, and the judge buys that signed verdict (D48).

use crate::rules::{self, Answer, Config, Input, VerifierCall, MAX_DIFF_BYTES};
use crate::verify::{self, ResponseSignature};
use serde_json::{json, Value};
use std::time::Duration;
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
const OUTLAYER_TESTNET: &str = "testnet-api.outlayer.ai";
const OUTLAYER_MAINNET: &str = "api.outlayer.ai";
/// The verifier's secret row for calls from the judge (D48).
const VERIFIER_PROFILE: &str = "default";
/// The verifier's run takes ~5 s (measured); this leaves the judge's 60 s run room to answer.
const VERIFIER_FIRST_BYTE_TIMEOUT_NANOS: u64 = 35_000_000_000;
const SIGNING_KEY: &str = "verdict";

mod host {
    wit_bindgen::generate!({ world: "signing-keys-host", path: "wit" });
}
use host::outlayer::signing_keys::api as signing_keys;

/// Error codes for one kind of outbound POST, so a failure says which call it was.
struct Codes {
    bad_header: &'static str,
    bad_request: &'static str,
    write_failed: &'static str,
    unreachable: &'static str,
    bad_body: &'static str,
}

const MODEL_CODES: Codes = Codes {
    bad_header: "model_bad_header",
    bad_request: "model_bad_request",
    write_failed: "model_write_failed",
    unreachable: "model_unreachable",
    bad_body: "model_bad_body",
};

const VERIFIER_CODES: Codes = Codes {
    bad_header: "verifier_bad_header",
    bad_request: "verifier_bad_request",
    write_failed: "verifier_write_failed",
    unreachable: "verifier_unreachable",
    bad_body: "verifier_bad_body",
};

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
fn post(
    authority: &str,
    url_path: &str,
    headers_in: &[(&str, &str)],
    body: &[u8],
    first_byte_nanos: u64,
    codes: &Codes,
) -> Result<(u16, Vec<u8>), &'static str> {
    let headers = Fields::new();
    let length = body.len().to_string();
    for (name, value) in headers_in.iter().copied().chain([("content-type", "application/json"), ("content-length", length.as_str())]) {
        headers.append(&name.to_string(), &value.as_bytes().to_vec()).map_err(|_| codes.bad_header)?;
    }

    let request = OutgoingRequest::new(headers);
    let configured = request.set_method(&Method::Post).is_ok()
        && request.set_scheme(Some(&Scheme::Https)).is_ok()
        && request.set_authority(Some(authority)).is_ok()
        && request.set_path_with_query(Some(url_path)).is_ok();
    if !configured {
        return Err(codes.bad_request);
    }

    let outgoing = request.body().map_err(|_| codes.bad_request)?;
    {
        let stream = outgoing.write().map_err(|_| codes.write_failed)?;
        let ready = stream.subscribe();
        let mut rest = body;
        while !rest.is_empty() {
            ready.block();
            let allowed = stream.check_write().map_err(|_| codes.write_failed)? as usize;
            let count = allowed.min(rest.len());
            if count > 0 {
                stream.write(&rest[..count]).map_err(|_| codes.write_failed)?;
                rest = &rest[count..];
            }
        }
        stream.flush().map_err(|_| codes.write_failed)?;
        ready.block();
        stream.check_write().map_err(|_| codes.write_failed)?;
        drop(ready);
    }
    OutgoingBody::finish(outgoing, None).map_err(|_| codes.write_failed)?;

    let options = RequestOptions::new();
    let timeouts = options.set_connect_timeout(Some(CONNECT_TIMEOUT_NANOS)).is_ok()
        && options.set_first_byte_timeout(Some(first_byte_nanos)).is_ok();
    if !timeouts {
        return Err(codes.bad_request);
    }

    let pending = outgoing_handler::handle(request, Some(options)).map_err(|_| codes.unreachable)?;
    pending.subscribe().block();
    let response = pending
        .get()
        .ok_or(codes.unreachable)?
        .map_err(|_| codes.unreachable)?
        .map_err(|_| codes.unreachable)?;

    let status = response.status();
    let incoming = response.consume().map_err(|_| codes.bad_body)?;
    let mut bytes = Vec::new();
    {
        let stream = incoming.stream().map_err(|_| codes.bad_body)?;
        loop {
            match stream.blocking_read(READ_CHUNK_BYTES) {
                Ok(chunk) => bytes.extend_from_slice(&chunk),
                Err(StreamError::Closed) => break,
                Err(_) => return Err(codes.bad_body),
            }
        }
    }
    drop(incoming);
    Ok((status, bytes))
}

fn complete(cfg: &Config, key: &str, body: &[u8]) -> Result<Vec<u8>, &'static str> {
    let authorization = format!("Bearer {key}");
    let headers = [
        ("authorization", authorization.as_str()),
        // The signature covers uncompressed bytes.
        ("accept-encoding", "identity"),
        // Fails rather than rewriting to another model, which would change who signs.
        ("x-no-aliasing", "true"),
        ("user-agent", cfg.user_agent),
    ];
    match post(NEAR_AI_AUTHORITY, CHAT_COMPLETIONS_PATH, &headers, body, FIRST_BYTE_TIMEOUT_NANOS, &MODEL_CODES)? {
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

/// What this role's own review found, before any signing or second opinion.
enum Own {
    /// Decided before any model call: `too_large` or `incomplete_diff`, with its output.
    Early(&'static str, Value),
    Verified(Answer, String),
}

fn own_review(cfg: &Config, input: &Input, github_token: &str, near_ai_key: &str) -> Result<Own, &'static str> {
    let too_large = |bytes: Option<usize>| {
        Own::Early("too_large", json!({ "status": "too_large", "role": cfg.role, "head_sha": input.head_sha, "diff_bytes": bytes }))
    };
    let diff = match fetch_diff(cfg, github_token, input, &input.base_sha, &input.head_sha) {
        Ok(diff) => diff,
        Err("too_large") => return Ok(too_large(None)),
        Err(code) => return Err(code),
    };
    // D45: earlier paid PRs, as context. Their size counts toward the same cap.
    let mut paid = Vec::new();
    for part in &input.prior {
        match fetch_diff(cfg, github_token, input, &part.base_sha, &part.head_sha) {
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
        return Ok(Own::Early(
            "incomplete_diff",
            json!({
                "status": "incomplete_diff",
                "role": cfg.role,
                "head_sha": input.head_sha,
                "diff_files": rules::diff_file_count(diff_text),
                "changed_files": input.changed_files,
            }),
        ));
    }

    let request = rules::request_body(cfg, input, paid_text, diff_text);
    let response = complete(cfg, near_ai_key, &request)?;
    let chat_id = rules::chat_id(&response).ok_or("model_no_id")?;
    let signature = fetch_signature(cfg, near_ai_key, &chat_id)?;
    let response = tamper(response);
    verify::check(&signature, cfg.model, &request, &response, cfg.signer).map_err(verify::Rejected::code)?;
    let answer = rules::read_answer(&response, &input.head_sha)?;
    Ok(Own::Verified(answer, chat_id))
}

/// The verifier signs exactly what it decided, for exactly this review, naming the account
/// that paid for the run as the host reports it (D48).
/// Also returns the key's public half: in this run's attested output, it proves the key
/// was derived in the enclave for this project, which is how the milestone pin is checked.
fn sign_verdict(input: &Input, verdict: &str, percent: u8) -> Result<(String, String, String), &'static str> {
    let caller = std::env::var("NEAR_USER_ACCOUNT_ID").map_err(|_| "no_caller")?;
    let message = rules::verdict_message(&input.review, &input.head_sha, verdict, percent, &caller);
    let signature = signing_keys::sign(SIGNING_KEY, None, message.as_bytes()).map_err(|_| "signing_key_unavailable")?;
    let public_key = signing_keys::public_key(SIGNING_KEY, None).map_err(|_| "signing_key_unavailable")?;
    let hex = |b: &[u8]| b.iter().map(|x| format!("{x:02x}")).collect::<String>();
    Ok((caller, hex(&signature), hex(&public_key)))
}

/// The judge's second opinion: the pinned verifier version, over OutLayer's HTTPS API, paid
/// from the judge owner's payment key. Any failure is a reason code, never a verdict.
fn ask_verifier(stdin: &Value, input: &Input, call: &VerifierCall) -> Value {
    let Ok(payment_key) = std::env::var("PAYMENT_KEY") else { return json!({ "error": "missing_payment_key" }) };
    let Some((owner, name)) = call.project_id.split_once('/') else { return json!({ "error": "verifier_bad_project" }) };
    let authority = match std::env::var("NEAR_NETWORK_ID").as_deref() {
        Ok("mainnet") => OUTLAYER_MAINNET,
        _ => OUTLAYER_TESTNET,
    };
    // The verifier gets the same contract-built input, minus the part that is the judge's.
    let mut forwarded = stdin.clone();
    if let Some(object) = forwarded.as_object_mut() {
        object.remove("verifier");
    }
    let body = json!({
        "input": forwarded,
        "secrets_ref": { "profile": VERIFIER_PROFILE, "account_id": owner },
        "version_key": call.version_key,
    })
    .to_string();
    let fee = call.fee.to_string();
    let headers = [("x-payment-key", payment_key.as_str()), ("x-attached-deposit", fee.as_str()), ("user-agent", JUDGE_AGENT)];
    let path = format!("/call/{owner}/{name}");
    match post(authority, &path, &headers, body.as_bytes(), VERIFIER_FIRST_BYTE_TIMEOUT_NANOS, &VERIFIER_CODES) {
        Ok((status, reply)) => rules::read_verifier_reply(status, &reply, &input.head_sha),
        Err(code) => json!({ "error": code }),
    }
}

const JUDGE_AGENT: &str = "proofstream-judge";

pub fn run(stdin: &str) -> Result<Value, &'static str> {
    let raw: Value = serde_json::from_str(stdin).map_err(|_| "bad_input")?;
    let input: Input = serde_json::from_value(raw.clone()).map_err(|_| "bad_input")?;
    if !rules::valid_input(&input) {
        return Err("bad_input");
    }
    let cfg = rules::config_for_project(std::env::var("OUTLAYER_PROJECT_NAME").ok().as_deref())?;
    if !rules::role_fits_input(cfg, &input) {
        return Err("bad_input");
    }
    let github_token = std::env::var("GITHUB_TOKEN").map_err(|_| "missing_github_token")?;
    let near_ai_key = std::env::var("NEAR_AI_API_KEY").map_err(|_| "missing_near_ai_key")?;
    let own = own_review(cfg, &input, &github_token, &near_ai_key)?;

    if cfg.role == "verifier" {
        let (verdict, percent, mut out) = match &own {
            Own::Early(status, out) => {
                let (v, p) = rules::signed_verdict(None, status);
                (v, p, out.clone())
            }
            Own::Verified(answer, chat_id) => {
                let (v, p) = rules::signed_verdict(Some(answer), "");
                (v, p, rules::own_output(cfg, &input.head_sha, answer, chat_id))
            }
        };
        let (caller, signature, public_key) = sign_verdict(&input, verdict, percent)?;
        out["verdict"] = json!(verdict);
        out["percent_done"] = json!(percent);
        out["caller"] = json!(caller);
        out["signature"] = json!(signature);
        out["public_key"] = json!(public_key);
        return Ok(out);
    }

    // The judge: its own verdict is fixed before it asks. An objection or an early status
    // holds the milestone whatever the verifier says, so no fee is spent on asking.
    match own {
        Own::Early(_, out) => Ok(out),
        Own::Verified(answer, chat_id) => {
            let mut out = rules::own_output(cfg, &input.head_sha, &answer, &chat_id);
            let progress = matches!(answer.verdict, rules::Verdict::Meets | rules::Verdict::Partial);
            out["verifier"] = match (&input.verifier, progress) {
                (Some(call), true) => ask_verifier(&raw, &input, call),
                _ => Value::Null,
            };
            Ok(out)
        }
    }
}
