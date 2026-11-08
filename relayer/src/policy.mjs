// What the relayer will sign, and when it refuses. Everything a request can influence is a
// typed, validated argument; receiver, method, deposit and gas are fixed here (D40 cond. 1).

export const CONTRACT = 'proofstream-v1.testnet';

const YOCTO_PER_MILLINEAR = 10n ** 21n;
const YOCTO_PER_MICRONEAR = 10n ** 18n;
const TGAS = 10n ** 12n;
export const GAS = 300n * TGAS;

// Mirrors contract/src/lib.rs (MIN_CLAIM_DEPOSIT, MIN_AI_DEPOSIT, MIN_BIND_COMPUTE,
// TOKEN_STORAGE, IMPLICIT_PROBE). Each is above OutLayer's estimate for its run.
export const COMPUTE_PER_FACTS_RUN = 10n * YOCTO_PER_MILLINEAR;
export const COMPUTE_PER_AI_RUN = 30n * YOCTO_PER_MILLINEAR;
const TOKEN_STORAGE = 1_250n * YOCTO_PER_MICRONEAR;
const IMPLICIT_PROBE = 50n * YOCTO_PER_MILLINEAR;

export const METHODS = Object.freeze({
  bind_payee: { deposit: TOKEN_STORAGE + IMPLICIT_PROBE + COMPUTE_PER_FACTS_RUN, fields: ['milestone_id', 'gist_id'] },
  claim: { deposit: COMPUTE_PER_FACTS_RUN, fields: ['milestone_id', 'pr_number'] },
  review: { deposit: 2n * COMPUTE_PER_AI_RUN, fields: ['milestone_id'] },
});

// Own keys only: `__proto__` and friends are not methods.
export const isMethod = (name) => Object.hasOwn(METHODS, name);

// Resource limits the contract requests (contract/src/lib.rs FACTS_* and AI_*), for the
// startup check against OutLayer's live estimate.
export const FACTS_LIMITS = { max_instructions: 10_000_000_000, max_memory_mb: 128, max_execution_seconds: 20 };
export const AI_LIMITS = { max_instructions: 100_000_000_000, max_memory_mb: 256, max_execution_seconds: 60 };

// u64 on the contract side. Ids count up from 0 and PR numbers are small, so anything above
// 2^53 - 1 can't name a real milestone or PR; refusing it keeps every id an exact JS number.
const DIGITS = /^(0|[1-9][0-9]{0,15})$/;
// GitHub gist ids are lowercase hex; the contract allows up to 64 characters.
const GIST_ID = /^[0-9a-f]{1,64}$/;

export class Refusal extends Error {
  constructor(reason, status = 400) {
    super(reason);
    this.reason = reason;
    this.status = status;
  }
}

function u64(value, name) {
  const n = typeof value === 'string' && DIGITS.test(value) ? Number(value) : value;
  if (!Number.isSafeInteger(n) || n < 0) throw new Refusal(`bad_${name}`);
  return n;
}

/** Request body → { method, milestoneId, prNumber?, gistId?, args (Buffer) }. */
export function parseRequest(method, body) {
  if (!isMethod(method)) throw new Refusal('unknown_method', 404);
  const spec = METHODS[method];
  if (body === null || typeof body !== 'object' || Array.isArray(body)) throw new Refusal('bad_body');
  const keys = Object.keys(body);
  if (keys.length !== spec.fields.length || !spec.fields.every((f) => keys.includes(f))) throw new Refusal('bad_fields');
  const milestoneId = u64(body.milestone_id, 'milestone_id');
  // Args are built from the parsed values only, so nothing else in the body reaches the chain.
  if (method === 'bind_payee') {
    if (typeof body.gist_id !== 'string' || !GIST_ID.test(body.gist_id)) throw new Refusal('bad_gist_id');
    return { method, milestoneId, gistId: body.gist_id, args: Buffer.from(`{"milestone_id":${milestoneId},"gist_id":"${body.gist_id}"}`) };
  }
  if (method === 'claim') {
    const prNumber = u64(body.pr_number, 'pr_number');
    return { method, milestoneId, prNumber, args: Buffer.from(`{"milestone_id":${milestoneId},"pr_number":${prNumber}}`) };
  }
  return { method, milestoneId, args: Buffer.from(`{"milestone_id":${milestoneId}}`) };
}

const stateName = (state) => (typeof state === 'string' ? state : Object.keys(state ?? {})[0]);

/**
 * D40 condition 2: refuse what the contract would refuse, using its own views.
 * `view(method, args)` calls a view on CONTRACT. Returns the milestone for rate limiting.
 */
export async function precheck(req, view) {
  const id = req.milestoneId;
  const m = await view('get_milestone', { milestone_id: id });
  if (!m) throw new Refusal('no_such_milestone', 409);
  const state = stateName(m.state);

  if (req.method === 'bind_payee') {
    if (state !== 'Funding' && state !== 'Funded') throw new Refusal('not_bindable', 409);
    if (m.payee) throw new Refusal('already_bound', 409);
    if (m.binding_in_flight) throw new Refusal('binding_in_flight', 409);
    return m;
  }

  if (req.method === 'claim') {
    if (!m.payee) throw new Refusal('payee_not_bound', 409);
    if (state === 'Funded') {
      const pr = req.prNumber;
      if (await view('get_lock', { repo_id: m.repo_id, pr_number: pr })) throw new Refusal('pr_locked', 409);
      if (await view('is_pr_used', { milestone_id: id, pr_number: pr })) throw new Refusal('pr_used', 409);
      return m;
    }
    if (state === 'Approved') {
      if (m.state.Approved.pr !== req.prNumber) throw new Refusal('not_the_approved_pr', 409);
      return m;
    }
    throw new Refusal('not_claimable', 409);
  }

  if (state !== 'HardFactsPassed') throw new Refusal('not_reviewable', 409);
  if (m.paid_attempts_used >= m.max_paid_attempts) throw new Refusal('no_paid_attempts_left', 409);
  if (BigInt(m.fee_remaining) < BigInt(m.pins.verifier.fee)) throw new Refusal('fee_exhausted', 409);
  return m;
}
