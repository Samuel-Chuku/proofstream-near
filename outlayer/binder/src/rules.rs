//! What a gist must contain to prove who chose a payout account. No I/O.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;

pub const PROOF_FILE: &str = "proofstream-bind.json";
const MAX_GIST_ID_LEN: usize = 64;
const GIT_SHA_LEN: usize = 40;

#[derive(Deserialize, Debug)]
pub struct Input {
    pub gist_id: String,
    pub contract: String,
    pub milestone_id: u64,
    pub nonce: String,
}

#[derive(Deserialize)]
struct Gist {
    public: bool,
    owner: Option<Owner>,
    files: HashMap<String, GistFile>,
    history: Vec<Revision>,
}

#[derive(Deserialize)]
struct Owner {
    id: u64,
}

#[derive(Deserialize)]
struct GistFile {
    content: Option<String>,
    truncated: Option<bool>,
}

#[derive(Deserialize)]
struct Revision {
    version: String,
}

#[derive(Deserialize)]
struct ProofFile {
    contract: String,
    milestone_id: u64,
    nonce: String,
    account: String,
}

/// The contract compares `owner_id` to the milestone's GitHub id and validates `account`.
/// `revision` is the gist commit whose content was read; the contract stores it with the
/// binding, so a later edit of the gist can't change what the record says it chose.
#[derive(Serialize, Debug, PartialEq)]
pub struct Proof {
    pub gist_id: String,
    pub revision: String,
    pub owner_id: u64,
    pub contract: String,
    pub milestone_id: u64,
    pub nonce: String,
    pub account: String,
}

/// It goes into a URL sent with our token.
pub fn valid_gist_id(id: &str) -> bool {
    !id.is_empty() && id.len() <= MAX_GIST_ID_LEN && id.chars().all(|c| c.is_ascii_hexdigit())
}

fn is_git_sha(s: &str) -> bool {
    s.len() == GIT_SHA_LEN && s.chars().all(|c| c.is_ascii_digit() || ('a'..='f').contains(&c))
}

pub fn read_proof(gist_json: &[u8], input: &Input) -> Result<Proof, &'static str> {
    let gist: Gist = serde_json::from_slice(gist_json).map_err(|_| "gist_bad_json")?;
    if !gist.public {
        return Err("gist_not_public");
    }
    let owner = gist.owner.ok_or("gist_no_owner")?;
    let file = gist.files.get(PROOF_FILE).ok_or("proof_file_missing")?;
    if file.truncated != Some(false) {
        return Err("proof_file_truncated");
    }
    let content = file.content.as_deref().ok_or("proof_file_empty")?;
    let proof: ProofFile = serde_json::from_str(content).map_err(|_| "proof_file_bad_json")?;
    if proof.contract != input.contract {
        return Err("wrong_contract");
    }
    if proof.milestone_id != input.milestone_id {
        return Err("wrong_milestone");
    }
    if proof.nonce != input.nonce {
        return Err("wrong_nonce");
    }
    // GitHub lists history newest first, from the same response as `files`, so the first
    // entry is the revision of the content above.
    let revision = gist.history.first().ok_or("gist_no_history")?.version.clone();
    if !is_git_sha(&revision) {
        return Err("gist_bad_revision");
    }
    Ok(Proof {
        gist_id: input.gist_id.clone(),
        revision,
        owner_id: owner.id,
        contract: proof.contract,
        milestone_id: proof.milestone_id,
        nonce: proof.nonce,
        account: proof.account,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::{json, Value};

    const NONCE: &str = "abc123";
    const NEWEST: &str = "2222222222222222222222222222222222222222";
    const OLDER: &str = "1111111111111111111111111111111111111111";

    fn input() -> Input {
        Input { gist_id: "6cad32".into(), contract: "c.testnet".into(), milestone_id: 7, nonce: NONCE.into() }
    }

    fn file(contract: &str, milestone: u64, nonce: &str) -> String {
        json!({ "contract": contract, "milestone_id": milestone, "nonce": nonce, "account": "me.testnet" }).to_string()
    }

    fn gist(content: Value) -> Value {
        json!({
            "public": true,
            "owner": { "id": 42 },
            "files": { PROOF_FILE: { "content": content, "truncated": false } },
            "history": [{ "version": NEWEST }, { "version": OLDER }],
        })
    }

    fn read(g: Value) -> Result<Proof, &'static str> {
        read_proof(g.to_string().as_bytes(), &input())
    }

    #[test]
    fn valid_gist_gives_the_owner_and_account() {
        let p = read(gist(json!(file("c.testnet", 7, NONCE)))).unwrap();
        assert_eq!((p.owner_id, p.account.as_str()), (42, "me.testnet"));
    }

    #[test]
    fn revision_is_the_newest_history_entry() {
        let p = read(gist(json!(file("c.testnet", 7, NONCE)))).unwrap();
        assert_eq!(p.revision, NEWEST);
    }

    #[test]
    fn output_names_the_field_the_contract_reads() {
        let out = serde_json::to_value(read(gist(json!(file("c.testnet", 7, NONCE)))).unwrap()).unwrap();
        assert_eq!(out["revision"], NEWEST);
        assert!(out.get("gist_version").is_none());
    }

    #[test]
    fn a_revision_that_is_not_a_git_sha_is_refused() {
        let mut g = gist(json!(file("c.testnet", 7, NONCE)));
        g["history"] = json!([{ "version": "v2" }]);
        assert_eq!(read(g), Err("gist_bad_revision"));
    }

    #[test]
    fn a_gist_with_no_history_is_refused() {
        let mut g = gist(json!(file("c.testnet", 7, NONCE)));
        g["history"] = json!([]);
        assert_eq!(read(g), Err("gist_no_history"));
    }

    #[test]
    fn nonce_from_another_milestone_is_refused() {
        assert_eq!(read(gist(json!(file("c.testnet", 7, "old")))), Err("wrong_nonce"));
    }

    #[test]
    fn another_milestone_id_is_refused() {
        assert_eq!(read(gist(json!(file("c.testnet", 8, NONCE)))), Err("wrong_milestone"));
    }

    #[test]
    fn another_contract_is_refused() {
        assert_eq!(read(gist(json!(file("other.testnet", 7, NONCE)))), Err("wrong_contract"));
    }

    #[test]
    fn secret_gist_is_refused() {
        let mut g = gist(json!(file("c.testnet", 7, NONCE)));
        g["public"] = json!(false);
        assert_eq!(read(g), Err("gist_not_public"));
    }

    #[test]
    fn ownerless_gist_is_refused() {
        let mut g = gist(json!(file("c.testnet", 7, NONCE)));
        g["owner"] = Value::Null;
        assert_eq!(read(g), Err("gist_no_owner"));
    }

    #[test]
    fn other_file_names_do_not_count() {
        let g = json!({ "public": true, "owner": { "id": 42 }, "history": [{ "version": NEWEST }],
            "files": { "notes.json": { "content": file("c.testnet", 7, NONCE), "truncated": false } } });
        assert_eq!(read(g), Err("proof_file_missing"));
    }

    #[test]
    fn truncated_file_is_refused() {
        let mut g = gist(json!(file("c.testnet", 7, NONCE)));
        g["files"][PROOF_FILE]["truncated"] = json!(true);
        assert_eq!(read(g), Err("proof_file_truncated"));
    }

    #[test]
    fn prose_is_refused() {
        assert_eq!(read(gist(json!("please pay me.testnet"))), Err("proof_file_bad_json"));
    }

    #[test]
    fn gist_id_rejects_url_tricks() {
        assert!(valid_gist_id("6cad326836d38bd3a7ae"));
        for bad in ["", "a/b", "../x", "abc?x=1", "zz"] {
            assert!(!valid_gist_id(bad), "{bad}");
        }
    }
}
