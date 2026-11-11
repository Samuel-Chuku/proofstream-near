// Reading NEAR: contract views over JSON-RPC, a transaction's events, and the contract's
// transaction list from NearBlocks. No wallet needed for any of it.
import { CONTRACT, RPC, NEARBLOCKS, RELAYER, TOKEN } from './config.mjs';

const TIMEOUT_MS = 15000;

async function rpc(method, params, urls = RPC, timeoutMs = TIMEOUT_MS) {
  let last;
  for (const url of urls) {
    for (let attempt = 0; attempt < 3; attempt++) {
      try {
        const res = await fetch(url, {
          method: 'POST',
          headers: { 'Content-Type': 'application/json' },
          body: JSON.stringify({ jsonrpc: '2.0', id: 1, method, params }),
          signal: AbortSignal.timeout(timeoutMs),
        });
        // Rate-limited: this node won't help for a while, so try the next one now.
        if (res.status === 429) { last = new Error('rpc 429'); break; }
        if (res.status >= 500) throw new Error(`rpc ${res.status}`);
        const body = await res.json();
        if (body.error) throw Object.assign(new Error(body.error.cause?.name || body.error.message), { rpc: body.error });
        return body.result;
      } catch (e) {
        last = e;
        // The caller ran out of budget (functions/api.js): retrying can't help.
        if (e.stop) throw e;
        // A node that doesn't know the transaction (too old for it): try the next node now.
        if (e.rpc && /UNKNOWN_TRANSACTION/.test(e.message)) break;
        if (e.rpc && !/TIMEOUT|TOO_MANY/.test(e.message)) throw e;
        await new Promise((r) => setTimeout(r, 400 * 2 ** attempt));
      }
    }
  }
  throw last;
}

const b64 = (obj) => btoa(String.fromCharCode(...new TextEncoder().encode(JSON.stringify(obj))));

export async function view(method, args = {}, account = CONTRACT) {
  const r = await rpc('query', { request_type: 'call_function', finality: 'final', account_id: account, method_name: method, args_base64: b64(args) });
  const text = new TextDecoder().decode(new Uint8Array(r.result));
  return text ? JSON.parse(text) : null;
}

export const getMilestone = (id) => view('get_milestone', { milestone_id: id });
export const getBindNonce = (id) => view('get_bind_nonce', { milestone_id: id });
export const getConfig = () => view('get_config');
export const tokenBalance = (account) => view('ft_balance_of', { account_id: account }, TOKEN.id);

/// An account's NEAR balance in yocto, or null if the account doesn't exist.
export async function nearBalance(account) {
  try {
    const r = await rpc('query', { request_type: 'view_account', finality: 'final', account_id: account });
    return BigInt(r.amount);
  } catch { return null; }
}

/// The next milestone id: the first one that doesn't exist yet. Used to name the
/// milestone in the funding transfer that goes out in the same wallet batch.
export async function nextMilestoneId() {
  let hi = 1;
  while (await getMilestone(hi - 1)) hi *= 2;
  let lo = 0;
  while (lo < hi) {
    const mid = (lo + hi) >> 1;
    if (await getMilestone(mid)) lo = mid + 1;
    else hi = mid;
  }
  return lo;
}

/// One transaction, read once: its signer, ProofStream's NEP-297 events, and, for a review,
/// the judge's full answer (its reasons and the verifier's signed verdict are a receipt's
/// return value, not an event). RPC nodes that track every shard accept any sender.
const TX_CACHE = 'ps-tx-';
const TX_TIMEOUT_MS = 30000;

export async function txDetail(hash) {
  try {
    const cached = localStorage.getItem(TX_CACHE + hash);
    if (cached) return JSON.parse(cached);
  } catch { /* no storage: read it fresh */ }
  const detail = await readTx(hash);
  // A final transaction never changes, so it is kept for good.
  try { localStorage.setItem(TX_CACHE + hash, JSON.stringify(detail)); } catch { /* full or blocked */ }
  return detail;
}

async function readTx(hash) {
  const r = await rpc('tx', { tx_hash: hash, sender_account_id: CONTRACT, wait_until: 'FINAL' }, RPC, TX_TIMEOUT_MS);
  const events = [];
  let judge = null;
  for (const o of r.receipts_outcome) {
    for (const log of o.outcome.logs) {
      if (!log.startsWith('EVENT_JSON:')) continue;
      try {
        const e = JSON.parse(log.slice('EVENT_JSON:'.length));
        if (e.standard === 'proofstream') for (const d of e.data) events.push({ event: e.event, ...d });
      } catch { /* not ours */ }
    }
    const value = o.outcome.status?.SuccessValue;
    if (value && value.length > 40) {
      try {
        const out = JSON.parse(new TextDecoder().decode(Uint8Array.from(atob(value), (c) => c.charCodeAt(0))));
        if (out && out.role === 'judge') judge = out;
      } catch { /* not JSON */ }
    }
  }
  return { hash, signer: r.transaction.signer_id, events, judge };
}

export async function txEvents(hash) {
  return (await txDetail(hash)).events;
}

/// Every transaction that touched the contract, newest first, from NearBlocks. Its list of
/// direct transactions misses funding (a transfer to the token that calls back into the
/// contract), so recent receipts are merged in to catch those. NearBlocks pages at 100.
const PAGE = 100;
const MAX_PAGES = 5;

async function nearblocks(path) {
  const res = await fetch(`${NEARBLOCKS}${path}`, { signal: AbortSignal.timeout(TIMEOUT_MS) });
  if (!res.ok) throw new Error(`nearblocks ${res.status}`);
  return res.json();
}

export async function contractTxns() {
  const seen = new Map();
  const add = (hash, ns) => { if (hash && !seen.has(hash)) seen.set(hash, BigInt(ns ?? 0)); };
  let cursor = null;
  let firstOfLast = null;
  for (let page = 0; page < MAX_PAGES; page++) {
    const body = await nearblocks(`/accounts/${CONTRACT}/txns?limit=${PAGE}${cursor ? `&cursor=${encodeURIComponent(cursor)}` : ''}`);
    const rows = body.data || [];
    if (!rows.length || rows[0].transaction_hash === firstOfLast) break; // the cursor was ignored
    firstOfLast = rows[0].transaction_hash;
    for (const t of rows) add(t.transaction_hash, t.block?.block_timestamp ?? t.block_timestamp);
    cursor = body.meta?.next_page;
    if (!cursor) break;
  }
  const receipts = await nearblocks(`/accounts/${CONTRACT}/receipts?limit=${PAGE}`).catch(() => ({ data: [] }));
  for (const r of receipts.data || []) add(r.transaction_hash, r.included_in_block_timestamp ?? r.block?.block_timestamp);
  return [...seen].map(([hash, ns]) => ({ hash, ns })).sort((a, b) => (a.ns > b.ns ? -1 : a.ns < b.ns ? 1 : 0));
}

/// A call through the relayer. Returns the tx hash, or throws with the relayer's reason.
export async function relay(method, body) {
  const res = await fetch(`${RELAYER}/${method}`, {
    method: 'POST',
    headers: { 'Content-Type': 'application/json' },
    body: JSON.stringify(body),
    signal: AbortSignal.timeout(90000),
  });
  const out = await res.json().catch(() => ({}));
  if (!res.ok || !out.tx_hash) throw Object.assign(new Error(out.refused || out.error || `relayer ${res.status}`), { reason: out.refused, tx: out.tx_hash });
  return out.tx_hash;
}

/// The state's variant name and its fields: "Funded" or { Held: {...} } → ["Held", {...}].
export function stateOf(m) {
  if (typeof m.state === 'string') return [m.state, {}];
  const [name] = Object.keys(m.state);
  return [name, m.state[name]];
}

// ---------------------------------------------------------------- the index

const INDEX_TTL_MS = 30000;
const INDEX_KEY = 'ps-index-v1';
let indexPromise = null;

function storedIndex() {
  try {
    const raw = sessionStorage.getItem(INDEX_KEY);
    if (!raw) return null;
    const v = JSON.parse(raw);
    return Date.now() - v.at < INDEX_TTL_MS ? v : null;
  } catch { return null; }
}

/// Every milestone, read live from the contract: one view call each, eight at a time.
export async function readMilestones() {
  const n = await nextMilestoneId();
  const milestones = [];
  for (let i = 0; i < n; i += 8) {
    const ids = Array.from({ length: Math.min(8, n - i) }, (_, k) => i + k);
    milestones.push(...(await Promise.all(ids.map(async (id) => ({ id, ...(await getMilestone(id)) })))));
  }
  return milestones;
}

/// The index kept from an earlier page in this tab, if it is under 30 seconds old.
export const cachedIndex = () => storedIndex();

/// Builds the index: every milestone, and every transaction that touched the contract with its
/// events, newest first. `readTx` lets the server-side copy cache transactions its own way.
export async function buildIndex(readTx = txDetail) {
  const milestones = await readMilestones();
  const list = await contractTxns();
  const txs = [];
  const READS_AT_ONCE = 3;
  for (let i = 0; i < list.length; i += READS_AT_ONCE) {
    const batch = await Promise.all(list.slice(i, i + READS_AT_ONCE).map(async (t) => ({ ...t, ns: String(t.ns), ...(await readTx(t.hash).catch(() => ({ events: [], judge: null, signer: null, unread: true }))) })));
    txs.push(...batch);
  }
  return { at: Date.now(), milestones, txs, unread: txs.filter((t) => t.unread).length };
}

/// The index, from the site's shared copy (/api, built once for every visitor) when it
/// answers, otherwise read from the chain in this browser. Kept 30 seconds between pages.
export function loadIndex({ fresh = false } = {}) {
  if (!fresh) {
    const cached = storedIndex();
    if (cached) return Promise.resolve(cached);
    if (indexPromise) return indexPromise;
  }
  indexPromise = (async () => {
    let index = null;
    try {
      const res = await fetch('/api', { signal: AbortSignal.timeout(45000) });
      if (res.ok && (res.headers.get('Content-Type') || '').includes('application/json')) index = await res.json();
    } catch { /* no shared copy: read it here */ }
    if (!index || !Array.isArray(index.txs)) index = await buildIndex();
    index.at = Date.now();
    try { sessionStorage.setItem(INDEX_KEY, JSON.stringify(index)); } catch { /* storage full or blocked: still works */ }
    return index;
  })();
  return indexPromise;
}

/// The numbers on the app page, all from events.
export function stats(index) {
  const all = index.txs.flatMap((t) => t.events);
  const sum = (name, field) => all.filter((e) => e.event === name).reduce((acc, e) => acc + BigInt(e[field] ?? 0), 0n);
  const results = all.filter((e) => e.event === 'review_result');
  return {
    milestones: index.milestones.length,
    escrowed: index.milestones.filter((m) => m.state !== 'Funding').reduce((acc, m) => acc + BigInt(m.budget_total), 0n),
    paid: sum('milestone_paid', 'amount'),
    reviews: results.length,
    secondOpinions: results.filter((e) => e.verifier_call).length,
    refusals: all.filter((e) => e.event === 'claim_refused' || e.event === 'held').length,
  };
}

/// The AI decisions, newest first: each review with its outcome and the judge's reasons.
export function decisions(index) {
  const out = [];
  for (const t of index.txs) {
    const result = t.events.find((e) => e.event === 'review_result');
    if (!result) continue;
    const m = index.milestones.find((x) => x.id === result.milestone_id);
    const paid = t.events.find((e) => e.event === 'milestone_paid' || e.event === 'payout_started');
    const held = t.events.find((e) => e.event === 'held');
    const certified = t.events.find((e) => e.event === 'certified');
    out.push({
      hash: t.hash, ns: t.ns, milestone: m, pr: result.pr, judge: result.judge, verifier: result.verifier,
      verifierCall: result.verifier_call, pct: certified?.pct ?? null, amount: paid?.amount ?? null,
      held: held?.reason ?? null, reasons: t.judge?.reasons ?? [], verifierClaim: t.judge?.verifier ?? null,
    });
  }
  return out;
}

/// Who sent a transaction, as the page shows it.
export function actor(signer, m, relayer) {
  if (signer === relayer) return 'relayer';
  if (m && signer === m.funder) return 'funder';
  if (m?.payee && signer === m.payee.account) return 'contributor';
  return 'anyone';
}
