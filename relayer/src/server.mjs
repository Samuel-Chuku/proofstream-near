// HTTP front of the relayer: POST /bind_payee, /claim, /review and GET /health.
// Stateless across restarts; all limits are in memory (BUILD_BRIEF 0.1).
import { CONTRACT, GAS, METHODS, Refusal, isMethod, parseRequest, precheck } from './policy.mjs';
import { base58Decode, signTransaction } from './tx.mjs';

const MAX_BODY_BYTES = 1024;
const HOUR_MS = 3_600_000;
export const LIMITS = {
  perIpPerHour: 10,
  perMilestonePerHour: 6,
  perGithubIdPerHour: 10,
  maxWaiting: 8,
  maxTrackedKeys: 10_000,
};

/** Sliding one-hour windows, pruned as they're used, with a cap on tracked keys. */
export class Limiter {
  constructor(limit, now, windowMs = HOUR_MS) {
    this.limit = limit;
    this.now = now;
    this.windowMs = windowMs;
    this.hits = new Map();
  }
  recent(key) {
    const cutoff = this.now() - this.windowMs;
    const list = (this.hits.get(key) ?? []).filter((t) => t > cutoff);
    if (list.length) this.hits.set(key, list);
    else this.hits.delete(key);
    return list;
  }
  check(key) {
    if (this.recent(key).length >= this.limit) throw new Refusal('rate_limited', 429);
    if (!this.hits.has(key) && this.hits.size >= LIMITS.maxTrackedKeys) {
      for (const k of [...this.hits.keys()]) this.recent(k);
      if (this.hits.size >= LIMITS.maxTrackedKeys) throw new Refusal('busy', 503);
    }
  }
  record(key) {
    this.hits.set(key, [...this.recent(key), this.now()]);
  }
}

/** NEAR committed per UTC day: deposit plus all prepaid gas at the current gas price. */
export class SpendCap {
  constructor(capYocto, now) {
    this.cap = capYocto;
    this.now = now;
    this.day = null;
    this.spent = 0n;
  }
  take(amount) {
    const day = new Date(this.now()).toISOString().slice(0, 10);
    if (day !== this.day) {
      this.day = day;
      this.spent = 0n;
    }
    if (this.spent + amount > this.cap) throw new Refusal('daily_cap_reached', 503);
    this.spent += amount;
  }
}

/** One send at a time (one nonce sequence), with a bounded wait list. */
export function serialQueue(maxWaiting) {
  let tail = Promise.resolve();
  let waiting = 0;
  return (job) => {
    if (waiting >= maxWaiting) return Promise.reject(new Refusal('busy', 503));
    waiting++;
    const run = tail.then(job);
    tail = run.catch(() => {}).finally(() => waiting--);
    return run;
  };
}

const log = (event, fields) => console.log(JSON.stringify({ t: new Date().toISOString(), event, ...fields }));

/**
 * deps: { rpc, signer, accountId, publicKeyText, allowedOrigin, dailyCapYocto,
 *         trustProxy, now }
 */
export function createHandler(deps) {
  const now = deps.now ?? Date.now;
  const perIp = new Limiter(LIMITS.perIpPerHour, now);
  const perMilestone = new Limiter(LIMITS.perMilestonePerHour, now);
  const perGithub = new Limiter(LIMITS.perGithubIdPerHour, now);
  const spend = new SpendCap(deps.dailyCapYocto, now);
  const enqueue = serialQueue(LIMITS.maxWaiting);
  const view = (method, args) => deps.rpc.view(CONTRACT, method, args);

  async function submit(req) {
    const m = await precheck(req, view);
    perMilestone.check(String(req.milestoneId));
    perGithub.check(String(m.payee_github_id));
    const [key, block] = await Promise.all([deps.rpc.viewAccessKey(deps.accountId, deps.publicKeyText), deps.rpc.finalBlock()]);
    const deposit = METHODS[req.method].deposit;
    spend.take(deposit + GAS * BigInt(block.header.gas_price));
    perMilestone.record(String(req.milestoneId));
    perGithub.record(String(m.payee_github_id));
    const { hash, signedBase64 } = signTransaction(
      {
        signerId: deps.accountId,
        nonce: BigInt(key.nonce) + 1n,
        receiverId: CONTRACT,
        blockHash: base58Decode(block.header.hash),
        methodName: req.method,
        args: req.args,
        gas: GAS,
        deposit,
      },
      deps.signer,
    );
    try {
      await deps.rpc.sendTx(signedBase64);
    } catch (e) {
      // The tx may still land: report its hash so the caller can look it up.
      log('send_failed', { method: req.method, milestone_id: req.milestoneId, tx: hash, error: e.data?.cause?.name ?? e.name });
      throw Object.assign(new Refusal('send_failed', 502), { tx: hash });
    }
    log('sent', { method: req.method, milestone_id: req.milestoneId, tx: hash });
    return hash;
  }

  function reply(res, status, body, origin) {
    const headers = { 'content-type': 'application/json', 'cache-control': 'no-store' };
    if (origin && origin === deps.allowedOrigin) {
      headers['access-control-allow-origin'] = origin;
      headers.vary = 'Origin';
    }
    res.writeHead(status, headers);
    res.end(JSON.stringify(body));
  }

  function readBody(httpReq) {
    return new Promise((resolve, reject) => {
      const chunks = [];
      let size = 0;
      httpReq.on('data', (c) => {
        size += c.length;
        // Past the limit, stop keeping data but let the reply go out; Nginx caps it too.
        if (size > MAX_BODY_BYTES) reject(new Refusal('body_too_large', 413));
        else chunks.push(c);
      });
      httpReq.on('end', () => resolve(Buffer.concat(chunks).toString('utf8')));
      httpReq.on('error', reject);
    });
  }

  async function handle(httpReq, res) {
    const origin = httpReq.headers.origin;
    const path = new URL(httpReq.url, 'http://relayer').pathname;
    const method = path.slice(1);
    try {
      if (origin !== undefined && origin !== deps.allowedOrigin) throw new Refusal('origin_not_allowed', 403);
      if (httpReq.method === 'GET' && path === '/health') return reply(res, 200, { ok: true, contract: CONTRACT }, origin);
      if (httpReq.method === 'OPTIONS' && isMethod(method)) {
        res.writeHead(204, {
          'access-control-allow-origin': deps.allowedOrigin,
          'access-control-allow-methods': 'POST',
          'access-control-allow-headers': 'content-type',
          'access-control-max-age': '600',
          vary: 'Origin',
        });
        return res.end();
      }
      if (httpReq.method !== 'POST' || !isMethod(method)) throw new Refusal('not_found', 404);
      if (!(httpReq.headers['content-type'] ?? '').startsWith('application/json')) throw new Refusal('json_only', 415);
      const ip = deps.trustProxy ? httpReq.headers['x-real-ip'] : httpReq.socket.remoteAddress;
      perIp.check(String(ip));
      perIp.record(String(ip));
      let body;
      try {
        body = JSON.parse(await readBody(httpReq));
      } catch (e) {
        throw e instanceof Refusal ? e : new Refusal('bad_json');
      }
      const req = parseRequest(method, body);
      const tx = await enqueue(() => submit(req));
      return reply(res, 200, { tx_hash: tx }, origin);
    } catch (e) {
      if (e instanceof Refusal) {
        log('refused', { method: isMethod(method) ? method : undefined, reason: e.reason, ...(e.tx ? { tx: e.tx } : {}) });
        return reply(res, e.status, { refused: e.reason, ...(e.tx ? { tx_hash: e.tx } : {}) }, origin);
      }
      log('error', { method: isMethod(method) ? method : undefined, error: e.data?.cause?.name ?? e.name });
      return reply(res, 502, { refused: 'upstream_error' }, origin);
    }
  }
  // Auto-claim sends through the same queue, prechecks, limits and daily cap.
  handle.submit = (req) => enqueue(() => submit(req));
  handle.view = view;
  return handle;
}
