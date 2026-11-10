// One milestone: state, the certified share, details, the event log, and the funder's actions.
import { CONTRACT, PROJECTS, VERIFIER_PUBLIC_KEY, OUTLAYER_API, GAS_300 } from './config.mjs';
import { getMilestone, nextMilestoneId, contractTxns, txEvents, stateOf } from './chain.mjs';
import { $, h, shell, amount, usdc, txLink, short, problem, ago } from './ui.mjs';
import * as wallet from './wallet.mjs';

shell('milestone');
const id = new URLSearchParams(location.search).get('id');

const STATE_TEXT = {
  Funding: 'Created; waiting for the funder to deposit the budget and fee.',
  Funded: 'Funded. Open to a claim from the contributor.',
  Checking: 'The checker is reading GitHub inside TDX.',
  HardFactsPassed: 'The hard facts passed. Waiting for a review.',
  Reviewing: 'The judge and the verifier are reviewing.',
  Approved: 'Both AIs certified the open PR. It pays once merged at that commit.',
  Held: 'Held for the funder: an objection, a disagreement, or no new progress.',
  PayingOut: 'Paying out.',
  PayoutFailed: 'The payout transfer failed. Anyone may retry it.',
  Paid: 'Paid in full.',
  Refunded: 'Refunded to the funder after the deadline.',
};

async function standardBuilds() {
  const builds = await (await fetch('builds.json')).json();
  return Object.fromEntries(builds.map((b) => [b.project, b.sha256]));
}

function nonStandard(m, builds) {
  const p = m.pins;
  const off = [];
  if (p.checker.wasm_hash !== builds.checker || p.checker.project_id !== PROJECTS.checker) off.push('checker');
  if (p.binder.wasm_hash !== builds.binder || p.binder.project_id !== PROJECTS.binder) off.push('binder');
  if (p.judge.wasm_hash !== builds['ai-review'] || p.judge.project_id !== PROJECTS.judge) off.push('judge');
  if (p.verifier.wasm_hash !== builds['ai-review'] || p.verifier.project_id !== PROJECTS.verifier || p.verifier.public_key !== VERIFIER_PUBLIC_KEY) off.push('verifier');
  return off;
}

function bar(m) {
  const pct = m.certified_pct;
  const cells = Array.from({ length: 10 }, (_, i) => h('span', { class: `ps-bar-cell ${i < Math.floor(pct / 10) ? 'ps-fill-unlocked' : 'ps-fill-locked'}` }));
  const el = $('#bar');
  el.replaceChildren(...cells);
  el.setAttribute('aria-label', `${pct} percent certified and released`);
}

function detail(term, ...value) {
  return [h('dt', {}, term), h('dd', {}, ...value)];
}

async function githubLogin(numericId) {
  try {
    const r = await fetch(`https://api.github.com/user/${numericId}`, { signal: AbortSignal.timeout(8000) });
    return r.ok ? (await r.json()).login : null;
  } catch { return null; }
}

function describe(e) {
  const a = (x) => `${usdc(x)} USDC`;
  switch (e.event) {
    case 'milestone_created': return `budget ${a(e.budget)}, fee ${a(e.fee_budget)}`;
    case 'milestone_funded': return a(e.amount);
    case 'bind_started': return `gist ${e.gist_id}`;
    case 'payee_bound': return `${e.account} (gist revision ${short(e.gist_revision)})`;
    case 'bind_refused': case 'claim_refused': case 'review_refused': return e.reason;
    case 'claim_started': return `PR #${e.pr}${e.from_approved ? ', approved before merge' : ''}`;
    case 'hard_facts_passed': return `PR #${e.pr} at ${short(e.head_sha)}${e.merged ? ', merged' : ', open'}`;
    case 'review_started': return `attempt ${e.attempt}, fee ${a(e.fee)}`;
    case 'review_result': return `judge ${e.judge}, verifier ${e.verifier}`;
    case 'review_retry': return `no verdict; ${e.attempts_left} attempts left`;
    case 'certified': return `${e.pct}% of the milestone`;
    case 'held': return e.reason;
    case 'payout_started': return `${a(e.amount)} to ${e.to} (${e.pct}%)`;
    case 'milestone_paid': return `${a(e.amount)} to ${e.to}, ${e.pct}% certified`;
    case 'funder_released': return `${e.pct}%`;
    case 'funder_rejected': return `PR #${e.pr}: ${e.reason}`;
    case 'refund_started': case 'milestone_refunded': return a(e.amount);
    default: return '';
  }
}

async function log(mid) {
  const status = $('#log-status');
  try {
    const txs = await contractTxns(100);
    const rows = [];
    for (let i = 0; i < txs.length; i += 4) {
      const batch = await Promise.all(txs.slice(i, i + 4).map(async (t) => ({ t, events: await txEvents(t.hash).catch(() => []) })));
      for (const { t, events } of batch) {
        // Newest first throughout: transactions come newest first, so their events are reversed too.
        for (const e of events.filter((x) => x.milestone_id === mid).reverse()) rows.push({ t, e });
      }
    }
    status.textContent = rows.length ? '' : 'No events yet.';
    $('#log').replaceChildren(...rows.map(({ t, e }) => h('li', { class: 'ps-tx-row' },
      h('span', { class: 'ps-tx-time' }, ago(t.ns)),
      h('span', { class: 'ps-tx-action' }, e.event.replaceAll('_', ' ')),
      h('span', { class: 'ps-tx-detail' }, describe(e)),
      h('span', { class: 'ps-chip-group' }, txLink(t.hash),
        e.verifier_call ? h('a', { href: `${OUTLAYER_API}/attestations/by-call/${e.verifier_call}`, class: 'ps-chip ps-chip-link', target: '_blank', rel: 'noopener' }, 'VERIFIER ATTESTATION ↗') : null))));
  } catch (err) {
    status.textContent = `Couldn't read the log (${err.message}). The state above is read from the contract directly.`;
  }
}

function funderPanel(m, name, fields, account) {
  const box = $('#funder');
  const show = account === m.funder;
  $('#funder-rule').hidden = !show;
  box.hidden = !show;
  if (!show) return;
  const out = h('div');
  const run = async (label, calls) => {
    out.replaceChildren(h('p', { class: 'ps-detail' }, `Confirm "${label}" in your wallet…`));
    try {
      await wallet.send([{ receiverId: CONTRACT, calls }]);
      location.reload();
    } catch (e) { out.replaceChildren(problem(e.message || String(e))); }
  };
  const call = (method, args) => ({ method, args, gas: GAS_300, deposit: '0' });
  const parts = [];
  if (name === 'Held') {
    const pct = h('input', { class: 'ps-input ps-input-short', inputmode: 'numeric', value: '100', 'aria-label': 'Release this share, in percent' });
    const reason = h('input', { class: 'ps-input', placeholder: 'Why the PR is rejected (shown on-chain)', maxlength: '280' });
    parts.push(h('p', { class: 'ps-detail' }, `Held: ${fields.reason}. Release a share of the milestone, or reject the PR.`),
      h('div', { class: 'ps-actions' }, h('span', { class: 'ps-caption' }, 'Certify to %'), pct,
        h('button', { class: 'ps-button ps-button-primary', type: 'button', onclick: () => run('release', [call('funder_release', { milestone_id: Number(id), percent: Number(pct.value) })]) }, 'Release')),
      h('div', { class: 'ps-field' }, reason),
      h('button', { class: 'ps-button', type: 'button', onclick: () => run('reject', [call('funder_reject', { milestone_id: Number(id), reason: reason.value })]) }, 'Reject the PR'));
  }
  if (name === 'Approved') {
    const reason = h('input', { class: 'ps-input', placeholder: 'Why the PR is rejected (shown on-chain)', maxlength: '280' });
    parts.push(h('div', { class: 'ps-field' }, reason), h('button', { class: 'ps-button', type: 'button', onclick: () => run('reject', [call('funder_reject', { milestone_id: Number(id), reason: reason.value })]) }, 'Reject the approval'));
  }
  const afterDeadline = Date.now() * 1e6 >= Number(m.deadline_ns);
  if (['Funding', 'Funded', 'PayoutFailed', 'Approved', 'Paid'].includes(name)) {
    const can = name === 'Paid' ? BigInt(m.fee_remaining) + BigInt(m.fee_refundable) > 0n : afterDeadline;
    parts.push(h('div', { class: 'ps-actions' }, h('button', { class: 'ps-button', type: 'button', disabled: !can, onclick: () => run('refund', [call('refund', { milestone_id: Number(id) })]) }, name === 'Paid' ? 'Refund unspent fee' : 'Refund the uncertified rest'),
      can ? null : h('span', { class: 'ps-caption' }, 'after the deadline')));
  }
  box.replaceChildren(...parts, out);
}

async function showOne(mid) {
  $('#view').hidden = false;
  const [m, builds] = await Promise.all([getMilestone(mid), standardBuilds()]);
  if (!m) { $('#view').replaceChildren(problem(`There is no milestone #${mid} on ${CONTRACT}.`)); return; }
  const [name, fields] = stateOf(m);
  document.title = `Milestone #${mid} · ProofStream`;
  $('#crumb').textContent = `MILESTONE #${mid} · ${m.repo_owner}/${m.repo_name}`;
  $('#title').textContent = name.replace(/([a-z])([A-Z])/g, '$1 $2');
  const deadline = new Date(Number(BigInt(m.deadline_ns) / 1_000_000n));
  $('#meta').replaceChildren(h('span', {}, `DEADLINE ${deadline.toISOString().slice(0, 16).replace('T', ' ')} UTC`), h('span', {}, `ATTEMPTS ${m.paid_attempts_used}/${m.max_paid_attempts}`));
  const total = BigInt(m.budget_total);
  $('#paid').replaceChildren(amount(total - BigInt(m.budget), 'xl'));
  $('#total').replaceChildren(amount(total, 'm'));
  bar(m);
  $('#text').textContent = STATE_TEXT[name] ?? '';
  const off = nonStandard(m, builds);
  if (off.length) $('#notice').replaceChildren(problem(`Non-standard build: ${off.join(', ')}.`, 'This milestone pins code or a key that is not in the published reproducible builds (builds.json). Check what it runs before relying on it.'));
  const claimable = ['Funding', 'Funded', 'HardFactsPassed', 'Approved'].includes(name);
  $('#actions').replaceChildren(...[claimable ? h('a', { class: 'ps-button', href: `claim.html?id=${mid}` }, 'Contributor: bind and claim') : null,
    h('button', { class: 'ps-button', type: 'button', id: 'connect', onclick: async () => { await wallet.connect(); } }, 'Funder: connect wallet')].filter(Boolean));
  const login = await githubLogin(m.payee_github_id);
  $('#sheet').replaceChildren(
    ...detail('Repository', h('a', { href: `https://github.com/${m.repo_owner}/${m.repo_name}` }, `${m.repo_owner}/${m.repo_name}`), ` · id ${m.repo_id} · branch ${m.base_branch}`),
    ...detail('Milestone', h('span', { class: 'ps-detail' }, m.milestone_text)),
    ...detail('Contributor', login ? h('a', { href: `https://github.com/${login}` }, login) : '', ` (GitHub id ${m.payee_github_id})`),
    ...detail('Pays to', m.payee ? m.payee.account : 'not bound yet'),
    ...detail('Certified', `${m.certified_pct}% · ${m.parts.length} paid PR${m.parts.length === 1 ? '' : 's'}${m.parts.length ? ` (#${m.parts.map((p) => p.pr).join(', #')})` : ''}`),
    ...detail('Required checks', m.required_checks.join(', ')),
    ...detail('Review fee left', amount(m.fee_remaining, 's')),
    ...detail('Funder', m.funder),
    ...detail('Self-merge', m.allow_self_merge ? 'allowed' : 'needs an independent merge or approval'),
    ...detail('Pinned builds', h('span', { class: 'ps-mono-id' }, `checker ${short(m.pins.checker.wasm_hash, 12)} · binder ${short(m.pins.binder.wasm_hash, 12)} · AI ${short(m.pins.judge.wasm_hash, 12)} · verifier key ${short(m.pins.verifier.public_key, 12)}`), off.length ? '' : h('span', { class: 'ps-caption' }, ' · standard')));
  wallet.onAccount((acct) => { funderPanel(m, name, fields, acct); const b = $('#connect'); if (b && acct) b.textContent = acct; });
  wallet.restore().catch(() => {});
  log(Number(mid));
}

async function showList() {
  $('#pick').hidden = false;
  const n = await nextMilestoneId();
  const items = [];
  for (let i = n - 1; i >= Math.max(0, n - 20); i--) {
    const m = await getMilestone(i);
    const [name] = stateOf(m);
    items.push(h('li', { class: 'ps-tx-row' }, h('span', { class: 'ps-tx-time' }, `#${i}`), h('span', { class: 'ps-tx-action' }, name),
      h('span', { class: 'ps-tx-detail' }, `${m.repo_owner}/${m.repo_name} · ${usdc(m.budget_total)} USDC · ${m.certified_pct}% certified`),
      h('a', { class: 'ps-chip ps-chip-link', href: `milestone.html?id=${i}` }, 'OPEN')));
  }
  $('#list').replaceChildren(...(items.length ? items : [h('li', { class: 'ps-detail' }, 'No milestones yet.')]));
}

(id !== null && /^\d+$/.test(id) ? showOne(Number(id)) : showList()).catch((e) => $('main').prepend(problem(`Couldn't read the contract: ${e.message}`)));
