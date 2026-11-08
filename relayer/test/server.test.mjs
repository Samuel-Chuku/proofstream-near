import assert from 'node:assert/strict';
import { generateKeyPairSync } from 'node:crypto';
import { createServer } from 'node:http';
import { test } from 'node:test';
import { CONTRACT, GAS, METHODS } from '../src/policy.mjs';
import { LIMITS, Limiter, SpendCap, createHandler, serialQueue } from '../src/server.mjs';
import { base58Encode, loadSigner } from '../src/tx.mjs';

const ORIGIN = 'https://proofstream.example';
const GAS_PRICE = 100_000_000n;
const BLOCK_HASH = base58Encode(Buffer.alloc(32, 9));

function secret() {
  const { privateKey, publicKey } = generateKeyPairSync('ed25519');
  const d = Buffer.from(privateKey.export({ format: 'jwk' }).d, 'base64url');
  const x = Buffer.from(publicKey.export({ format: 'jwk' }).x, 'base64url');
  return 'ed25519:' + base58Encode(Buffer.concat([d, x]));
}

/** Reads back what the relayer signed: receiver, method, args, gas, deposit. */
function decodeSigned(b64) {
  const buf = Buffer.from(b64, 'base64');
  let o = 0;
  const u32 = () => ((o += 4), buf.readUInt32LE(o - 4));
  const str = () => {
    const n = u32();
    o += n;
    return buf.subarray(o - n, o).toString();
  };
  const uint = (bytes) => {
    let v = 0n;
    for (let i = bytes - 1; i >= 0; i--) v = (v << 8n) | BigInt(buf[o + i]);
    o += bytes;
    return v;
  };
  const signer = str();
  o += 33;
  const nonce = uint(8);
  const receiver = str();
  o += 32;
  const actions = u32();
  const kind = buf[o++];
  const method = str();
  const args = str();
  return { signer, nonce, receiver, actions, kind, method, args, gas: uint(8), deposit: uint(16) };
}

const passed = { HardFactsPassed: { pr: 8, base_sha: 'b', head_sha: 'h', merged: true, changed_files: 1 } };
function milestone(state, extra = {}) {
  return {
    repo_id: 42, payee_github_id: 7, payee: { account: 'a.testnet' }, binding_in_flight: false,
    paid_attempts_used: 0, max_paid_attempts: 3, fee_remaining: '60000', pins: { verifier: { fee: '20000' } },
    state, ...extra,
  };
}

async function harness({ m = milestone(passed), capYocto = 10n ** 24n, sendFails = false } = {}) {
  const sent = [];
  const rpc = {
    view: async (contract, method) => {
      assert.equal(contract, CONTRACT);
      if (method === 'get_milestone') return typeof m === 'function' ? m() : m;
      if (method === 'get_lock') return null;
      if (method === 'is_pr_used') return false;
      throw new Error(method);
    },
    viewAccessKey: async () => ({ nonce: 41, permission: 'FullAccess' }),
    finalBlock: async () => ({ header: { hash: BLOCK_HASH, gas_price: String(GAS_PRICE) } }),
    sendTx: async (b64) => {
      if (sendFails) throw Object.assign(new Error('x'), { data: { cause: { name: 'TIMEOUT_ERROR' } } });
      sent.push(b64);
      return {};
    },
  };
  const keyText = secret();
  const lines = [];
  const realLog = console.log;
  console.log = (line) => lines.push(line);
  const handle = createHandler({
    rpc, signer: loadSigner(keyText), accountId: 'proofstream-relayer.testnet', publicKeyText: 'ed25519:x',
    allowedOrigin: ORIGIN, dailyCapYocto: capYocto, trustProxy: true,
  });
  const server = createServer(handle);
  await new Promise((r) => server.listen(0, '127.0.0.1', r));
  const url = `http://127.0.0.1:${server.address().port}`;
  const post = (path, body, { ip = '1.1.1.1', origin = ORIGIN, type = 'application/json' } = {}) =>
    fetch(url + path, {
      method: 'POST',
      headers: { 'content-type': type, 'x-real-ip': ip, ...(origin ? { origin } : {}) },
      body: typeof body === 'string' ? body : JSON.stringify(body),
    });
  const close = () => {
    console.log = realLog;
    return new Promise((r) => server.close(r));
  };
  return { post, url, sent, lines, keyText, close };
}

test('a valid review is signed with the fixed receiver, method, gas and deposit', async () => {
  const h = await harness();
  try {
    const res = await h.post('/review', { milestone_id: 5 });
    assert.equal(res.status, 200);
    assert.ok((await res.json()).tx_hash);
    assert.equal(h.sent.length, 1);
    const tx = decodeSigned(h.sent[0]);
    assert.deepEqual(
      { ...tx },
      { signer: 'proofstream-relayer.testnet', nonce: 42n, receiver: CONTRACT, actions: 1, kind: 2, method: 'review', args: '{"milestone_id":5}', gas: GAS, deposit: METHODS.review.deposit },
    );
    assert.equal(res.headers.get('access-control-allow-origin'), ORIGIN);
  } finally {
    await h.close();
  }
});

test('a request the contract would refuse is never sent', async () => {
  const h = await harness({ m: milestone('Funded') });
  try {
    const res = await h.post('/review', { milestone_id: 5 });
    assert.equal(res.status, 409);
    assert.deepEqual(await res.json(), { refused: 'not_reviewable' });
    assert.equal(h.sent.length, 0);
  } finally {
    await h.close();
  }
});

test('other origins, methods, content types and oversized bodies are refused', async () => {
  const h = await harness();
  try {
    assert.equal((await h.post('/review', { milestone_id: 5 }, { origin: 'https://evil.example' })).status, 403);
    assert.equal((await h.post('/funder_release', { milestone_id: 5 })).status, 404);
    assert.equal((await h.post('/review', 'milestone_id=5', { type: 'text/plain' })).status, 415);
    assert.equal((await h.post('/review', '{"milestone_id":')).status, 400);
    assert.equal((await h.post('/review', JSON.stringify({ milestone_id: 5, pad: 'x'.repeat(2000) }))).status, 413);
    assert.equal(h.sent.length, 0);
  } finally {
    await h.close();
  }
});

test('per-IP limit: the 11th request in an hour is refused', async () => {
  const h = await harness({ m: milestone('Funded') });
  try {
    for (let i = 0; i < LIMITS.perIpPerHour; i++) assert.equal((await h.post('/review', { milestone_id: 5 }, { ip: '2.2.2.2' })).status, 409);
    assert.equal((await h.post('/review', { milestone_id: 5 }, { ip: '2.2.2.2' })).status, 429);
    assert.equal((await h.post('/review', { milestone_id: 5 }, { ip: '3.3.3.3' })).status, 409);
  } finally {
    await h.close();
  }
});

test('per-milestone limit holds across IPs', async () => {
  const h = await harness();
  try {
    for (let i = 0; i < LIMITS.perMilestonePerHour; i++) assert.equal((await h.post('/review', { milestone_id: 5 }, { ip: `9.0.0.${i}` })).status, 200);
    const res = await h.post('/review', { milestone_id: 5 }, { ip: '9.0.1.1' });
    assert.equal(res.status, 429);
    assert.equal(h.sent.length, LIMITS.perMilestonePerHour);
  } finally {
    await h.close();
  }
});

test('per-GitHub-id limit holds across milestones', async () => {
  const h = await harness();
  try {
    for (let i = 0; i < LIMITS.perGithubIdPerHour; i++) assert.equal((await h.post('/review', { milestone_id: i }, { ip: `8.0.0.${i}` })).status, 200);
    assert.equal((await h.post('/review', { milestone_id: 99 }, { ip: '8.0.1.1' })).status, 429);
  } finally {
    await h.close();
  }
});

test('the daily cap refuses a send that would exceed it', async () => {
  const one = METHODS.review.deposit + GAS * GAS_PRICE;
  const h = await harness({ capYocto: one * 2n });
  try {
    assert.equal((await h.post('/review', { milestone_id: 1 }, { ip: 'a' })).status, 200);
    assert.equal((await h.post('/review', { milestone_id: 2 }, { ip: 'b' })).status, 200);
    const res = await h.post('/review', { milestone_id: 3 }, { ip: 'c' });
    assert.equal(res.status, 503);
    assert.deepEqual(await res.json(), { refused: 'daily_cap_reached' });
    assert.equal(h.sent.length, 2);
  } finally {
    await h.close();
  }
});

test('a failed send reports the tx hash so the caller can look it up', async () => {
  const h = await harness({ sendFails: true });
  try {
    const res = await h.post('/review', { milestone_id: 5 });
    assert.equal(res.status, 502);
    const body = await res.json();
    assert.equal(body.refused, 'send_failed');
    assert.ok(body.tx_hash);
  } finally {
    await h.close();
  }
});

test('logs hold tx hashes and refusals only: never the key, signed bytes or IPs', async () => {
  const h = await harness();
  try {
    await h.post('/review', { milestone_id: 5 }, { ip: '7.7.7.7' });
    await h.post('/claim', { milestone_id: 5, pr_number: 'x' }, { ip: '7.7.7.7' });
    const text = h.lines.join('\n');
    assert.ok(text.includes('"event":"sent"') && text.includes('"event":"refused"'));
    assert.ok(!text.includes(h.keyText.slice('ed25519:'.length, 20)));
    assert.ok(!text.includes(h.sent[0].slice(0, 40)));
    assert.ok(!text.includes('7.7.7.7'));
    for (const line of h.lines) assert.deepEqual(Object.keys(JSON.parse(line)).filter((k) => !['t', 'event', 'method', 'milestone_id', 'tx', 'reason'].includes(k)), []);
  } finally {
    await h.close();
  }
});

test('sends run one at a time and the wait list is bounded', async () => {
  const enqueue = serialQueue(2);
  let running = 0;
  let maxRunning = 0;
  let release;
  const gate = new Promise((r) => (release = r));
  const job = async () => {
    running++;
    maxRunning = Math.max(maxRunning, running);
    await gate;
    running--;
  };
  const a = enqueue(job);
  const b = enqueue(job);
  await assert.rejects(enqueue(job), (e) => e.reason === 'busy');
  release();
  await Promise.all([a, b]);
  assert.equal(maxRunning, 1);
  await enqueue(async () => {});
});

test('limiter windows expire and the spend cap resets each UTC day', () => {
  let t = Date.UTC(2026, 10, 8, 23, 0);
  const now = () => t;
  const l = new Limiter(1, now);
  l.check('k');
  l.record('k');
  assert.throws(() => l.check('k'), (e) => e.reason === 'rate_limited');
  t += 3_600_001;
  l.check('k');
  const cap = new SpendCap(10n, () => t - 3_600_001);
  cap.take(10n);
  assert.throws(() => cap.take(1n), (e) => e.reason === 'daily_cap_reached');
  const next = new SpendCap(10n, now);
  next.take(10n);
});
