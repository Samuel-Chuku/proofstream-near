#!/usr/bin/env bash
# Shows two claims racing for one payout, read from NEAR testnet. Read-only: no keys, no spending.
# Needs curl and jq. Usage: scripts/race-demo.sh [winner_tx winner_signer loser_tx loser_signer review_tx review_signer]
# Defaults: milestone 3 on proofstream-v1.testnet, PR #24, claimed by two accounts at once.
set -euo pipefail

RPC=${NEAR_RPC:-https://archival-rpc.testnet.near.org}
WINNER_TX=${1:-Br1y2oCKGP5KYqAJjtxpDDzgWVPFkeJtKNKzQKbt7Lm6}
WINNER_SIGNER=${2:-proofstream-contributor.testnet}
LOSER_TX=${3:-8fcfPyi6uwm8cW1UByeLsoZAg9fukDCcQq5Xf1wmKaQL}
LOSER_SIGNER=${4:-proofstream-funder.testnet}
REVIEW_TX=${5:-C9CjQJStg7SKxVLTZvNLL9qpD6GNxZEHSFH2JqSJxwHA}
REVIEW_SIGNER=${6:-proofstream-contributor.testnet}

rpc() { curl -fsS -m 40 "$RPC" -H 'Content-Type: application/json' -d "$1"; }
tx() { rpc "{\"jsonrpc\":\"2.0\",\"id\":\"1\",\"method\":\"tx\",\"params\":{\"tx_hash\":\"$1\",\"sender_account_id\":\"$2\",\"wait_until\":\"FINAL\"}}"; }
height() { rpc "{\"jsonrpc\":\"2.0\",\"id\":\"1\",\"method\":\"block\",\"params\":{\"block_id\":\"$1\"}}" | jq -r .result.header.height; }
block_of() { jq -r --arg log "$2" '[.result.receipts_outcome[] | select(.outcome.logs | any(contains($log))) | .block_hash][0] // empty' <<<"$1"; }

winner=$(tx "$WINNER_TX" "$WINNER_SIGNER")
loser=$(tx "$LOSER_TX" "$LOSER_SIGNER")
review=$(tx "$REVIEW_TX" "$REVIEW_SIGNER")

locked=$(height "$(block_of "$winner" claim_started)")
checked=$(height "$(block_of "$winner" hard_facts_passed)")
paid=$(height "$(block_of "$review" milestone_paid)")
paid_times=$(jq '[.result.receipts_outcome[].outcome.logs[] | select(contains("\"milestone_paid\""))] | length' <<<"$review")
refused_hash=$(jq -r '[.result.receipts_outcome[] | select(.outcome.status.Failure != null) | .block_hash][0]' <<<"$loser")
refused=$(height "$refused_hash")
reason=$(jq -r '[.result.receipts_outcome[].outcome.status.Failure | select(. != null) | tostring | capture("panicked: (?<m>[^\"]+)").m][0]' <<<"$loser")

printf '\nTwo claims for one PR, submitted back to back\n\n'
printf '%-9s %-46s %s\n' 'Block' 'Claim 1 (got the lock)' 'Claim 2'
printf '%-9s %-46s %s\n' "$locked" 'locks the milestone and PR' ''
printf '%-9s %-46s %s\n' "$refused" '' "refused: $reason"
printf '%-9s %-46s %s\n' "$checked" 'checker passed: the hard facts hold' ''
printf '%-9s %-46s %s\n' "$paid" "both AIs reviewed it, paid $paid_times time" ''
printf '\nClaim 1: %s\nClaim 2: %s\nReview:  %s\n' "$WINNER_TX" "$LOSER_TX" "$REVIEW_TX"
printf 'Claim 2 was refused %s blocks before claim 1 finished its check.\n\n' "$((checked - refused))"
