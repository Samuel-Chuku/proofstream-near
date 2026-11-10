# ProofStream on NEAR: front end

Static pages for `proofstream-v1.testnet`, served at **near.proofstream.site** on Cloudflare Pages. No build step, no framework, and no npm install. The design is the ProofStream system from the Arc site (`assets/tokens.css`, unchanged).

| Page | Who | What it does |
|---|---|---|
| `index.html` | anyone | What ProofStream does and how a milestone pays |
| `new.html` | funder (wallet) | Looks up the repo and contributor on GitHub (the contract pins numeric ids), then creates and funds a milestone in one wallet prompt |
| `milestone.html?id=N` | anyone | State, the certified share, details, the pinned builds (flags a **non-standard build**), the event log with links to each transaction and to the verifier's attestation, and the funder's release / reject / refund |
| `claim.html?id=N` | contributor (no wallet) | Bind a payout account with a public gist, claim with a merged PR, ask for the review. Every step goes through the relayer at `relayer.proofstream.site` |

## What it reads and calls

- **Contract views:** NEAR's public RPC (`rpc.testnet.near.org`, falling back to the archival RPC).
- **The event log:** NearBlocks' v3 API lists every transaction that touched the contract; each transaction's events are then read from the RPC.
- **Contributor actions:** the relayer, which only accepts this site's origin (`https://near.proofstream.site`). Locally, every page renders, but relayed actions are refused by design.
- **Funder actions:** the wallet, through `@hot-labs/near-connect` 0.12.0. It's vendored and hashed in `vendor/` (see `vendor/README.md`); this is the only third-party code on the site.
- **Pins:** `builds.json` holds the reproducible-build hashes for commit `068742e`. `js/config.mjs` holds the contract, relayer, verifier key and fee.

## Run locally

```bash
cd web
python3 -m http.server 8765
# open http://127.0.0.1:8765
```

## Deploy (Cloudflare Pages)

1. Cloudflare dashboard: **Workers & Pages → Create → Pages → Connect to Git**, then choose this repository.
2. **Build settings:**
   - Framework preset: None
   - Build command: empty
   - Build output directory: `web`
3. **Deploy**, then go to **Custom domains → Set up a custom domain**: `near.proofstream.site`. Keep it proxied (orange cloud).
4. `_headers` sets the security headers: no framing, nosniff, referrer policy and HSTS.

## After a new deploy of the contract or enclave builds

- **Contract or relayer address:** update `CONTRACT` or `RELAYER` in `js/config.mjs`.
- **Enclave builds:** replace `builds.json` with the new CI `builds.json`. Milestones pinned to older builds are then shown as non-standard.
- **Verifier signing key:** if it changes, update `VERIFIER_PUBLIC_KEY`. Read it from an attested verifier run.
