import assert from 'node:assert/strict';
import { generateKeyPairSync } from 'node:crypto';
import { test } from 'node:test';
import { base58Decode, base58Encode, encodeTransaction, loadSigner, parseEd25519, signTransaction, txHash, verifySignature } from '../src/tx.mjs';

// tx 3ECgMRHSomNW7dZnocwyMPw1R8bUNG8GDpzShic8Zuou on testnet: a `claim` with the same
// shape the relayer sends. The referenced block hash was found by scripts/rehash.mjs.
const REAL_CLAIM = {
  signerId: 'proofstream-contributor.testnet',
  publicKey: parseEd25519('ed25519:D18gMWsTnfgqf5KWrRQ9eaY1WijxD5hDqDtNkKgdvVRc', 32),
  nonce: 276471039000002n,
  receiverId: 'proofstream-contract.testnet',
  blockHash: base58Decode('4QdViC2tq7AzHpDY2onatEspozE1vMdCnpoySPxA8iNM'),
  methodName: 'claim',
  args: Buffer.from('{"milestone_id":0,"pr_number":8}'),
  gas: 300_000_000_000_000n,
  deposit: 10_000_000_000_000_000_000_000n,
};
const REAL_HASH = '3ECgMRHSomNW7dZnocwyMPw1R8bUNG8GDpzShic8Zuou';
const REAL_SIG = 'ed25519:4ysZX2XwkPDmgxxBrE8bY5kDjv1K95Aw58MZ6izXYZsgGju6HCScJZccy8cBxMBf7J8hBZYN7raW39p78h7vgyaE';

function nearSecret() {
  const { privateKey, publicKey } = generateKeyPairSync('ed25519');
  const d = Buffer.from(privateKey.export({ format: 'jwk' }).d, 'base64url');
  const x = Buffer.from(publicKey.export({ format: 'jwk' }).x, 'base64url');
  return { text: 'ed25519:' + base58Encode(Buffer.concat([d, x])), d, x };
}

test('encoding a real testnet claim reproduces its on-chain hash', () => {
  const h = txHash(encodeTransaction(REAL_CLAIM));
  assert.equal(base58Encode(h), REAL_HASH);
  assert.ok(verifySignature(h, parseEd25519(REAL_SIG, 64), REAL_CLAIM.publicKey));
});

test('changing any field changes the hash', () => {
  const variants = [
    { nonce: REAL_CLAIM.nonce + 1n },
    { deposit: REAL_CLAIM.deposit + 1n },
    { gas: REAL_CLAIM.gas - 1n },
    { receiverId: 'proofstream-v1.testnet' },
    { methodName: 'review' },
    { args: Buffer.from('{"milestone_id":0,"pr_number":9}') },
  ];
  for (const v of variants) assert.notEqual(base58Encode(txHash(encodeTransaction({ ...REAL_CLAIM, ...v }))), REAL_HASH);
});

test('base58 round-trips, including leading zero bytes', () => {
  for (const bytes of [Buffer.from([0, 0, 1, 2, 255]), Buffer.alloc(32, 7), Buffer.from([0])]) {
    assert.deepEqual(base58Decode(base58Encode(bytes)), bytes);
  }
});

test('out-of-range integers are refused, not truncated', () => {
  assert.throws(() => encodeTransaction({ ...REAL_CLAIM, nonce: 1n << 64n }), /out of range/);
  assert.throws(() => encodeTransaction({ ...REAL_CLAIM, deposit: -1n }), /out of range/);
});

test('signing yields a signature the public key verifies, over the returned hash', () => {
  const secret = nearSecret();
  const signer = loadSigner(secret.text);
  const { hash, signedBase64 } = signTransaction({ ...REAL_CLAIM }, signer);
  const signed = Buffer.from(signedBase64, 'base64');
  const encoded = signed.subarray(0, signed.length - 65);
  assert.equal(base58Encode(txHash(encoded)), hash);
  assert.equal(signed[signed.length - 65], 0);
  assert.ok(verifySignature(txHash(encoded), signed.subarray(signed.length - 64), secret.x));
});

test('a secret whose halves do not belong together is refused', () => {
  const a = nearSecret();
  const b = nearSecret();
  const mixed = 'ed25519:' + base58Encode(Buffer.concat([a.d, b.x]));
  assert.throws(() => loadSigner(mixed), /do not match/);
  assert.throws(() => loadSigner('ed25519:abc'), /length/);
  assert.throws(() => loadSigner('secp256k1:abc'), /not an ed25519/);
});
