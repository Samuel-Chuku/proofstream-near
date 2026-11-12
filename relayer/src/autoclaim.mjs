// Auto-claim: when a bound contributor's pull request merges, claim it and ask for the review,
// so nobody has to press a button. It polls GitHub (a webhook would need setup on every repo)
// and sends through the same path as the HTTP endpoints, so the same prechecks, limits and
// daily cap apply. Only PRs merged after this process started are claimed, and only when the
// milestone is unambiguous: the one open milestone that fits, or the one the PR names with
// "ProofStream #N" in its title or description.
import { parseRequest } from './policy.mjs';

export const AUTO = Object.freeze({
  pollMs: 15_000,          // GitHub poll per repository; unchanged lists come back 304 (free)
  followMs: 3_000,         // milestone reads while the checker runs
  followMaxMs: 180_000,    // give up following a claim after 3 minutes
  prsPerRepo: 20,          // most recently updated closed PRs read per poll
  maxMilestones: 500,      // stop scanning ids past this
  githubTimeoutMs: 10_000,
});

/// "ProofStream #9" (any case, optional space) in a PR's title or description names milestone 9.
const NAMES_MILESTONE = /proofstream\s*#(\d+)/gi;
const stateName = (state) => (typeof state === 'string' ? state : Object.keys(state ?? {})[0]);
const TERMINAL = new Set(['Paid', 'Refunded']);
const realSleep = (ms) => new Promise((r) => setTimeout(r, ms));

/** Closed PRs of a public repo, newest updated first, with ETags so unchanged lists cost nothing. */
export function makeGithub(token, { fetchImpl = fetch } = {}) {
  const cache = new Map(); // "owner/repo" -> { etag, prs }
  return async function closedPrs(owner, repo) {
    const key = `${owner}/${repo}`;
    const hit = cache.get(key);
    const res = await fetchImpl(
      `https://api.github.com/repos/${encodeURIComponent(owner)}/${encodeURIComponent(repo)}/pulls?state=closed&sort=updated&direction=desc&per_page=${AUTO.prsPerRepo}`,
      {
        headers: {
          accept: 'application/vnd.github+json',
          authorization: `Bearer ${token}`,
          'user-agent': 'proofstream-relayer',
          'x-github-api-version': '2022-11-28',
          ...(hit ? { 'if-none-match': hit.etag } : {}),
        },
        signal: AbortSignal.timeout(AUTO.githubTimeoutMs),
      },
    );
    if (res.status === 304 && hit) return hit.prs;
    if (!res.ok) throw new Error(`github ${res.status} for ${key}`);
    const prs = (await res.json()).map((p) => ({
      number: p.number,
      mergedAt: p.merged_at ? Date.parse(p.merged_at) : null,
      authorId: p.user?.id ?? null,
      base: p.base?.ref ?? null,
      text: `${p.title ?? ''}\n${(p.body ?? '').slice(0, 4000)}`,
    }));
    const etag = res.headers.get('etag');
    if (etag) cache.set(key, { etag, prs });
    return prs;
  };
}

/**
 * deps: { view(method, args), submit(req) -> tx hash, closedPrs(owner, repo), log(event, fields),
 *         now?, sleep? }
 */
export function createAutoclaim(deps) {
  const now = deps.now ?? Date.now;
  const sleep = deps.sleep ?? realSleep;
  const startedMs = now();
  const tried = new Set();     // "owner/repo#pr": each merged PR gets one auto-claim, ever
  const finished = new Set();  // milestone ids in a terminal state
  const following = new Set(); // milestone ids with a claim in flight
  let nextId = 0;

  async function milestones() {
    while (nextId < AUTO.maxMilestones && (await deps.view('get_milestone', { milestone_id: nextId }))) nextId++;
    const out = [];
    for (let id = 0; id < nextId; id++) {
      if (finished.has(id)) continue;
      const m = await deps.view('get_milestone', { milestone_id: id });
      if (!m) continue;
      if (TERMINAL.has(stateName(m.state))) finished.add(id);
      else out.push({ id, ...m });
    }
    return out;
  }

  /// Claim, then wait for the checker and ask for the review. Refusals end it quietly.
  async function claimAndReview(m, pr) {
    following.add(m.id);
    try {
      let tx;
      try {
        tx = await deps.submit(parseRequest('claim', { milestone_id: m.id, pr_number: pr }));
      } catch (e) {
        deps.log('auto_claim_refused', { milestone_id: m.id, pr, reason: e.reason ?? e.message });
        return;
      }
      deps.log('auto_claim_sent', { milestone_id: m.id, pr, tx });
      const deadline = now() + AUTO.followMaxMs;
      while (now() < deadline) {
        await sleep(AUTO.followMs);
        const cur = await deps.view('get_milestone', { milestone_id: m.id });
        const name = stateName(cur?.state);
        if (name === 'Checking') continue;
        if (name !== 'HardFactsPassed' || cur.state.HardFactsPassed.pr !== pr) {
          deps.log('auto_claim_stopped', { milestone_id: m.id, pr, state: name });
          return;
        }
        try {
          const rtx = await deps.submit(parseRequest('review', { milestone_id: m.id }));
          deps.log('auto_review_sent', { milestone_id: m.id, pr, tx: rtx });
        } catch (e) {
          deps.log('auto_review_refused', { milestone_id: m.id, pr, reason: e.reason ?? e.message });
        }
        return;
      }
      deps.log('auto_claim_timeout', { milestone_id: m.id, pr });
    } finally {
      following.delete(m.id);
    }
  }

  /// One pass: every Funded, bound milestone, grouped by repository. Returns the follows it started.
  async function poll() {
    const open = (await milestones()).filter((m) => stateName(m.state) === 'Funded' && m.payee && !following.has(m.id));
    const busy = new Set();
    const byRepo = new Map();
    for (const m of open) {
      const key = `${m.repo_owner}/${m.repo_name}`;
      byRepo.set(key, [...(byRepo.get(key) ?? []), m]);
    }
    const started = [];
    for (const [key, ms] of byRepo) {
      let prs;
      try {
        prs = await deps.closedPrs(ms[0].repo_owner, ms[0].repo_name);
      } catch (e) {
        deps.log('auto_github_failed', { repo: key, error: e.message });
        continue;
      }
      const fresh = prs.filter((p) => p.mergedAt !== null && p.mergedAt > startedMs && !tried.has(`${key}#${p.number}`));
      for (const pr of fresh.sort((a, b) => a.mergedAt - b.mergedAt)) {
        const fits = ms.filter((m) => pr.authorId === m.payee_github_id && pr.base === m.base_branch
          && pr.mergedAt > Number(BigInt(m.created_at_ns) / 1_000_000n));
        if (!fits.length) continue;
        const named = [...(pr.text ?? '').matchAll(NAMES_MILESTONE)].map((x) => Number(x[1]));
        const pick = named.length ? fits.filter((m) => named.includes(m.id)) : fits;
        if (pick.length === 1 && busy.has(pick[0].id)) continue; // claiming another PR now: next pass
        tried.add(`${key}#${pr.number}`);
        if (pick.length !== 1) {
          deps.log('auto_claim_ambiguous', { repo: key, pr: pr.number, fits: fits.map((m) => m.id), named });
          continue;
        }
        busy.add(pick[0].id); // one claim per milestone per pass
        started.push(claimAndReview(pick[0], pr.number));
      }
    }
    return started;
  }

  async function run() {
    for (;;) {
      try {
        await poll();
      } catch (e) {
        deps.log('auto_poll_failed', { error: e.message });
      }
      await sleep(AUTO.pollMs);
    }
  }

  return { poll, run };
}
