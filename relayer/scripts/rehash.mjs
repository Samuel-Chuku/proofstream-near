#!/usr/bin/env node
// Gate (D40 condition 4): rebuild real testnet transactions from their fields with our own
// encoder and require the exact on-chain hash and a valid signature.
// Usage: node relayer/scripts/rehash.mjs <tx_hash>:<signer_account> [...]
// The block hash a tx refers to isn't returned by the RPC, so it is searched for in the
// blocks just before the one that included the tx; a match proves every encoded byte.
import { makeRpc } from '../src/rpc.mjs';
import { base58Decode, base58Encode, encodeTransaction, parseEd25519, txHash, verifySignature } from '../src/tx.mjs';

const ARCHIVAL = ['https://archival-rpc.testnet.fastnear.com', 'https://archival-rpc.testnet.near.org'];
const SEARCH_BLOCKS = 200;
const rpc = makeRpc(ARCHIVAL, { timeoutMs: 30_000, attempts: 6 });

async function rehash(hash, signer) {
  const r = await rpc.call('EXPERIMENTAL_tx_status', { tx_hash: hash, sender_account_id: signer, wait_until: 'NONE' });
  const t = r.transaction;
  if (t.actions.length !== 1 || !t.actions[0].FunctionCall) throw new Error('not a single FunctionCall');
  if (t.priority_fee) throw new Error('priority_fee set: not a V0 transaction');
  const fc = t.actions[0].FunctionCall;
  const fields = {
    signerId: t.signer_id,
    publicKey: parseEd25519(t.public_key, 32),
    nonce: BigInt(t.nonce),
    receiverId: t.receiver_id,
    methodName: fc.method_name,
    args: Buffer.from(fc.args, 'base64'),
    gas: BigInt(fc.gas),
    deposit: BigInt(fc.deposit),
  };
  const included = await rpc.call('block', { block_id: r.transaction_outcome.block_hash });
  let height = included.header.height;
  for (let i = 0; i < SEARCH_BLOCKS; i++, height--) {
    let block;
    try {
      block = await rpc.call('block', { block_id: height });
    } catch (e) {
      if (e.data?.cause?.name === 'UNKNOWN_BLOCK') continue; // skipped height
      throw e;
    }
    const encoded = encodeTransaction({ ...fields, blockHash: base58Decode(block.header.hash) });
    const h = txHash(encoded);
    if (base58Encode(h) === hash) {
      const sigOk = verifySignature(h, parseEd25519(t.signature, 64), fields.publicKey);
      return { hash, method: fields.methodName, args: fields.args.toString(), deposit: String(fields.deposit), gas: String(fields.gas), block_hash_height: height, blocks_back: i, signature_valid: sigOk, match: sigOk };
    }
  }
  return { hash, method: fields.methodName, match: false, reason: `no block in the ${SEARCH_BLOCKS} before inclusion gives this hash` };
}

const targets = process.argv.slice(2);
if (!targets.length) {
  console.error('usage: rehash.mjs <tx_hash>:<signer_account> [...]');
  process.exit(2);
}
let failed = 0;
for (const target of targets) {
  const [hash, signer] = target.split(':');
  try {
    const out = await rehash(hash, signer);
    if (!out.match) failed++;
    console.log(JSON.stringify(out));
  } catch (e) {
    failed++;
    console.log(JSON.stringify({ hash, match: false, error: e.message }));
  }
}
console.log(failed ? `FAIL: ${failed} of ${targets.length}` : `PASS: ${targets.length} of ${targets.length} re-encoded exactly`);
process.exit(failed ? 1 : 0);
