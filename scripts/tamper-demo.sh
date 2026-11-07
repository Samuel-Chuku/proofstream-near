#!/usr/bin/env bash
# Shows a forged AI verdict being rejected inside the enclave, read from NEAR testnet.
# Read-only: no keys, no spending. Needs curl and jq.
#
# Both runs judge the same commit against the same milestone. The second run's build
# changes "does_not_meet" to "meets" in the model's response before verifying it, which
# is what anyone altering the response on its way to the enclave would have to do.
# Add --tests to also run the signature tests locally (needs Rust; no keys).
set -euo pipefail

RPC=${NEAR_RPC:-https://rpc.testnet.near.org}
SENDER=proofstream-checker-owner.testnet
HONEST_TX=9hrsYmttmdg6QGMtnvXLA5zsNi1ysmuWvhijL7m6zGGV
FORGED_TX=9F76frZeVEJyHFU53V6iXNVmCgsGKeRHEZMbaFPNMW5H

tx() {
  curl -fsS -m 40 "$RPC" -H 'Content-Type: application/json' \
    -d "{\"jsonrpc\":\"2.0\",\"id\":\"1\",\"method\":\"tx\",\"params\":{\"tx_hash\":\"$1\",\"sender_account_id\":\"$SENDER\",\"wait_until\":\"FINAL\"}}"
}
args() { jq -c '.result.transaction.actions[0].FunctionCall.args | @base64d | fromjson' <<<"$1"; }
output() { jq -c '.result.status.SuccessValue | @base64d | fromjson' <<<"$1"; }
field() { jq -r "$2" <<<"$1"; }

honest=$(tx "$HONEST_TX")
forged=$(tx "$FORGED_TX")
honest_args=$(args "$honest")
forged_args=$(args "$forged")
honest_input=$(field "$honest_args" .input_data)
forged_input=$(field "$forged_args" .input_data)
honest_out=$(output "$honest")
forged_out=$(output "$forged")

if [ "$honest_input" != "$forged_input" ]; then
  echo "These two runs had different inputs, so the comparison doesn't hold." >&2
  exit 1
fi
if [ "$(field "$forged_out" '.verdict // "none"')" != none ] || [ "$(field "$forged_out" '.error // ""')" != signature_text_mismatch ]; then
  echo "The forged run didn't fail with signature_text_mismatch: $forged_out" >&2
  exit 1
fi

printf '\nA forged AI verdict, rejected inside the enclave\n\n'
printf 'Commit:     %s\n' "$(field "$honest_input" .head_sha)"
printf 'Milestone:  %s\n\n' "$(field "$honest_input" .milestone_text)"
printf 'Unchanged response\n'
printf '  Build:    %s\n' "$(field "$honest_args" .source.Project.version_key)"
printf '  Signer:   %s (attested once, then pinned in the code)\n' "$(field "$honest_out" .signer)"
printf '  Verdict:  %s, signature verified\n' "$(field "$honest_out" .verdict)"
printf '  Tx:       %s\n\n' "$HONEST_TX"
printf 'Response changed from does_not_meet to meets\n'
printf '  Build:    %s (test build, never pinned)\n' "$(field "$forged_args" .source.Project.version_key)"
printf '  Result:   %s, no verdict\n' "$(field "$forged_out" .error)"
printf '  Tx:       %s\n\n' "$FORGED_TX"
printf 'The signature covers the exact bytes, so one changed word means no verdict at all.\n'
printf 'This shows the check rejects altered bytes. It does not show the model is right.\n\n'

if [ "${1:-}" = --tests ]; then
  (cd "$(dirname "$0")/../outlayer/judge" && cargo test verify:: 2>&1 | grep -E '^test verify|^test result')
fi
