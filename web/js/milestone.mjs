// One milestone, laid out like an Arc stream page: state, money, actions, the milestone,
// the paid PRs, the AI decisions with their reasons, and every transaction.
import { CONTRACT, PROJECTS, VERIFIER_PUBLIC_KEY, VERIFIER_FEE, OUTLAYER_API, GAS_300, RELAYER_ACCOUNT } from './config.mjs';
import { loadIndex, stateOf, decisions, actor, getMilestone } from './chain.mjs';
import { $, h, fill, shell, amount, usdc, txLink, short, problem, ago, loading, stateChip, sectionRule, verdictText, retryLabel } from './ui.mjs';
import * as wallet from './wallet.mjs';

shell('app');
const id = new URLSearchParams(location.search).get('id');
const page = $('#page');

const STATE_TEXT = {
  Funding: 'Created. Waiting for the funder to deposit the budget and the review fee; nothing is claimable until it lands.',
  Funded: 'Funded and open. The contributor claims with a merged pull request.',
  Checking: 'The checker is reading GitHub inside TDX.',
  HardFactsPassed: 'The hard facts passed. Waiting for the review.',
  Reviewing: 'The judge is reviewing, then buying the verifier\'s signed opinion.',
  Approved: 'Both agents certified the open pull request. It pays once merged at that exact commit.',
  Held: 'Held for the funder: an objection, a disagreement, or no new progress. The funder can release a share or reject the pull request.',
  PayingOut: 'Paying out.',
  PayoutFailed: 'The payout transfer failed. Anyone may retry it.',
  Paid: 'Fully earned. Everything certified has been paid.',
  Refunded: 'Refunded to the funder after the deadline.',
};

const ACTION = {
  milestone_created: 'CREATE', milestone_funded: 'FUND', payee_bound: 'BIND', bind_refused: 'BIND REFUSED',
  claim_refused: 'CLAIM REFUSED', hard_facts_passed: 'CLAIM', review_result: 'REVIEW', milestone_paid: 'PAY',
  funder_rejected: 'REJECT', funder_released: 'RELEASE', milestone_refunded: 'REFUND', verdict_approved_premerge: 'APPROVE',
};

function nonStandard(m, builds) {
  const p = m.pins;
  const off = [];
  if (p.checker.wasm_hash !== builds.checker || p.checker.project_id !== PROJECTS.checker) off.push('checker');
  if (p.binder.wasm_hash !== builds.binder || p.binder.project_id !== PROJECTS.binder) off.push('binder');
  if (p.judge.wasm_hash !== builds['ai-review'] || p.judge.project_id !== PROJECTS.judge) off.push('judge');
  if (p.verifier.wasm_hash !== builds['ai-review'] || p.verifier.project_id !== PROJECTS.verifier || p.verifier.public_key !== VERIFIER_PUBLIC_KEY) off.push('verifier');
  return off;
}

function cells(m, name, fields) {
  const total = 50;
  const paidCells = Math.round((m.certified_pct / 100) * total);
  const approved = name === 'Approved' ? Math.round((fields.pct / 100) * total) : 0;
  const bar = h('div', { class: 'ps-cells', role: 'img', 'aria-label': `${m.certified_pct}% certified and paid` });
  for (let i = 0; i < total; i++) {
    const cls = i < paidCells ? 'ps-fill-unlocked' : i < approved ? 'ps-fill-valued' : 'ps-fill-locked';
    bar.append(h('span', { class: cls }));
  }
  return h('div', {}, bar,
    h('p', { class: 'ps-caption' }, 'PAID OUT TO THE CONTRIBUTOR'),
    h('div', { class: 'ps-bar-legend ps-caption' },
      h('span', {}, h('span', { class: 'ps-swatch ps-fill-unlocked' }), 'PAID'),
      h('span', {}, h('span', { class: 'ps-swatch ps-fill-valued' }), 'CERTIFIED, PAYS AT MERGE'),
      h('span', {}, h('span', { class: 'ps-swatch ps-fill-locked' }), 'LOCKED IN ESCROW')),
    h('p', { class: 'ps-caption' }, '1 CELL = 2% OF THE BUDGET'));
}

function fact(label, value) {
  return h('div', { class: 'ps-fact' }, h('span', { class: 'ps-caption' }, label), h('span', {}, value));
}

async function githubLogin(numericId) {
  try {
    const r = await fetch(`https://api.github.com/user/${numericId}`, { signal: AbortSignal.timeout(8000) });
    return r.ok ? (await r.json()).login : null;
  } catch { return null; }
}

function decisionCard(d, m) {
  const outcome = d.amount ? h('span', { class: 'ps-state ps-state-paid' }, `CERTIFIED ${d.pct}%`)
    : d.held ? h('span', { class: 'ps-state ps-state-held' }, 'HELD') : h('span', { class: 'ps-state ps-state-open' }, retryLabel(d));
  const claim = d.verifierClaim && !d.verifierClaim.error ? d.verifierClaim : null;
  return h('details', { class: `ps-card${d.amount ? ' ps-card-paid' : ''}` },
    h('summary', {}, h('span', { class: 'ps-label' }, `PR #${d.pr}`), outcome,
      h('span', { class: 'ps-detail ps-ellipsis' }, d.reasons[0] ?? ''),
      h('span', { class: 'ps-caption' }, d.amount ? `${usdc(d.amount)} USDC ▾` : '▾')),
    h('div', { class: 'ps-card-body' },
      h('p', { class: 'ps-subhead' }, `Judge · ${verdictText(d.judge)}`),
      d.reasons.length ? h('ul', { class: 'ps-reasons' }, ...d.reasons.map((r) => h('li', {}, r))) : h('p', { class: 'ps-detail' }, 'No reasons recorded.'),
      h('p', { class: 'ps-subhead' }, `Verifier · ${verdictText(d.verifier)}`),
      claim
        ? h('p', { class: 'ps-detail' }, `Signed verdict: ${claim.verdict}, ${claim.percent_done}%, paid for by ${claim.caller}. Signature ${short(claim.signature, 16)} checked on-chain against the verifier key ${short(m.pins.verifier.public_key, 12)}.`)
        : h('p', { class: 'ps-detail' }, d.verifierClaim?.error ? `Not obtained: ${d.verifierClaim.error}.` : 'Not asked: the judge objected first, so no second opinion was bought.'),
      h('p', { class: 'ps-detail' }, d.amount ? `The lower share counts: ${d.pct}% of the milestone is now certified.` : d.held ? `Held: ${d.held}.` : 'No payout; the attempt was counted.'),
      h('div', { class: 'ps-chip-group' }, txLink(d.hash, 'REVIEW TX ↗'),
        d.verifierCall ? h('a', { class: 'ps-chip ps-chip-link', href: `${OUTLAYER_API}/attestations/by-call/${d.verifierCall}`, target: '_blank', rel: 'noopener' }, 'VERIFIER ATTESTATION ↗') : null)));
}

function txRows(rows) {
  if (!rows.length) return h('p', { class: 'ps-detail' }, 'None yet.');
  return h('ul', { class: 'ps-feed' }, ...rows.map(({ t, e, label }) => h('li', { class: 'ps-tx-row' },
    h('span', { class: 'ps-tx-time' }, ago(t.ns)),
    h('span', { class: 'ps-tx-action' }, label),
    h('span', { class: 'ps-tx-detail' }, describe(e)),
    txLink(t.hash))));
}

function describe(e) {
  const a = (x) => `${usdc(x)} USDC`;
  switch (e.event) {
    case 'milestone_created': return `budget ${a(e.budget)} · review fee ${a(e.fee_budget)}`;
    case 'milestone_funded': return a(e.amount);
    case 'payee_bound': return `pays ${e.account} · gist ${short(e.gist_id, 10)}`;
    case 'bind_refused': case 'claim_refused': return e.reason;
    case 'hard_facts_passed': return `PR #${e.pr} at ${short(e.head_sha)}${e.merged ? ', merged' : ', open'}`;
    case 'review_result': return `judge ${verdictText(e.judge)} · verifier ${verdictText(e.verifier)}`;
    case 'milestone_paid': return `${a(e.amount)} to ${e.to} · ${e.pct}% certified`;
    case 'funder_rejected': return `PR #${e.pr}: ${e.reason}`;
    case 'funder_released': return `${e.pct}%`;
    case 'milestone_refunded': return a(e.amount);
    default: return '';
  }
}

function funderActions(m, name, fields, box) {
  const out = h('div');
  const run = async (label, method, args) => {
    out.replaceChildren(h('p', { class: 'ps-detail' }, `Confirm "${label}" in your wallet…`));
    try { await wallet.send([{ receiverId: CONTRACT, calls: [{ method, args, gas: GAS_300, deposit: '0' }] }]); sessionStorage.clear(); location.reload(); }
    catch (e) { out.replaceChildren(problem(e.message || String(e))); }
  };
  const parts = [h('p', { class: 'ps-caption' }, 'YOU FUNDED THIS MILESTONE')];
  if (name === 'Held' || name === 'Approved') {
    const reason = h('input', { class: 'ps-input', placeholder: 'Why the pull request is rejected (recorded on-chain)', maxlength: '280' });
    if (name === 'Held') {
      const pct = h('input', { class: 'ps-input ps-input-short', inputmode: 'numeric', value: '100', 'aria-label': 'Certify the milestone to this percent' });
      parts.push(h('p', { class: 'ps-detail' }, `Held: ${fields.reason}. Release a share of the milestone, or reject the pull request.`),
        h('div', { class: 'ps-actions' }, h('span', { class: 'ps-caption' }, 'CERTIFY TO %'), pct,
          h('button', { class: 'ps-button ps-button-primary', type: 'button', onclick: () => run('release', 'funder_release', { milestone_id: Number(id), percent: Number(pct.value) }) }, '[ Release ]')));
    }
    parts.push(h('div', { class: 'ps-field' }, reason), h('div', { class: 'ps-actions' }, h('button', { class: 'ps-button', type: 'button', onclick: () => run('reject', 'funder_reject', { milestone_id: Number(id), reason: reason.value }) }, '[ Reject the pull request ]')));
  }
  const afterDeadline = Date.now() * 1e6 >= Number(m.deadline_ns);
  if (['Funding', 'Funded', 'PayoutFailed', 'Approved', 'Paid'].includes(name)) {
    const leftover = BigInt(m.fee_remaining) + BigInt(m.fee_refundable);
    const can = name === 'Paid' ? leftover > 0n : afterDeadline;
    parts.push(h('div', { class: 'ps-actions' },
      h('button', { class: 'ps-button', type: 'button', disabled: !can, onclick: () => run('refund', 'refund', { milestone_id: Number(id) }) }, name === 'Paid' ? `[ Refund unspent fee (${usdc(leftover)}) ]` : '[ Refund the uncertified rest ]'),
      can ? null : h('span', { class: 'ps-caption' }, 'AFTER THE DEADLINE')));
  }
  box.replaceChildren(...parts, out);
}

/// The parts that need the chain index (every transaction's events and the judge's output).
/// They render after the milestone itself, which needs one RPC view call.
function evidence(index, m) {
  const mid = m.id;
  const mine = index.txs.filter((t) => t.events.some((e) => e.milestone_id === mid));
  const ds = decisions(index).filter((d) => d.milestone?.id === mid);
  const rows = mine.flatMap((t) => t.events.filter((e) => e.milestone_id === mid && ACTION[e.event]).map((e) => ({ t, e, label: ACTION[e.event], who: actor(t.signer, m, RELAYER_ACCOUNT) })));
  const human = rows.filter((r) => r.who === 'funder');
  const automated = rows.filter((r) => r.who !== 'funder');
  const opinions = ds.filter((d) => d.verifierCall).length;
  const refused = ds.filter((d) => !d.verifierCall && /Objects/.test(d.judge)).length;
  const unread = index.unread ? h('p', { class: 'ps-caption' }, `${index.unread} TRANSACTION${index.unread === 1 ? '' : 'S'} NOT READ YET · RELOAD IN A MINUTE`) : null;
  return {
    prs: prTable(m, mine),
    ai: [unread, ds.length ? h('div', {}, ...ds.map((d) => decisionCard(d, m))) : h('p', { class: 'ps-detail' }, 'No reviews yet.')],
    txs: [h('p', { class: 'ps-caption' }, 'NEWEST FIRST · EVERY ROW LINKS TO NEARBLOCKS, SO NOTHING HERE HAS TO BE TAKEN ON TRUST'),
      h('p', { class: 'ps-subhead' }, 'By the contract, the relayer and the contributor'), txRows(automated),
      h('p', { class: 'ps-subhead' }, 'By the funder'), txRows(human)],
    fee: [h('p', { class: 'ps-body' }, h('strong', {}, `${usdc(BigInt(VERIFIER_FEE) * BigInt(opinions))} USDC`), ` for ${opinions} second opinion${opinions === 1 ? '' : 's'}, from the judge owner's OutLayer payment key to the verifier's owner: one agent paying another for a service. Each review pays the judge's owner that fee from the milestone's review budget; the judge passes it on only when it asks for a second opinion.`),
      h('p', { class: 'ps-caption' }, `${refused} REVIEW${refused === 1 ? '' : 'S'} WHERE THE JUDGE OBJECTED: NO SECOND OPINION BOUGHT, THE FEE STAYED WITH THE JUDGE'S OWNER`)],
  };
}

/// Paid pull requests. Without the index (mine = null) the paid amount and transaction are left blank.
function prTable(m, mine) {
  if (!m.parts.length) return h('p', { class: 'ps-detail' }, 'Nothing paid yet.');
  return h('table', { class: 'ps-table' }, h('thead', {}, h('tr', {}, h('th', {}, 'PR'), h('th', {}, 'Certified after it'), h('th', {}, 'Paid for it'), h('th', {}, ''))),
    h('tbody', {}, ...m.parts.map((p, i) => {
      const prev = i ? m.parts[i - 1].pct : 0;
      const paidRow = (mine ?? []).flatMap((t) => t.events.filter((e) => e.event === 'milestone_paid' && e.pr === p.pr && e.milestone_id === m.id).map((e) => ({ t, e })))[0];
      return h('tr', {}, h('td', {}, h('a', { href: `https://github.com/${m.repo_owner}/${m.repo_name}/pull/${p.pr}`, target: '_blank', rel: 'noopener' }, `#${p.pr} ↗`)),
        h('td', {}, `${p.pct}% (+${p.pct - prev})`), h('td', { class: 'ps-num' }, paidRow ? `${usdc(paidRow.e.amount)} USDC` : ''), h('td', {}, paidRow ? txLink(paidRow.t.hash) : ''));
    })));
}

async function main() {
  page.replaceChildren(loading('READING THE MILESTONE'));
  if (id === null || !/^\d+$/.test(id)) { location.replace('app.html#milestones'); return; }
  const mid = Number(id);
  const indexP = loadIndex();
  indexP.catch(() => {}); // handled where it is awaited, below
  const [builds, fresh] = await Promise.all([fetch('builds.json').then((r) => r.json()).then((b) => Object.fromEntries(b.map((x) => [x.project, x.sha256]))), getMilestone(mid)]);
  if (!fresh) { page.replaceChildren(problem(`There is no milestone #${mid} on ${CONTRACT}.`)); return; }
  const m = { id: mid, ...fresh };
  const [name, fields] = stateOf(m);
  document.title = `#${mid} ${m.repo_name} · ProofStream`;
  const total = BigInt(m.budget_total);
  const paid = total - BigInt(m.budget);
  const off = nonStandard(m, builds);
  const actions = h('div');
  const contributor = h('dd', {}, `GitHub id ${m.payee_github_id}`);
  const slot = (word) => h('div', {}, loading(word, { inline: true }));
  const prs = h('div', {}, prTable(m, null));
  const ai = slot('READING THE REVIEWS');
  const txs = slot('READING THE TRANSACTIONS');
  const fee = slot('ADDING UP THE FEES');

  fill(page,
    h('header', { class: 'ps-masthead' }, h('div', {},
      h('h1', { class: 'ps-display-l' }, m.repo_name),
      h('div', { class: 'ps-chips' },
        h('span', { class: 'ps-chip ps-chip-current' }, `MILESTONE #${mid}`),
        h('a', { class: 'ps-chip ps-chip-link', href: `https://github.com/${m.repo_owner}/${m.repo_name}/tree/${m.base_branch}`, target: '_blank', rel: 'noopener' }, `${m.repo_owner}/${m.repo_name}#${m.base_branch} ↗`),
        stateChip(name, m),
        h('span', { class: `ps-tag${off.length ? '' : ' ps-tag-next'}` }, off.length ? `NON-STANDARD BUILD: ${off.join(', ').toUpperCase()}` : 'STANDARD BUILD')))),
    h('div', { class: 'ps-banner' }, h('span', { class: 'ps-detail' }, STATE_TEXT[name] ?? '')),
    off.length ? problem('This milestone pins code or a key that is not in the published reproducible builds.', 'Check what it runs before relying on it.', 'NON-STANDARD BUILD') : null,
    amount(paid, 'xl'), h('p', { class: 'ps-caption' }, 'PAID ON THIS MILESTONE'),
    h('div', { class: 'ps-facts' },
      fact('CERTIFIED', `${m.certified_pct}%`),
      fact('BUDGET', `${usdc(total)} USDC`),
      fact('DEADLINE', new Date(Number(BigInt(m.deadline_ns) / 1_000_000n)).toISOString().slice(0, 16).replace('T', ' ') + ' UTC'),
      fact('REVIEWS', `${m.paid_attempts_used} of ${m.max_paid_attempts}`),
      fact('REVIEW FEE LEFT', `${usdc(m.fee_remaining)} USDC`)),
    cells(m, name, fields),
    sectionRule('Your actions'),
    h('div', { class: 'ps-actions' },
      ['Funding', 'Funded', 'HardFactsPassed', 'Approved'].includes(name) ? h('a', { class: 'ps-button', href: `claim.html?id=${mid}` }, '[ Contributor: bind and claim ]') : null,
      h('span', { class: 'ps-caption', id: 'funder-hint' }, `Funder actions appear when ${short(m.funder, 24)} is connected.`)),
    actions,
    sectionRule('Milestone'),
    h('div', { class: 'ps-block' }, m.milestone_text),
    h('dl', { class: 'ps-sheet' },
      h('dt', {}, 'Repository'), h('dd', {}, `${m.repo_owner}/${m.repo_name} (GitHub id ${m.repo_id})`),
      h('dt', {}, 'Merge into'), h('dd', {}, `${m.base_branch}: work merged anywhere else earns nothing`),
      h('dt', {}, 'Contributor'), contributor,
      h('dt', {}, 'Paid to'), h('dd', {}, m.payee ? m.payee.account : 'not bound yet: the contributor binds an account with a gist'),
      h('dt', {}, 'Required checks'), h('dd', {}, m.required_checks.join(', ')),
      h('dt', {}, 'Self-merge'), h('dd', {}, m.allow_self_merge ? 'allowed' : 'needs another person to merge or approve the exact commit'),
      h('dt', {}, 'Second opinion'), h('dd', {}, `${usdc(VERIFIER_FEE)} USDC per review, from the fee budget`),
      h('dt', {}, 'Funder'), h('dd', {}, m.funder),
      h('dt', {}, 'Pinned code'), h('dd', { class: 'ps-mono-id' }, `checker ${short(m.pins.checker.wasm_hash, 12)} · binder ${short(m.pins.binder.wasm_hash, 12)} · judge ${short(m.pins.judge.wasm_hash, 12)} · verifier key ${short(m.pins.verifier.public_key, 12)}`)),
    sectionRule('Paid pull requests'), prs,
    sectionRule('Agent decisions'), ai,
    sectionRule('On-chain transactions'), txs,
    sectionRule('What the judge paid the verifier'), fee);

  wallet.onAccount((acct) => {
    const isFunder = acct && acct === m.funder;
    $('#funder-hint').hidden = !!isFunder;
    if (isFunder) funderActions(m, name, fields, actions); else actions.replaceChildren();
  });

  githubLogin(m.payee_github_id).then((login) => {
    if (login) contributor.replaceChildren(h('a', { href: `https://github.com/${login}`, target: '_blank', rel: 'noopener' }, login), ` (GitHub id ${m.payee_github_id})`);
  });

  try {
    const e = evidence(await indexP, m);
    prs.replaceChildren(e.prs);
    fill(ai, ...e.ai);
    fill(txs, ...e.txs);
    fill(fee, ...e.fee);
  } catch (err) {
    const msg = problem(`Couldn't read the transaction history: ${err.message}`, 'The milestone above is read live from the contract. Reload to try the history again.', 'HISTORY UNAVAILABLE');
    ai.replaceChildren(msg); txs.replaceChildren(); fee.replaceChildren();
  }
}

main().catch((e) => page.replaceChildren(problem(`Couldn't read the chain: ${e.message}`)));
