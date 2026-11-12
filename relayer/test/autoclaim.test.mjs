import assert from 'node:assert/strict';
import { test } from 'node:test';
import { AUTO, createAutoclaim, makeGithub } from '../src/autoclaim.mjs';

const START = 1_800_000_000_000; // ms
const NS = (ms) => String(BigInt(ms) * 1_000_000n);
const AUTHOR = 34985055;

function milestone(over = {}) {
  return {
    repo_owner: 'o', repo_name: 'r', base_branch: 'main', payee_github_id: AUTHOR, repo_id: 7,
    created_at_ns: NS(START - 60_000), state: 'Funded', payee: { account: 'p.testnet' }, ...over,
  };
}

/** A fake chain: milestones by id; claim moves to Checking, then to `after` on the next read. */
function rig({ milestones, prs, after = (pr) => ({ HardFactsPassed: { pr } }), refuse = {}, reviewOutcomes = [] }) {
  let clock = START;
  const sent = [];
  const logs = [];
  const pending = new Map();
  const view = async (method, { milestone_id: id }) => {
    assert.equal(method, 'get_milestone');
    if (pending.has(id)) {
      const step = pending.get(id);
      if (step.reads++ >= 1) milestones[id] = { ...milestones[id], state: step.next };
    }
    return milestones[id] ?? null;
  };
  const submit = async (req) => {
    if (refuse[req.method]) throw Object.assign(new Error(refuse[req.method]), { reason: refuse[req.method] });
    sent.push({ method: req.method, id: req.milestoneId, pr: req.prNumber });
    if (req.method === 'claim') {
      milestones[req.milestoneId] = { ...milestones[req.milestoneId], state: { Checking: { pr: req.prNumber } } };
      pending.set(req.milestoneId, { reads: 0, next: after(req.prNumber) });
    } else {
      // After a review, the next read shows its outcome: by default paid.
      const pr = milestones[req.milestoneId].state.HardFactsPassed?.pr;
      milestones[req.milestoneId] = { ...milestones[req.milestoneId], state: { Reviewing: {} } };
      const outcome = reviewOutcomes.length ? reviewOutcomes.shift() : { Paid: { pr } };
      pending.set(req.milestoneId, { reads: 0, next: outcome === 'retry' ? { HardFactsPassed: { pr } } : outcome });
    }
    return `tx-${sent.length}`;
  };
  const auto = createAutoclaim({
    view, submit, closedPrs: async () => prs, log: (event, f) => logs.push({ event, ...f }),
    now: () => clock, sleep: async (ms) => { clock += ms; },
  });
  return { auto, sent, logs, advance: (ms) => { clock += ms; } };
}

const merged = (number, at, over = {}) => ({ number, mergedAt: at, authorId: AUTHOR, base: 'main', ...over });

test('a bound contributor\'s PR merged after start is claimed, then reviewed', async () => {
  const r = rig({ milestones: [milestone()], prs: [merged(29, START + 5_000)] });
  await Promise.all(await r.auto.poll());
  assert.deepEqual(r.sent, [{ method: 'claim', id: 0, pr: 29 }, { method: 'review', id: 0, pr: undefined }]);
});

test('PRs merged before the relayer started, or before the milestone, are never claimed', async () => {
  const r = rig({
    milestones: [milestone({ created_at_ns: NS(START + 10_000) })],
    prs: [merged(1, START - 1_000), merged(2, START + 5_000)],
  });
  await Promise.all(await r.auto.poll());
  assert.deepEqual(r.sent, []);
});

test('another author, another base branch or an unmerged PR is ignored', async () => {
  const r = rig({
    milestones: [milestone()],
    prs: [merged(3, START + 1, { authorId: 1 }), merged(4, START + 2, { base: 'dev' }), { number: 5, mergedAt: null, authorId: AUTHOR, base: 'main' }],
  });
  await Promise.all(await r.auto.poll());
  assert.deepEqual(r.sent, []);
});

test('unbound, unfunded or finished milestones are skipped', async () => {
  const r = rig({
    milestones: [milestone({ payee: null }), milestone({ state: 'Funding' }), milestone({ state: { Paid: { pr: 1 } } })],
    prs: [merged(6, START + 5_000)],
  });
  await Promise.all(await r.auto.poll());
  assert.deepEqual(r.sent, []);
});

test('the same PR is claimed once, however many polls see it', async () => {
  const r = rig({ milestones: [milestone()], prs: [merged(7, START + 5_000)], after: () => 'Funded' });
  await Promise.all(await r.auto.poll());
  await Promise.all(await r.auto.poll());
  assert.deepEqual(r.sent.filter((s) => s.method === 'claim'), [{ method: 'claim', id: 0, pr: 7 }]);
});

test('two open milestones fit the PR: nothing is claimed (this sent PR #30 to the wrong milestone)', async () => {
  const r = rig({ milestones: [milestone(), milestone()], prs: [merged(8, START + 5_000)] });
  await Promise.all(await r.auto.poll());
  await Promise.all(await r.auto.poll());
  assert.deepEqual(r.sent, []);
  assert.equal(r.logs.filter((l) => l.event === 'auto_claim_ambiguous').length, 1);
});

test('"ProofStream #1" in the PR picks milestone 1 among several', async () => {
  const r = rig({ milestones: [milestone(), milestone()], prs: [merged(12, START + 5_000, { text: 'Add gcd\nFor proofstream #1' })] });
  await Promise.all(await r.auto.poll());
  assert.deepEqual(r.sent.map((s) => [s.method, s.id]), [['claim', 1], ['review', 1]]);
});

test('a PR naming a milestone that does not fit is not claimed anywhere', async () => {
  const r = rig({ milestones: [milestone(), milestone()], prs: [merged(13, START + 5_000, { text: 'ProofStream #5' })] });
  await Promise.all(await r.auto.poll());
  assert.deepEqual(r.sent, []);
});

test('a PR is auto-claimed once ever, even if its milestone goes back to Funded', async () => {
  const r = rig({ milestones: [milestone()], prs: [merged(14, START + 5_000)], after: () => ({ Held: {} }) });
  await Promise.all(await r.auto.poll());
  r.sent.length = 0;
  await Promise.all(await r.auto.poll());
  assert.deepEqual(r.sent, []);
});

test('a refused claim (checker said no) stops without a review', async () => {
  const r = rig({ milestones: [milestone()], prs: [merged(9, START + 5_000)], after: () => 'Funded' });
  await Promise.all(await r.auto.poll());
  assert.deepEqual(r.sent, [{ method: 'claim', id: 0, pr: 9 }]);
  assert.ok(r.logs.some((l) => l.event === 'auto_claim_stopped' && l.state === 'Funded'));
});

test('a claim the relayer refuses (limits, precheck) is logged, not retried', async () => {
  const r = rig({ milestones: [milestone()], prs: [merged(10, START + 5_000)], refuse: { claim: 'rate_limited' } });
  await Promise.all(await r.auto.poll());
  await Promise.all(await r.auto.poll());
  assert.deepEqual(r.sent, []);
  assert.equal(r.logs.filter((l) => l.event === 'auto_claim_refused' && l.reason === 'rate_limited').length, 1);
});

test('a checker that never finishes is given up after followMaxMs', async () => {
  const r = rig({ milestones: [milestone()], prs: [merged(11, START + 5_000)], after: (pr) => ({ Checking: { pr } }) });
  await Promise.all(await r.auto.poll());
  assert.deepEqual(r.sent.map((s) => s.method), ['claim']);
  assert.ok(r.logs.some((l) => l.event === 'auto_claim_timeout'));
});

test('github: sends the token and the ETag, and reuses the list on 304', async () => {
  const calls = [];
  const body = [{ number: 1, merged_at: '2026-11-12T10:00:00Z', user: { id: AUTHOR }, base: { ref: 'main' }, title: 'Add gcd', body: 'ProofStream #9' }];
  const fetchImpl = async (url, init) => {
    calls.push(init.headers);
    if (init.headers['if-none-match'] === '"e1"') return { status: 304, ok: false, headers: new Map() };
    return { status: 200, ok: true, headers: new Map([['etag', '"e1"']]), json: async () => body };
  };
  const closedPrs = makeGithub('t0ken', { fetchImpl });
  const first = await closedPrs('o', 'r');
  const second = await closedPrs('o', 'r');
  assert.equal(calls[0].authorization, 'Bearer t0ken');
  assert.equal(calls[1]['if-none-match'], '"e1"');
  assert.deepEqual(second, first);
  assert.deepEqual(first, [{ number: 1, mergedAt: Date.parse('2026-11-12T10:00:00Z'), authorId: AUTHOR, base: 'main', text: 'Add gcd\nProofStream #9' }]);
});

test('github: an error status throws, so the pass skips that repo', async () => {
  const closedPrs = makeGithub('t', { fetchImpl: async () => ({ status: 403, ok: false, headers: new Map() }) });
  await assert.rejects(closedPrs('o', 'r'), /github 403/);
});

test('a review that loses its second opinion is asked again, up to maxReviews', async () => {
  const r = rig({ milestones: [milestone()], prs: [merged(15, START + 5_000)], reviewOutcomes: ['retry', 'retry'] });
  await Promise.all(await r.auto.poll());
  assert.deepEqual(r.sent.map((s) => s.method), ['claim', 'review', 'review', 'review']);
  assert.ok(r.logs.some((l) => l.event === 'auto_claim_stopped' && l.state === 'Paid' && l.reviews === 3));
});

test('after maxReviews lost second opinions it stops asking', async () => {
  const r = rig({ milestones: [milestone()], prs: [merged(16, START + 5_000)], reviewOutcomes: ['retry', 'retry', 'retry', 'retry'] });
  await Promise.all(await r.auto.poll());
  assert.equal(r.sent.filter((s) => s.method === 'review').length, AUTO.maxReviews);
  assert.ok(r.logs.some((l) => l.event === 'auto_review_gave_up'));
});

test('a held review is left for the funder: no second review', async () => {
  const r = rig({ milestones: [milestone()], prs: [merged(17, START + 5_000)], reviewOutcomes: [{ Held: { pr: 17 } }] });
  await Promise.all(await r.auto.poll());
  assert.deepEqual(r.sent.map((s) => s.method), ['claim', 'review']);
});
