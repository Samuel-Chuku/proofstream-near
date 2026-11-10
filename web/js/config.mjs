// Everything this site talks to, in one place. Testnet only.
export const NETWORK = 'testnet';
export const CONTRACT = 'proofstream-v1.testnet';
export const TOKEN = { id: 'usdc.fakes.testnet', symbol: 'USDC', decimals: 6 };
export const RELAYER = 'https://relayer.proofstream.site';
export const RPC = ['https://rpc.testnet.near.org', 'https://archival-rpc.testnet.near.org'];
export const NEARBLOCKS = 'https://api-testnet.nearblocks.io/v3';
export const OUTLAYER_API = 'https://testnet-api.outlayer.ai';
export const EXPLORER_TX = (hash) => `https://testnet.nearblocks.io/txns/${hash}`;
export const ARC_SITE = 'https://proofstream.site';
export const REPO = 'https://github.com/Samuel-Chuku/proofstream-near';

// The standard pins (D28). builds.json beside this file holds the reproducible-build hashes;
// a milestone pinning anything else is shown as a non-standard build.
export const OWNERS = {
  judge: 'proofstream-checker-owner.testnet',
  verifier: 'proofstream-verifier-owner.testnet',
};
export const PROJECTS = {
  checker: `${OWNERS.judge}/proofstream-checker`,
  binder: `${OWNERS.judge}/proofstream-binder`,
  judge: `${OWNERS.judge}/proofstream-judge`,
  verifier: `${OWNERS.verifier}/proofstream-verifier`,
};
// The verifier project's in-enclave signing key, read from an attested run (call 599eced2…).
export const VERIFIER_PUBLIC_KEY = '6abd46ea8cb3bffd74a8b4acce3d9a1fef81956e57ef86dd3f0970a00e25c7f1';
// The second opinion's fee per review, in token units ($0.02).
export const VERIFIER_FEE = '20000';

// Deposits and gas the contract requires (contract/src/lib.rs). The relayer uses the same.
export const TGAS = 10n ** 12n;
export const CREATE_DEPOSIT = 10n ** 23n; // 0.1 NEAR for storage; the unused part comes back
export const GAS_300 = (300n * TGAS).toString();
