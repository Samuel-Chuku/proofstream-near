//! Checks a NEAR AI response signature against the signer pinned at attestation (D7).
//! The signed text covers the exact request and response bytes.

use k256::ecdsa::{RecoveryId, Signature, VerifyingKey};
use serde::Deserialize;
use sha2::{Digest as _, Sha256};
use sha3::Keccak256;

const PROVIDER_TEE: &str = "provider_tee";
const ECDSA: &str = "ecdsa";
const SIGNATURE_BYTES: usize = 65;
const ADDRESS_BYTES: usize = 20;
const R_S_BYTES: usize = 64;
const ETHEREUM_V_OFFSET: u8 = 27;

#[derive(Deserialize, Debug, Clone)]
pub struct ResponseSignature {
    pub text: String,
    pub signature: String,
    pub signing_address: String,
    pub signing_algo: String,
    pub signature_kind: Option<String>,
}

#[derive(Debug, PartialEq, Eq, Clone, Copy)]
pub enum Rejected {
    NotProviderTee,
    NotEcdsa,
    SignerNotPinned,
    TextMismatch,
    BadSignature,
    RecoveredWrongSigner,
}

impl Rejected {
    pub fn code(self) -> &'static str {
        match self {
            Rejected::NotProviderTee => "signature_not_provider_tee",
            Rejected::NotEcdsa => "signature_not_ecdsa",
            Rejected::SignerNotPinned => "signer_not_pinned",
            Rejected::TextMismatch => "signature_text_mismatch",
            Rejected::BadSignature => "signature_malformed",
            Rejected::RecoveredWrongSigner => "signature_wrong_signer",
        }
    }
}

pub fn sha256_hex(bytes: &[u8]) -> String {
    to_hex(&Sha256::digest(bytes))
}

fn to_hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

fn from_hex(s: &str) -> Option<Vec<u8>> {
    let s = s.strip_prefix("0x").or_else(|| s.strip_prefix("0X")).unwrap_or(s);
    if s.len() % 2 != 0 {
        return None;
    }
    (0..s.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(s.get(i..i + 2)?, 16).ok())
        .collect()
}

/// The lowercase `0x` address that signed `text` under EIP-191.
pub fn recover_signer(text: &str, signature_hex: &str) -> Option<String> {
    let raw = from_hex(signature_hex)?;
    if raw.len() != SIGNATURE_BYTES {
        return None;
    }
    let v = raw[R_S_BYTES];
    let recovery = RecoveryId::from_byte(if v >= ETHEREUM_V_OFFSET { v - ETHEREUM_V_OFFSET } else { v })?;
    let signature = Signature::from_slice(&raw[..R_S_BYTES]).ok()?;

    let mut hasher = Keccak256::new();
    hasher.update(format!("\x19Ethereum Signed Message:\n{}", text.len()).as_bytes());
    hasher.update(text.as_bytes());
    let prehash = hasher.finalize();

    let key = VerifyingKey::recover_from_prehash(&prehash, &signature, recovery).ok()?;
    let point = key.to_sec1_point(false);
    // Skip the 0x04 tag: an Ethereum address hashes the bare public key.
    let hash = Keccak256::digest(&point.as_bytes()[1..]);
    Some(format!("0x{}", to_hex(&hash[hash.len() - ADDRESS_BYTES..])))
}

/// Every check must pass. A rotated signer fails here until a new attestation is pinned.
pub fn check(
    signature: &ResponseSignature,
    model: &str,
    request: &[u8],
    response: &[u8],
    pinned_signer: &str,
) -> Result<(), Rejected> {
    if signature.signature_kind.as_deref() != Some(PROVIDER_TEE) {
        return Err(Rejected::NotProviderTee);
    }
    if signature.signing_algo != ECDSA {
        return Err(Rejected::NotEcdsa);
    }
    if !signature.signing_address.eq_ignore_ascii_case(pinned_signer) {
        return Err(Rejected::SignerNotPinned);
    }
    let expected = format!("{model}:{}:{}", sha256_hex(request), sha256_hex(response));
    if signature.text != expected {
        return Err(Rejected::TextMismatch);
    }
    let recovered = recover_signer(&signature.text, &signature.signature).ok_or(Rejected::BadSignature)?;
    if !recovered.eq_ignore_ascii_case(pinned_signer) {
        return Err(Rejected::RecoveredWrongSigner);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::Value;

    const JUDGE_CALL: &str = include_str!("../tests/fixtures/judge_real_call.json");
    const VERIFIER_CALL: &str = include_str!("../tests/fixtures/verifier_real_call.json");

    /// One real call to NEAR AI Cloud: exact bytes and the signature it returned.
    fn call(fixture: &str) -> (String, Vec<u8>, Vec<u8>, ResponseSignature, String) {
        let v: Value = serde_json::from_str(fixture).unwrap();
        let text = |k: &str| v[k].as_str().unwrap().to_string();
        (
            text("model"),
            text("request").into_bytes(),
            text("response").into_bytes(),
            serde_json::from_value(v["signature"].clone()).unwrap(),
            text("pinned_signer"),
        )
    }

    fn real_call() -> (String, Vec<u8>, Vec<u8>, ResponseSignature, String) {
        call(JUDGE_CALL)
    }

    #[test]
    fn real_response_passes_for_both_models() {
        for (fixture, cfg) in [(JUDGE_CALL, &crate::rules::JUDGE), (VERIFIER_CALL, &crate::rules::VERIFIER)] {
            let (model, req, res, sig, pinned) = call(fixture);
            assert_eq!((model.as_str(), pinned.as_str()), (cfg.model, cfg.signer), "fixture matches the role's pins");
            assert_eq!(check(&sig, cfg.model, &req, &res, cfg.signer), Ok(()));
        }
    }

    #[test]
    fn one_models_signature_does_not_pass_as_the_others() {
        let (_, req, res, sig, _) = call(VERIFIER_CALL);
        let judge = &crate::rules::JUDGE;
        assert_eq!(check(&sig, judge.model, &req, &res, judge.signer), Err(Rejected::SignerNotPinned));
    }

    #[test]
    fn tampered_response_fails() {
        let (model, req, mut res, sig, pinned) = real_call();
        let last = res.len() - 1;
        res[last] ^= 1;
        assert_eq!(check(&sig, &model, &req, &res, &pinned), Err(Rejected::TextMismatch));
    }

    #[test]
    fn tampered_request_fails() {
        let (model, mut req, res, sig, pinned) = real_call();
        req[0] ^= 1;
        assert_eq!(check(&sig, &model, &req, &res, &pinned), Err(Rejected::TextMismatch));
    }

    #[test]
    fn another_model_name_fails() {
        let (_, req, res, sig, pinned) = real_call();
        assert_eq!(check(&sig, "other/model", &req, &res, &pinned), Err(Rejected::TextMismatch));
    }

    #[test]
    fn rotated_signer_fails_closed() {
        let (model, req, res, sig, _) = real_call();
        let other = "0x0000000000000000000000000000000000000001";
        assert_eq!(check(&sig, &model, &req, &res, other), Err(Rejected::SignerNotPinned));
    }

    #[test]
    fn gateway_signature_is_refused() {
        let (model, req, res, mut sig, pinned) = real_call();
        sig.signature_kind = Some("gateway".to_string());
        assert_eq!(check(&sig, &model, &req, &res, &pinned), Err(Rejected::NotProviderTee));
    }

    #[test]
    fn missing_kind_is_refused() {
        let (model, req, res, mut sig, pinned) = real_call();
        sig.signature_kind = None;
        assert_eq!(check(&sig, &model, &req, &res, &pinned), Err(Rejected::NotProviderTee));
    }

    #[test]
    fn non_ecdsa_is_refused() {
        let (model, req, res, mut sig, pinned) = real_call();
        sig.signing_algo = "ed25519".to_string();
        assert_eq!(check(&sig, &model, &req, &res, &pinned), Err(Rejected::NotEcdsa));
    }

    #[test]
    fn signature_from_another_key_fails_even_when_text_matches() {
        let (model, req, res, mut sig, pinned) = real_call();
        // Flipping a byte of `s` recovers a different key, or none.
        let mut raw = from_hex(&sig.signature).unwrap();
        raw[40] ^= 1;
        sig.signature = format!("0x{}", to_hex(&raw));
        let result = check(&sig, &model, &req, &res, &pinned);
        assert!(matches!(result, Err(Rejected::RecoveredWrongSigner | Rejected::BadSignature)), "{result:?}");
    }

    #[test]
    fn malformed_signature_fails() {
        let (model, req, res, mut sig, pinned) = real_call();
        sig.signature = "0xzz".to_string();
        assert_eq!(check(&sig, &model, &req, &res, &pinned), Err(Rejected::BadSignature));
    }

    #[test]
    fn sha256_matches_a_known_value() {
        assert_eq!(
            sha256_hex(b"abc"),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
    }
}
