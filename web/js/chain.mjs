// Reading NEAR: contract views over JSON-RPC, a transaction's events, and the contract's
// transaction list from NearBlocks. No wallet needed for any of it.
import { CONTRACT, RPC, NEARBLOCKS, RELAYER, TOKEN } from './config.mjs';

const TIMEOUT_MS = 15000;

async function rpc(method, params, urls = RPC) {
  let last;
  for (const url of urls) {
    for (let attempt = 0; attempt < 3; attempt++) {
      try {
        const res = await fetch(url, {
          method: 'POST',
          headers: { 'Content-Type': 'application/json' },
          body: JSON.stringify({ jsonrpc: '2.0', id: 1, method, params }),
          signal: AbortSignal.timeout(TIMEOUT_MS),
        });
        if (res.status === 429 || res.status >= 500) throw new Error(`rpc ${res.status}`);
        const body = await res.json();
        if (body.error) throw Object.assign(new Error(body.error.cause?.name || body.error.message), { rpc: body.error });
        return body.result;
      } catch (e) {
        last = e;
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

/// ProofStream's NEP-297 events in one transaction, every receipt included.
/// RPC nodes that track every shard accept any sender, so the contract is used.
export async function txEvents(hash, sender = CONTRACT) {
  const r = await rpc('tx', { tx_hash: hash, sender_account_id: sender, wait_until: 'FINAL' }, [...RPC].reverse());
  const events = [];
  for (const o of r.receipts_outcome) {
    for (const log of o.outcome.logs) {
      if (!log.startsWith('EVENT_JSON:')) continue;
      try {
        const e = JSON.parse(log.slice('EVENT_JSON:'.length));
        if (e.standard === 'proofstream') for (const d of e.data) events.push({ event: e.event, ...d });
      } catch { /* not ours */ }
    }
  }
  return events;
}

/// Every transaction that touched the contract, newest first, from NearBlocks' receipts index
/// (funding goes to the token first, so the contract's own tx list would miss it).
export async function contractTxns(limit = 100) {
  const res = await fetch(`${NEARBLOCKS}/accounts/${CONTRACT}/receipts?limit=${limit}`, { signal: AbortSignal.timeout(TIMEOUT_MS) });
  if (!res.ok) throw new Error(`nearblocks ${res.status}`);
  const body = await res.json();
  const seen = new Map();
  for (const r of body.data || []) {
    if (!seen.has(r.transaction_hash)) seen.set(r.transaction_hash, BigInt(r.included_in_block_timestamp ?? r.block?.block_timestamp ?? 0));
  }
  return [...seen].map(([hash, ns]) => ({ hash, ns }));
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
