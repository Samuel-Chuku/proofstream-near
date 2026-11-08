// The one transaction shape the relayer signs: a single FunctionCall, borsh-encoded as
// NEAR's Transaction (V0) and SignedTransaction. No npm packages (D40).
import { createHash, createPrivateKey, createPublicKey, sign, verify } from 'node:crypto';

const B58 = '123456789ABCDEFGHJKLMNPQRSTUVWXYZabcdefghijkmnopqrstuvwxyz';
const KEY_TYPE_ED25519 = 0;
const ACTION_FUNCTION_CALL = 2;
const ED25519_PUBLIC_BYTES = 32;
const ED25519_SECRET_BYTES = 64;
const ED25519_SIGNATURE_BYTES = 64;
const HASH_BYTES = 32;
const U64_MAX = (1n << 64n) - 1n;
const U128_MAX = (1n << 128n) - 1n;

export function base58Encode(bytes) {
  let n = 0n;
  for (const b of bytes) n = (n << 8n) | BigInt(b);
  let out = '';
  while (n > 0n) {
    out = B58[Number(n % 58n)] + out;
    n /= 58n;
  }
  for (const b of bytes) {
    if (b !== 0) break;
    out = '1' + out;
  }
  return out;
}

export function base58Decode(text) {
  let n = 0n;
  for (const c of text) {
    const v = B58.indexOf(c);
    if (v < 0) throw new Error('invalid base58');
    n = n * 58n + BigInt(v);
  }
  const bytes = [];
  while (n > 0n) {
    bytes.unshift(Number(n & 0xffn));
    n >>= 8n;
  }
  for (const c of text) {
    if (c !== '1') break;
    bytes.unshift(0);
  }
  return Buffer.from(bytes);
}

/** `ed25519:<base58>` → raw bytes of the expected length. */
export function parseEd25519(text, length) {
  if (typeof text !== 'string' || !text.startsWith('ed25519:')) throw new Error('not an ed25519 key');
  const bytes = base58Decode(text.slice('ed25519:'.length));
  if (bytes.length !== length) throw new Error('wrong ed25519 length');
  return bytes;
}

class Writer {
  constructor() {
    this.parts = [];
  }
  u8(v) {
    this.parts.push(Buffer.from([v]));
  }
  u32(v) {
    const b = Buffer.alloc(4);
    b.writeUInt32LE(v);
    this.parts.push(b);
  }
  uint(v, bytes, max) {
    const n = BigInt(v);
    if (n < 0n || n > max) throw new Error('integer out of range');
    const b = Buffer.alloc(bytes);
    for (let i = 0; i < bytes; i++) b[i] = Number((n >> BigInt(8 * i)) & 0xffn);
    this.parts.push(b);
  }
  bytes(buf) {
    this.u32(buf.length);
    this.parts.push(Buffer.from(buf));
  }
  fixed(buf, length) {
    if (buf.length !== length) throw new Error('wrong fixed length');
    this.parts.push(Buffer.from(buf));
  }
  string(s) {
    this.bytes(Buffer.from(s, 'utf8'));
  }
  done() {
    return Buffer.concat(this.parts);
  }
}

/**
 * borsh(Transaction V0) with exactly one FunctionCall action.
 * tx: { signerId, publicKey (32 bytes), nonce, receiverId, blockHash (32 bytes),
 *       methodName, args (Buffer), gas, deposit }
 */
export function encodeTransaction(tx) {
  const w = new Writer();
  w.string(tx.signerId);
  w.u8(KEY_TYPE_ED25519);
  w.fixed(tx.publicKey, ED25519_PUBLIC_BYTES);
  w.uint(tx.nonce, 8, U64_MAX);
  w.string(tx.receiverId);
  w.fixed(tx.blockHash, HASH_BYTES);
  w.u32(1);
  w.u8(ACTION_FUNCTION_CALL);
  w.string(tx.methodName);
  w.bytes(tx.args);
  w.uint(tx.gas, 8, U64_MAX);
  w.uint(tx.deposit, 16, U128_MAX);
  return w.done();
}

export const txHash = (encoded) => createHash('sha256').update(encoded).digest();

export function encodeSignedTransaction(encoded, signature) {
  const w = new Writer();
  w.fixed(encoded, encoded.length);
  w.u8(KEY_TYPE_ED25519);
  w.fixed(signature, ED25519_SIGNATURE_BYTES);
  return w.done();
}

const b64url = (buf) => Buffer.from(buf).toString('base64url');

/** A NEAR ed25519 secret (`ed25519:` + base58 of seed‖public) as node:crypto keys. */
export function loadSigner(secretText) {
  const secret = parseEd25519(secretText, ED25519_SECRET_BYTES);
  const seed = secret.subarray(0, ED25519_PUBLIC_BYTES);
  const publicKey = Buffer.from(secret.subarray(ED25519_PUBLIC_BYTES));
  const privateKey = createPrivateKey({ key: { kty: 'OKP', crv: 'Ed25519', d: b64url(seed), x: b64url(publicKey) }, format: 'jwk' });
  // Seed and public half must belong together, or every signature would be rejected.
  const derived = createPublicKey(privateKey).export({ format: 'jwk' }).x;
  if (derived !== b64url(publicKey)) throw new Error('secret key halves do not match');
  return { publicKey, privateKey };
}

/** Encode, hash and sign. Returns the hash (base58) and the base64 SignedTransaction. */
export function signTransaction(tx, signer) {
  const encoded = encodeTransaction({ ...tx, publicKey: signer.publicKey });
  const hash = txHash(encoded);
  const signature = sign(null, hash, signer.privateKey);
  return { hash: base58Encode(hash), signedBase64: encodeSignedTransaction(encoded, signature).toString('base64') };
}

export function verifySignature(hash, signature, publicKeyBytes) {
  const key = createPublicKey({ key: { kty: 'OKP', crv: 'Ed25519', x: b64url(publicKeyBytes) }, format: 'jwk' });
  return verify(null, hash, key, signature);
}
