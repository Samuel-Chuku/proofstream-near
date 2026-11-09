// Test signatures for the verifier's verdicts (D48), made with Node's own ed25519 and a
// fixed, public test seed: these keys sign nothing real. The contract tests look each
// message up in verifier-sigs.json; a missing one is written to missing-verifier-sigs.txt,
// and `node tests/fixtures/sign-verifier.mjs` (from contract/) signs and adds it.
import { createPrivateKey, createPublicKey, sign } from 'node:crypto';
import { existsSync, readFileSync, writeFileSync, rmSync } from 'node:fs';

const dir = new URL('.', import.meta.url).pathname;
const FIXTURE = `${dir}verifier-sigs.json`;
const MISSING = `${dir}missing-verifier-sigs.txt`;
const SEED = Buffer.from('proofstream test verifier key 01'); // 32 bytes, public on purpose

const pkcs8 = Buffer.concat([Buffer.from('302e020100300506032b657004220420', 'hex'), SEED]);
const key = createPrivateKey({ key: pkcs8, format: 'der', type: 'pkcs8' });
const publicKey = createPublicKey(key).export({ format: 'der', type: 'spki' }).subarray(12).toString('hex');

const fixture = existsSync(FIXTURE) ? JSON.parse(readFileSync(FIXTURE, 'utf8')) : { public_key: publicKey, sigs: {} };
if (fixture.public_key !== publicKey) throw new Error('seed changed');
const wanted = existsSync(MISSING) ? readFileSync(MISSING, 'utf8').split('\n').filter(Boolean) : [];
for (const message of wanted) fixture.sigs[message] = sign(null, Buffer.from(message), key).toString('hex');
fixture.sigs = Object.fromEntries(Object.entries(fixture.sigs).sort());
writeFileSync(FIXTURE, `${JSON.stringify(fixture, null, 1)}\n`);
if (existsSync(MISSING)) rmSync(MISSING);
console.log(`public key ${publicKey}, ${wanted.length} signed, ${Object.keys(fixture.sigs).length} in total`);
