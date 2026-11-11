// The chain index, built once and shared by every visitor, so the public RPC sees one reader
// instead of one per page view. If this is down, the pages read the chain themselves.
//
// Two Cloudflare free-plan limits shape it:
// - 50 subrequests per call, and every Cache API match or put counts toward the same 50.
//   So it makes four cache calls at most: the fresh index (kept 30 seconds) and the last index
//   built (kept a year), whose finished transactions are reused because they never change.
// - A call can be cancelled once the visitor's page stops waiting (it waits 45 seconds).
//   So all chain reads stop at a deadline, well inside that.
// Whatever isn't read by then is marked unread and served, and the next call carries on from it.
import { buildIndex, txDetail } from '../../js/chain.mjs';

const INDEX_TTL_S = 30;
const LAST_TTL_S = 365 * 24 * 3600;
const SUBREQUEST_LIMIT = 50;
/// Two cache matches before reading and two cache puts after.
const CACHE_CALLS = 4;
const READ_FOR_MS = 25000;
/// How old an earlier index may be and still be answered while a new one is built.
const STALE_FOR_MS = 10 * 60 * 1000;

/// Every fetch in this isolate goes through this guard. It refuses once the strictest running
/// call has used its share, and aborts at that call's deadline. Counting from the oldest running
/// call's start also counts any call running beside it, so it can only stop reads early.
const running = new Set();
let fetchesTotal = 0;
const realFetch = globalThis.fetch;
globalThis.fetch = function guardedFetch(input, init = {}) {
  let start = Infinity;
  let deadline = Infinity;
  for (const c of running) { start = Math.min(start, c.start); deadline = Math.min(deadline, c.deadline); }
  if (running.size) {
    if (fetchesTotal - start >= SUBREQUEST_LIMIT - CACHE_CALLS) return Promise.reject(Object.assign(new Error('subrequest budget spent'), { stop: true }));
    const left = deadline - Date.now();
    if (left <= 0) return Promise.reject(Object.assign(new Error('read deadline passed'), { stop: true }));
    const stop = AbortSignal.timeout(left);
    init = { ...init, signal: init.signal ? AbortSignal.any([init.signal, stop]) : stop };
  }
  fetchesTotal += 1;
  return realFetch.call(globalThis, input, init);
};

const json = (body, ttl) => new Response(body, { headers: { 'Content-Type': 'application/json', 'Cache-Control': `public, max-age=${ttl}` } });

/// Reads the chain and stores the result. Every index is kept as the last one, so the next build
/// reuses what this one read; only a complete one is shared as the fresh index. Each call builds
/// its own: Workers don't let one request use I/O started by another.
async function rebuild(cache, keys, known) {
  const readTx = async (hash) => known.get(hash) ?? txDetail(hash);
  const call = { start: fetchesTotal, deadline: Date.now() + READ_FOR_MS };
  running.add(call);
  let index;
  try { index = await buildIndex(readTx); } finally { running.delete(call); }
  const body = JSON.stringify(index);
  const puts = [cache.put(keys.last, json(body, LAST_TTL_S))];
  if (!index.unread) puts.push(cache.put(keys.index, json(body, INDEX_TTL_S)));
  return { body, puts: Promise.all(puts) };
}

export async function onRequestGet(context) {
  try {
    return await serve(context);
  } catch (e) {
    // Answer with the reason rather than Cloudflare's bare error page; the pages then read the
    // chain themselves.
    return new Response(JSON.stringify({ error: 'index_failed', message: String(e?.message ?? e) }), { status: 502, headers: { 'Content-Type': 'application/json', 'Cache-Control': 'no-store' } });
  }
}

async function serve(context) {
  const { request } = context;
  const waitUntil = (p) => context.waitUntil(p);
  const cache = caches.default;
  const origin = new URL(request.url).origin;
  const keys = { index: new Request(`${origin}/api/index`), last: new Request(`${origin}/api/__index-last`) };

  const hit = await cache.match(keys.index);
  if (hit) return hit;

  const lastRes = await cache.match(keys.last);
  const last = lastRes ? await lastRes.json() : null;
  const known = new Map();
  for (const t of last?.txs ?? []) if (!t.unread) known.set(t.hash, { signer: t.signer, events: t.events, judge: t.judge });

  // A complete earlier index is answered at once and refreshed behind it: the pages read each
  // milestone live, so this only delays the history and the totals by one rebuild.
  if (last && !last.unread && Date.now() - last.at < STALE_FOR_MS) {
    waitUntil(rebuild(cache, keys, known).then((r) => r.puts));
    return json(JSON.stringify({ ...last, stale: true }), 0);
  }
  const { body, puts } = await rebuild(cache, keys, known);
  waitUntil(puts);
  return json(body, INDEX_TTL_S);
}
