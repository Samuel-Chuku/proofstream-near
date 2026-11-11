# ProofStream on NEAR

Milestone payouts for open-source work, released by verified facts and two independent AI reviews instead of a person's say-so.

A funder escrows a budget in a NEAR contract against one milestone of a GitHub repository. A contributor gets paid when a merged pull request passes the **hard facts**, checked inside an Intel TDX enclave on OutLayer, and **two AI reviewers** from different model vendors and owners both certify how much of the milestone it completes. The contract pays the **lower** of the two shares, so half a job earns half the budget. No AI verdict can move money on its own.

> **Status (2026-11-09):** every component is built and tested. The final contract, `proofstream-v1.testnet`, isn't deployed yet. The on-chain evidence below comes from the spike contracts that proved each piece. Testnet only, with free test USDC.

## How a milestone pays

1. **Create and fund.** The funder sets the repo (pinned by its numeric GitHub id), the branch, the contributor's GitHub id, what "done" means, the CI checks that must pass, a deadline, and the exact wasm hash of every enclave program. Then they send exactly the budget plus the verifier's fee in USDC.
2. **Bind a payout account.** The contributor posts a public gist naming the contract, the milestone, a one-time nonce and a NEAR account. An enclave program confirms that the gist's owner holds the milestone's GitHub id. The contract then checks that the account exists (or creates an implicit one) before recording it. From then on, money can only go there.
3. **Claim.** The checker runs in TDX and reads GitHub's API. The contract then applies the rules:
   - merged into the right repo and branch, by the right author, after the milestone was created;
   - the required checks succeeded;
   - nothing under `.github/` changed;
   - GitHub returned the complete file list;
   - a human other than the author merged it or approved its exact head.

   The PR is locked across every milestone while this runs.
4. **Review.** The contract starts the judge (GLM, on NEAR AI Cloud) in TDX and pays it the second opinion's fee.
   - **The judge decides first:** it verifies its model's signature inside the enclave and returns `meets`, `partial`, `does_not_meet` or `unclear`, with the share of the whole milestone that's done.
   - **Then it buys a second opinion:** if it reports progress, it calls the verifier (Qwen, a different project and owner) through OutLayer's HTTPS API, paying from its owner's payment key, capped in its code at $0.10.
   - **The verifier signs:** it checks its own model's signature, then signs its verdict with an ed25519 key that exists only inside its enclave, bound to this contract, milestone, attempt, commit and share, and to the account that paid.
   - **The contract checks it:** with `ed25519_verify`, against the verifier key pinned in the milestone. The judge can pass the verdict on, but it can't change it.
   - **Context:** both AIs see earlier paid PRs, marked as already paid, so redoing paid work earns nothing.
5. **Pay the certified share.** The lower of the two shares counts.
   - **Pays:** the newly certified part of the budget, after which the milestone stays open for the next PR. At 100 % it's paid in full.
   - **Waits for the funder:** any objection, two shares more than 30 points apart, or no new progress. The funder can release a share they choose, or reject the PR.
   - **Refund:** after the deadline, the funder takes back whatever was never certified.

Contributors without NEAR can use the relayer ([relayer/](relayer/)). It submits only `bind_payee`, `claim` and `review`, with fixed deposits, and checks the contract's state first. It can refuse to send a request, but it can't redirect money. Anyone with NEAR can call the contract directly instead.

## Claims and how to check them

None of these needs keys or spends anything.

| Claim | Command | Expected |
|---|---|---|
| Every contract rule has a test, including each refusal | `cd contract && cargo test --lib` | `153 passed` |
| Money moves through a real token as the rules say, and the books reconcile | `cd contract && cargo test --test sandbox` (local NEAR node, ~40 s) | `16 passed` |
| A verifier verdict the judge altered, replayed from another review, or signed by another key is refused | `cd contract && cargo test --lib verdict` | `13 passed` |
| The AI review refuses unsigned, altered, inconsistent or off-commit answers | `cd outlayer/ai-review && cargo test` | `50 passed` |
| The checker fails closed on every hard fact | `cd outlayer/checker && cargo test` | `36 passed` |
| The binder accepts only the GitHub id holder's gist for this contract and milestone | `cd outlayer/binder && cargo test` | `14 passed` |
| The relayer can only send what the contract would accept | `node --test relayer/test/*.test.mjs` | `pass 26` |
| Two claims racing for one payout: one pays, one is refused | `scripts/race-demo.sh` | `Claim 2 was refused 86 blocks before claim 1 finished its check.` |
| A forged AI verdict is rejected inside the enclave | `scripts/tamper-demo.sh` | `signature_text_mismatch, no verdict` |
| An execution ran the pinned build on approved TDX hardware, signed by Intel | `python3 -I scripts/verify-execution.py DqMmabKEXwxAYyM5KCubBmjMc18fFio819zJzYDQpqNV` | `PASS: 8 passed, 0 failed, 0 skipped` |
| The relayer's transaction encoder matches NEAR byte for byte | `node relayer/scripts/rehash.mjs 3ECgMRHSomNW7dZnocwyMPw1R8bUNG8GDpzShic8Zuou:proofstream-contributor.testnet` | `PASS: 1 of 1 re-encoded exactly` |
| Every enclave build and the contract are reproducible | [.github/workflows/build.yml](.github/workflows/build.yml): builds each one twice from scratch in pinned images | identical hashes, written to `builds.json` |

`verify-execution.py` uses the standard library only. Its last check, `intel-chain`, needs [`dcap-qvl`](https://crates.io/crates/dcap-qvl-cli) (`cargo install dcap-qvl-cli --version 0.7.1 --locked`) and prints `SKIP` without it.

## Trust model

**Attested** (bound into each execution's TDX quote): the task fields, including the project id, the wasm hash that ran, and the input and output hashes. The quote's measurements are on OutLayer's approved list, and the chain ends at Intel.

**The AI's role:** the AI's role comes from the OutLayer project the contract called. The worker sets it, and neither the input nor the secrets can change it. The project id, the wasm hash, the input and the output (which echoes the role) are all bound into the execution's TDX quote. The contract refuses a verdict from the wrong role; the quote itself is checked off-chain, by anyone, with `scripts/verify-execution.py`, not by the contract.

**Checked on-chain:**
- **The second opinion:** the verifier's verdict carries an ed25519 signature from a key that only exists inside the verifier project's enclave (OutLayer `signing_keys`, no caller). The contract checks it against the key pinned in the milestone, over a message it builds itself (contract, milestone, attempt, commit, verdict, share, and the paying account). The key belongs to the verifier's project, not to one build: a new version published by the verifier's owner would sign with the same key. The judge pins the verifier build it calls, and each call's attestation shows the build, but the contract can't tell which build signed.
- **Callbacks:** each one comes from `worker.outlayer.testnet`, through a key that was added only after that worker's TDX quote passed verification. That key can call four OutLayer methods.
- **The rules:** the contract applies every rule, refusal and payout itself. It has no admin, setter or upgrade method.

**Checked by anyone, off-chain:** each execution's quote, with `verify-execution.py`. OutLayer itself notes that the per-execution quote binds a result to an approved build, not to the worker's registered key, so the two are separate facts.

**Trusted:**
- **GitHub's API:** for every hard fact.
- **OutLayer:** its measurement allowlist and keystore. The keystore holds the secrets, locked to our contract and to each build's hash.
- **NEAR AI's TEE:** model responses are signed by a key attested once, off-enclave. Instances of one model share that key.
- **The people who merge:** their judgment.
- **The attested caller:** it's the transaction's signer, not the contract.

**The payment chain:**
- **Funder to contract:** on-chain.
- **Contract to the judge's owner:** on-chain, through OutLayer's `attached_usd`, once per review.
- **Judge to the verifier's owner:** the judge's run pays from its owner's OutLayer payment key, over OutLayer's HTTPS API. The key is topped up on-chain, but the payment is in OutLayer's ledger, not a NEAR transaction. Measured: this fee is paid even when the verifier's run fails, so a failed second opinion costs the judge's owner, not the funder.
- **Each AI to NEAR AI:** prepaid API credit, **off-chain**.

## Limits

- **Tests:** a contributor can weaken their own tests. That's what the AI is told to look for, and it can still miss it.
- **Independence:** a second GitHub account controlled by the author defeats the independent-approval rule.
- **Funder power:** a funder who disagrees can reject and, after the deadline, refund. The contributor's protection is the milestone text and deadline they read before starting, plus the public record of every rejection and its reason.
- **Partial shares are a model's estimate.** They're bounded by two models, the lower one and a 30-point gap, but how well they score partial work hasn't been measured beyond our test PRs.
- **Limits per milestone:** at most 5 paid PRs. Review attempts are shared by all of them (default 10, up to 25).
- **Private repos:** only GitHub Actions results are visible to the checker.
- **Operators:** the judge and verifier are independent in keys and accounts, but have the same operator for now.
- **The second opinion:** the judge only asks for it when it reports progress. When the judge objects, the milestone is held on the judge's word alone, with no verifier fee spent.
- **Money:** test USDC is free to mint, and nothing runs on mainnet.

## Run it

**Needs:** Rust 1.99 with the `wasm32-unknown-unknown` and `wasm32-wasip2` targets, Node 22, and OpenSSL headers for the sandbox tests. On a machine without `pkg-config`, set `OPENSSL_LIB_DIR` and `OPENSSL_INCLUDE_DIR`.

**Environment variables** (names in [.env.example](.env.example), values never committed). They're stored as OutLayer encrypted secrets, never passed as input:
- `GITHUB_TOKEN`: read-only GitHub token for the checker, binder and judge.
- `NEAR_AI_API_KEY`: the judge's NEAR AI key.
- `VERIFIER_GITHUB_TOKEN`, `VERIFIER_NEAR_AI_API_KEY`: the verifier's own, under its own owner.

The relayer has its own settings and key file; see [relayer/DEPLOY.md](relayer/DEPLOY.md).

## Next steps

- **Bind the verifier's key to its build.** OutLayer's `bind: "wasm"` keys belong to one exact build, but they are only issued to direct runs, not project runs. Either OutLayer supports per-build keys for projects, or the verifier runs directly from its wasm URL.
- **Open operators.** Anyone runs the agents and keeps whatever fee they don't spend, priced up front. Funders pick pinned code, not operators.
- **More:**
  - a GitHub App instead of tokens;
  - checking model attestation inside the enclave;
  - a third-party arbiter for rejections;
  - meta-transactions (NEP-366) once a contributor has an account;
  - open bounties for any author.
- **Pay contributors on any chain.** The gist names an address on Base, Solana or another chain, and the payout routes through NEAR Intents. Before this ships, the Intents deposit address has to be fetched and bound inside the enclave, so no relayer can substitute its own. Mainnet only, since NEAR Intents has no testnet.
- **Fund milestones from any chain.** USDC arrives through NEAR Intents' `customRecipientMsg`, which calls the contract's `ft_transfer_call`. This needs an exact-output quote and a funding message that names the funder, because the sender is Intents' withdrawal account. NEAR Intents marks this path as experimental.
- **Optional yield on escrowed budgets.** An opt-in setting would supply idle milestone budgets to RHEA Lending through `@rhea-finance/cross-chain-sdk` and withdraw them before payout. Like any DeFi integration, it would be off by default, and the funder chooses whether to use it.

## Prior work and the other entry

- **Prior work:** [ProofStream on Arc](https://github.com/Samuel-Chuku/Proof-Stream), where an agent signed an attestation and paid a second agent over x402 for a second opinion. This entry moves the facts and the judgement into attested enclaves, pins every build per milestone, and lets the contract, not an agent's key, decide.
- **The other entry:** Stead, Samuel's other NEAR HACKATHON entry. The two share no code, accounts or keys.

**Tracks:** Real-World Money, AI Security & Defense, Agent Economy, Privacy & Private AI.
