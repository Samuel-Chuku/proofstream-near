// Entry point: read config and the key file, check the account and OutLayer's prices, serve.
import { readFileSync } from 'node:fs';
import { createServer } from 'node:http';
import { AI_LIMITS, COMPUTE_PER_AI_RUN, COMPUTE_PER_FACTS_RUN, CONTRACT, FACTS_LIMITS } from './policy.mjs';
import { makeRpc } from './rpc.mjs';
import { createHandler } from './server.mjs';
import { base58Encode, loadSigner } from './tx.mjs';

const OUTLAYER = 'outlayer.testnet';
const DEFAULT_RPCS = 'https://rpc.testnet.near.org,https://test.rpc.fastnear.com';
const DEFAULT_KEY_FILE = '/run/secrets/relayer_key';
const DEFAULT_PORT = 8080;
const YOCTO_PER_MILLINEAR = 10n ** 21n;
const MILLINEAR = /^[0-9]{1,6}$/;

function required(name) {
  const v = process.env[name];
  if (!v) throw new Error(`${name} is required`);
  return v;
}

async function main() {
  const allowedOrigin = required('ALLOWED_ORIGIN');
  if (!/^https?:\/\/[^/]+$/.test(allowedOrigin)) throw new Error('ALLOWED_ORIGIN must be scheme://host[:port] with no path');
  const capText = required('DAILY_CAP_MILLINEAR');
  if (!MILLINEAR.test(capText)) throw new Error('DAILY_CAP_MILLINEAR must be a whole number of milliNEAR');
  const port = Number(process.env.PORT ?? DEFAULT_PORT);
  const rpc = makeRpc((process.env.RPC_URLS ?? DEFAULT_RPCS).split(',').map((u) => u.trim()).filter(Boolean));

  const accountId = required('RELAYER_ACCOUNT_ID');
  // The key file is JSON with `private_key` (other fields are ignored). Only the parsed key
  // objects are kept; nothing from the file is ever logged.
  let signer;
  try {
    signer = loadSigner(JSON.parse(readFileSync(process.env.RELAYER_KEY_FILE ?? DEFAULT_KEY_FILE, 'utf8')).private_key);
  } catch {
    // Node's JSON errors quote the input, which here is the key: report a fixed message.
    throw new Error('relayer key file is missing or is not JSON with an ed25519 private_key');
  }
  const publicKeyText = 'ed25519:' + base58Encode(signer.publicKey);

  const key = await rpc.viewAccessKey(accountId, publicKeyText);
  if (key.permission !== 'FullAccess') throw new Error('the relayer key must be a full-access key (deposits)');
  for (const [limits, perRun, name] of [
    [FACTS_LIMITS, COMPUTE_PER_FACTS_RUN, 'facts'],
    [AI_LIMITS, COMPUTE_PER_AI_RUN, 'ai'],
  ]) {
    const estimate = BigInt(await rpc.view(OUTLAYER, 'estimate_execution_cost', { resource_limits: limits, input_len: 0 }));
    if (estimate > perRun) throw new Error(`OutLayer's ${name} estimate ${estimate} exceeds the fixed ${perRun}; refusing to start`);
  }

  const handle = createHandler({
    rpc,
    signer,
    accountId,
    publicKeyText,
    allowedOrigin,
    dailyCapYocto: BigInt(capText) * YOCTO_PER_MILLINEAR,
    trustProxy: process.env.TRUST_PROXY === '1',
  });
  createServer(handle).listen(port, () =>
    console.log(JSON.stringify({ t: new Date().toISOString(), event: 'listening', port, account: accountId, contract: CONTRACT })),
  );
}

main().catch((e) => {
  console.error(JSON.stringify({ t: new Date().toISOString(), event: 'startup_failed', error: e.message }));
  process.exit(1);
});
