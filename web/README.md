# ProofStream on NEAR: front end

Pages for `proofstream-v1.testnet`, served at **near.proofstream.site** on Cloudflare Pages, plus two small Pages Functions. No build step, no framework, and no npm install. The design is the ProofStream system from the Arc site (`assets/tokens.css`, unchanged).

| Page | Who | What it does |
|---|---|---|
| `index.html` | anyone | What ProofStream does, with the latest AI decisions read from the chain |
| `app.html` | anyone | Totals from the contract's events, getting started (wallet, test NEAR, test USDC, GitHub), and every milestone with filters, including **Mine** |
| `new.html` | funder (wallet) | Repository (picked from GitHub, or typed), contributor, terms, and what is still blocking; one wallet prompt creates and funds the milestone |
| `milestone.html?id=N` | anyone | State, the certified share, paid pull requests, each AI decision with the judge's reasons and the verifier's signed verdict, every transaction, and the funder's release / reject / refund |
| `claim.html?id=N` | contributor (no wallet) | Bind a payout account with a public gist, claim with a merged PR, and ask for the review. Every step goes through the relayer at `relayer.proofstream.site` |
| `docs.html` | anyone | How it works: the flow, the two AIs, the guarantees, and what is trusted |

## What it reads and calls

- **Contract views:** JSON-RPC. FastNEAR's archival node first, then NEAR's public and archival nodes; a node that answers 429 is skipped.
- **History:** NearBlocks' v3 API lists the contract's transactions; each one's events and the judge's output are read from the RPC. A finished transaction never changes, so the browser keeps it in `localStorage`.
- **`/api/index`** (`functions/api/index.js`): the same history built once for every visitor and kept in Cloudflare's cache, so the public RPC sees one reader. It stays within the free plan's 50 subrequests and stops reading at a deadline; whatever is left is read by the next call. If it is down, each page reads the chain itself.
- **`/api/github/*`** (`functions/api/github/[[route]].js`): signs the funder in with the ProofStream GitHub App, then lists their repositories and branches and can create a branch. The session is an encrypted, HttpOnly cookie. Without the settings below it answers 503, and the form still accepts a typed public repository.
- **Contributor actions:** the relayer, which only accepts this site's origin (`https://near.proofstream.site`). Locally, every page renders, but relayed actions are refused by design.
- **Funder actions:** the wallet, through `@hot-labs/near-connect` 0.12.0. It's vendored and hashed in `vendor/` (see `vendor/README.md`), loaded only when a wallet is used, and is the only third-party code on the site.
- **Pins:** `builds.json` holds the reproducible-build hashes for commit `068742e`. `js/config.mjs` holds the contract, relayer, verifier key and fee.

## Run locally

```bash
cd web
python3 -m http.server 8765
# open http://127.0.0.1:8765
```

The functions don't run under a plain static server, so locally each page reads the chain itself and GitHub sign-in is unavailable.

## Deploy (Cloudflare Pages)

1. **Build settings:**
   - Framework preset: None
   - Build command: empty
   - Root directory: `web`
   - Build output directory: `.`

   The root must be `web` so Pages finds `functions/` and `_routes.json`; `_routes.json` runs the functions only for `/api/*`.
2. **Environment variables (Production, encrypted):**

   | Name | Value |
   |---|---|
   | `GITHUB_APP_CLIENT_ID` | the GitHub App's client id |
   | `GITHUB_APP_CLIENT_SECRET` | a client secret generated on the GitHub App |
   | `GITHUB_APP_SLUG` | `proofstream` |
   | `SESSION_SECRET` | `openssl rand -base64 48` |

3. **GitHub App:** add `https://near.proofstream.site/api/github/callback` as a callback URL, and set the repository permission **Contents** to *Read and write* (needed to create a branch).
4. **Custom domain:** `near.proofstream.site`, proxied (orange cloud).
5. `_headers` sets the security headers: no framing, nosniff, referrer policy and HSTS.

## After a new deploy of the contract or enclave builds

- **Contract or relayer address:** update `CONTRACT` or `RELAYER` in `js/config.mjs`.
- **Enclave builds:** replace `builds.json` with the new CI `builds.json`. Milestones pinned to older builds are then shown as non-standard.
- **Verifier signing key:** if it changes, update `VERIFIER_PUBLIC_KEY`. Read it from an attested verifier run.
