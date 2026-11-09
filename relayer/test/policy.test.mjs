import assert from 'node:assert/strict';
import { test } from 'node:test';
import { METHODS, parseRequest, precheck } from '../src/policy.mjs';

const refuses = (fn, reason) => assert.throws(fn, (e) => e.reason === reason);
const refusesAsync = (p, reason) => assert.rejects(p, (e) => e.reason === reason);

test('deposits are the contract minimums, fixed per method', () => {
  assert.equal(METHODS.bind_payee.deposit, 61_250_000_000_000_000_000_000n);
  assert.equal(METHODS.claim.deposit, 10_000_000_000_000_000_000_000n);
  assert.equal(METHODS.review.deposit, 60_000_000_000_000_000_000_000n);
});

test('args are rebuilt from parsed values only', () => {
  assert.equal(parseRequest('bind_payee', { milestone_id: 3, gist_id: 'ab12' }).args.toString(), '{"milestone_id":3,"gist_id":"ab12"}');
  assert.equal(parseRequest('claim', { milestone_id: '3', pr_number: 8 }).args.toString(), '{"milestone_id":3,"pr_number":8}');
  assert.equal(parseRequest('review', { milestone_id: 0 }).args.toString(), '{"milestone_id":0}');
});

test('only the three methods', () => {
  for (const m of ['funder_release', 'refund', 'create_milestone', 'ft_transfer', '__proto__', '']) {
    refuses(() => parseRequest(m, { milestone_id: 1 }), 'unknown_method');
  }
});

test('exactly the expected fields: no deposit, gas, receiver or extras', () => {
  refuses(() => parseRequest('review', { milestone_id: 1, deposit: '1' }), 'bad_fields');
  refuses(() => parseRequest('review', { milestone_id: 1, receiver_id: 'x.testnet' }), 'bad_fields');
  refuses(() => parseRequest('claim', { milestone_id: 1 }), 'bad_fields');
  refuses(() => parseRequest('bind_payee', { milestone_id: 1, pr_number: 2 }), 'bad_fields');
  refuses(() => parseRequest('review', [1]), 'bad_body');
  refuses(() => parseRequest('review', null), 'bad_body');
});

test('ids must be non-negative integers', () => {
  for (const bad of [-1, 1.5, '1.5', '-1', '01', '0x10', '1e3', ' 1', '', true, null, {}, 2 ** 53, '99999999999999999999']) {
    refuses(() => parseRequest('review', { milestone_id: bad }), 'bad_milestone_id');
  }
  refuses(() => parseRequest('claim', { milestone_id: 1, pr_number: 'x' }), 'bad_pr_number');
});

test('gist ids must be lowercase hex, 1 to 64 characters', () => {
  for (const bad of ['', 'ABC', 'xyz', 'ab cd', 'ab"}', 'a'.repeat(65), 12, null, '../x']) {
    refuses(() => parseRequest('bind_payee', { milestone_id: 1, gist_id: bad }), 'bad_gist_id');
  }
  parseRequest('bind_payee', { milestone_id: 1, gist_id: 'a'.repeat(64) });
});

const base = {
  repo_id: 42,
  payee_github_id: 7,
  payee: null,
  binding_in_flight: false,
  paid_attempts_used: 0,
  max_paid_attempts: 3,
  fee_remaining: '60000',
  pins: { verifier: { fee: '20000' } },
};
const fakeView = (m, { lock = null, used = false } = {}) => async (method) => {
  if (method === 'get_milestone') return m;
  if (method === 'get_lock') return lock;
  if (method === 'is_pr_used') return used;
  throw new Error(`unexpected view ${method}`);
};
const bound = { account: 'alice.testnet', gist_id: 'ab', gist_revision: 'r' };

test('bind: refused unless Funding/Funded, unbound and not in flight', async () => {
  const req = parseRequest('bind_payee', { milestone_id: 1, gist_id: 'ab' });
  await refusesAsync(precheck(req, fakeView(null)), 'no_such_milestone');
  await refusesAsync(precheck(req, fakeView({ ...base, state: { Checking: { pr: 1 } } })), 'not_bindable');
  await refusesAsync(precheck(req, fakeView({ ...base, state: 'Funded', payee: bound })), 'already_bound');
  await refusesAsync(precheck(req, fakeView({ ...base, state: 'Funding', binding_in_flight: true })), 'binding_in_flight');
  await precheck(req, fakeView({ ...base, state: 'Funding' }));
  await precheck(req, fakeView({ ...base, state: 'Funded' }));
});

test('claim: refused when unbound, at its most paid parts, locked, used, not the approved PR, or not claimable', async () => {
  const req = parseRequest('claim', { milestone_id: 1, pr_number: 8 });
  await refusesAsync(precheck(req, fakeView({ ...base, state: 'Funded' })), 'payee_not_bound');
  await refusesAsync(precheck(req, fakeView({ ...base, state: 'Funded', payee: bound }, { lock: { Locked: 2 } })), 'pr_locked');
  await refusesAsync(precheck(req, fakeView({ ...base, state: 'Funded', payee: bound }, { used: true })), 'pr_used');
  await refusesAsync(precheck(req, fakeView({ ...base, state: { Approved: { pr: 9, head_sha: 'h' } }, payee: bound })), 'not_the_approved_pr');
  await refusesAsync(precheck(req, fakeView({ ...base, state: { Paid: { pr: 8 } }, payee: bound })), 'not_claimable');
  await refusesAsync(precheck(req, fakeView({ ...base, state: 'Refunded', payee: bound })), 'not_claimable');
  const fiveParts = Array.from({ length: 5 }, (_, i) => ({ pr: i, base_sha: 'b', head_sha: 'h', pct: 10 * (i + 1) }));
  await refusesAsync(precheck(req, fakeView({ ...base, state: 'Funded', payee: bound, parts: fiveParts })), 'max_parts_paid');
  await precheck(req, fakeView({ ...base, state: 'Funded', payee: bound, parts: fiveParts.slice(1) }));
  await precheck(req, fakeView({ ...base, state: 'Funded', payee: bound }));
  await precheck(req, fakeView({ ...base, state: { Approved: { pr: 8, head_sha: 'h' } }, payee: bound }));
});

test('review: refused unless HardFactsPassed with attempts and fee left', async () => {
  const req = parseRequest('review', { milestone_id: 1 });
  const passed = { HardFactsPassed: { pr: 8, base_sha: 'b', head_sha: 'h', merged: true, changed_files: 1 } };
  await refusesAsync(precheck(req, fakeView({ ...base, state: 'Funded', payee: bound })), 'not_reviewable');
  await refusesAsync(precheck(req, fakeView({ ...base, state: { Reviewing: passed.HardFactsPassed } })), 'not_reviewable');
  await refusesAsync(precheck(req, fakeView({ ...base, state: passed, paid_attempts_used: 3 })), 'no_paid_attempts_left');
  await refusesAsync(precheck(req, fakeView({ ...base, state: passed, fee_remaining: '19999' })), 'fee_exhausted');
  await precheck(req, fakeView({ ...base, state: passed }));
});
