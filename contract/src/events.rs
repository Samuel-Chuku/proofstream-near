//! NEP-297 events. The front end's log and EVIDENCE are both built from these.

use near_sdk::env;
use near_sdk::serde_json::{json, Value};

const STANDARD: &str = "proofstream";
const VERSION: &str = "1.0.0";

pub fn emit(event: &str, data: Value) {
    let body = json!({ "standard": STANDARD, "version": VERSION, "event": event, "data": [data] });
    env::log_str(&format!("EVENT_JSON:{body}"));
}
