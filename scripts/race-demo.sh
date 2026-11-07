#!/usr/bin/env bash
# Shows two claims racing for one payout, read from NEAR testnet. Read-only: no keys, no spending.
# Needs curl and jq. Usage: scripts/race-demo.sh [winner_tx winner_signer loser_tx loser_signer]
set -euo pipefail

RPC=${NEAR_RPC:-https://rpc.testnet.near.org}
WINNER_TX=${1:-EhV5kyS1QREmsdPaqVCkmE8ZxfQpSfWmijyVQd4GqAJp}
WINNER_SIGNER=${2:-proofstream-contributor.testnet}
LOSER_TX=${3:-GUt5pkjUzb2FoeCYMsMPghVecpqYSwTjdChiiTXiSc61}
LOSER_SIGNER=${4:-proofstream-verifier-owner.testnet}

rpc() { curl -fsS -m 40 "$RPC" -H 'Content-Type: application/json' -d "$1"; }
tx() { rpc "{\"jsonrpc\":\"2.0\",\"id\":\"1\",\"method\":\"tx\",\"params\":{\"tx_hash\":\"$1\",\"sender_account_id\":\"$2\",\"wait_until\":\"FINAL\"}}"; }
height() { rpc "{\"jsonrpc\":\"2.0\",\"id\":\"1\",\"method\":\"block\",\"params\":{\"block_id\":\"$1\"}}" | jq -r .result.header.height; }
block_of() { jq -r --arg log "$2" '[.result.receipts_outcome[] | select(.outcome.logs | any(contains($log))) | .block_hash][0] // empty' <<<"$1"; }

winner=$(tx "$WINNER_TX" "$WINNER_SIGNER")
loser=$(tx "$LOSER_TX" "$LOSER_SIGNER")

locked=$(height "$(block_of "$winner" claim_started)")
paid=$(height "$(block_of "$winner" milestone_paid)")
refused_hash=$(jq -r '[.result.receipts_outcome[] | select(.outcome.status.Failure != null) | .block_hash][0]' <<<"$loser")
refused=$(height "$refused_hash")
reason=$(jq -r '[.result.receipts_outcome[].outcome.status.Failure | select(. != null) | tostring | capture("panicked: (?<m>[^\"]+)").m][0]' <<<"$loser")

printf '\nTwo claims for one PR, submitted back to back\n\n'
printf '%-9s %-46s %s\n' 'Block' 'Claim 1 (got the lock)' 'Claim 2'
printf '%-9s %-46s %s\n' "$locked" 'locks the milestone and PR' ''
printf '%-9s %-46s %s\n' "$refused" '' "refused: $reason"
printf '%-9s %-46s %s\n' "$paid" 'checker passed, paid once' ''
printf '\nClaim 1: %s\nClaim 2: %s\n' "$WINNER_TX" "$LOSER_TX"
printf 'Claim 2 was refused %s blocks before claim 1 finished.\n\n' "$((paid - refused))"
