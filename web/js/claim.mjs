// The contributor's flow, all through the relayer: bind a payout account with a public gist,
// claim with a merged PR, then ask for the review. Each step waits for the chain and says why
// if the contract refused.
import { CONTRACT } from './config.mjs';
import { getMilestone, getBindNonce, relay, txEvents, stateOf } from './chain.mjs';
import { $, h, shell, usdc, copyChip, problem, until, txLink } from './ui.mjs';

shell('claim');
const id = new URLSearchParams(location.search).get('id');

const REASONS = {
  NotTheGithubIdHolder: "The gist isn't owned by the milestone's GitHub account.",
  WrongNonce: 'The gist has an old or wrong nonce. Copy the file again.',
  WrongMilestone: 'The gist names another milestone.',
  WrongContract: 'The gist names another contract.',
  AccountMissing: "That NEAR account doesn't exist.",
  UnsupportedAccount: '0x and 0s accounts are not supported yet.',
  WrongAuthor: "The PR wasn't opened by the milestone's GitHub account.",
  NotMerged: "The PR isn't merged yet.",
  MergedBeforeMilestone: 'The PR was merged before this milestone was created.',
  ChecksNotSuccessful: "A required check didn't pass on the PR's head commit.",
  TouchesGithub: 'The PR changes files under .github/.',
  NoIndependentApproval: 'Someone other than the author must merge it or approve this exact commit.',
  IncompleteFileList: "GitHub didn't return the PR's full file list.",
  WrongBaseBranch: 'The PR targets another branch.',
  WrongRepo: 'The PR is in another repository.',
};
const explain = (reason) => REASONS[reason] ?? reason;

const NAMED = /^(?=.{2,64}$)(([a-z\d]+[-_])*[a-z\d]+\.)*([a-z\d]+[-_])*[a-z\d]+$/;
const IMPLICIT = /^[0-9a-f]{64}$/;

function checking(text) {
  return h('p', { class: 'ps-checking ps-detail' }, h('span', { class: 'ps-checking-cells', 'aria-hidden': 'true' }, h('span'), h('span'), h('span'), h('span')), text);
}

/// Sends through the relayer, then waits until the milestone leaves `busy` states.
async function step(box, method, body, busyStates, waitingText) {
  box.replaceChildren(checking('Sending through the relayer…'));
  let tx;
  try {
    tx = await relay(method, body);
  } catch (e) {
    box.replaceChildren(problem(explain(e.reason || e.message), e.tx ? `transaction ${e.tx}` : null));
    return null;
  }
  box.replaceChildren(checking(waitingText), txLink(tx));
  const m = await until(async () => {
    const now = await getMilestone(Number(id));
    const [name] = stateOf(now);
    return busyStates.includes(name) || (method === 'bind_payee' && now.binding_in_flight) ? null : now;
  }, { everyMs: 3000, maxMs: 150000 });
  const events = await until(() => txEvents(tx).then((e) => (e.length ? e : null)), { everyMs: 3000, maxMs: 30000 }) || [];
  return { tx, m, events };
}

function bindStep(m, nonce) {
  const box = $('#bind');
  if (m.payee) {
    $('#step-bind').classList.add('ps-step-done');
    box.replaceChildren(h('p', { class: 'ps-detail' }, `Payouts go to ${m.payee.account}, set by gist ${m.payee.gist_id}.`));
    return;
  }
  const account = h('input', { class: 'ps-input', id: 'acct', placeholder: 'your-name.testnet', autocomplete: 'off', spellcheck: 'false' });
  const file = h('pre', { class: 'ps-code' });
  const gist = h('input', { class: 'ps-input', placeholder: 'https://gist.github.com/you/…', autocomplete: 'off' });
  const out = h('div');
  const content = () => JSON.stringify({ contract: CONTRACT, milestone_id: Number(id), nonce, account: account.value.trim() }, null, 2);
  const render = () => { file.textContent = content(); };
  account.addEventListener('input', render);
  render();
  const send = h('button', { class: 'ps-button', type: 'button' }, 'Bind');
  send.addEventListener('click', async () => {
    const acct = account.value.trim();
    if (!NAMED.test(acct) && !IMPLICIT.test(acct)) { out.replaceChildren(problem('That isn\'t a NEAR account id.')); return; }
    const gid = (gist.value.match(/[0-9a-f]{20,64}/i) || [])[0]?.toLowerCase();
    if (!gid) { out.replaceChildren(problem('Paste the gist\'s URL or id.')); return; }
    send.disabled = true;
    const r = await step(out, 'bind_payee', { milestone_id: Number(id), gist_id: gid }, [], 'The binder is reading your gist inside TDX…');
    send.disabled = false;
    if (!r) return;
    const refused = r.events.find((e) => e.event === 'bind_refused');
    if (refused) { out.replaceChildren(problem(explain(refused.reason)), txLink(r.tx)); return; }
    load();
  });
  box.replaceChildren(
    h('p', { class: 'ps-detail' }, '1. The NEAR account to be paid (it must exist; a 64-hex implicit account is created for you):'),
    h('div', { class: 'ps-field' }, account),
    h('p', { class: 'ps-detail' }, '2. Create a ', h('strong', {}, 'public'), ' gist on GitHub, signed in as the milestone\'s contributor, with one file named ', h('code', {}, 'proofstream-bind.json'), ' holding exactly:'),
    file, h('div', { class: 'ps-actions' }, copyChip(content, 'COPY FILE'), h('a', { class: 'ps-chip ps-chip-link', href: 'https://gist.github.com/', target: '_blank', rel: 'noopener' }, 'NEW GIST ↗')),
    h('p', { class: 'ps-detail' }, '3. Paste the gist\'s URL:'), h('div', { class: 'ps-field' }, gist),
    h('div', { class: 'ps-actions' }, send), out);
}

function claimStep(m, name, fields) {
  const box = $('#claim');
  if (!m.payee) { box.replaceChildren(h('p', { class: 'ps-caption' }, 'after binding')); return; }
  if (['HardFactsPassed', 'Reviewing', 'Held', 'PayingOut', 'Paid', 'PayoutFailed'].includes(name)) {
    $('#step-claim').classList.add('ps-step-done');
    box.replaceChildren(h('p', { class: 'ps-detail' }, fields.pr ? `PR #${fields.pr} passed the hard facts.` : 'Done.'));
    return;
  }
  if (name === 'Checking') { box.replaceChildren(checking(`The checker is reading PR #${fields.pr}…`)); return; }
  if (name === 'Refunded' || name === 'Funding') { box.replaceChildren(h('p', { class: 'ps-detail' }, name === 'Funding' ? 'Waiting for the funder to deposit.' : 'This milestone was refunded.')); return; }
  const pr = h('input', { class: 'ps-input ps-input-short', inputmode: 'numeric', value: fields.pr ?? '', 'aria-label': 'Pull request number' });
  const out = h('div');
  const send = h('button', { class: 'ps-button', type: 'button' }, 'Claim');
  send.addEventListener('click', async () => {
    if (!/^\d+$/.test(pr.value)) { out.replaceChildren(problem('Enter the PR number.')); return; }
    send.disabled = true;
    const r = await step(out, 'claim', { milestone_id: Number(id), pr_number: Number(pr.value) }, ['Checking'], 'The checker is reading GitHub inside TDX (10–60 s)…');
    send.disabled = false;
    if (!r) return;
    const refused = r.events.find((e) => e.event === 'claim_refused');
    if (refused) { out.replaceChildren(problem(explain(refused.reason)), txLink(r.tx)); return; }
    load();
  });
  box.replaceChildren(
    h('p', { class: 'ps-detail' }, name === 'Approved'
      ? `Both AIs certified PR #${fields.pr} before it was merged. Merge it at commit ${fields.head_sha.slice(0, 8)}, then claim it again to be paid.`
      : `The PR must be merged into ${m.base_branch} of ${m.repo_owner}/${m.repo_name}, by someone else or with an independent approval${m.allow_self_merge ? ' (self-merge is allowed here)' : ''}, after this milestone was created, with ${m.required_checks.join(', ')} passing.`),
    h('div', { class: 'ps-actions' }, h('span', { class: 'ps-caption' }, 'PR #'), pr, send), out);
}

function reviewStep(m, name, fields) {
  const box = $('#review');
  if (name === 'Paid' || name === 'PayingOut') {
    $('#step-review').classList.add('ps-step-done');
    box.replaceChildren(h('p', { class: 'ps-detail' }, `Certified ${m.certified_pct}%. ${usdc(BigInt(m.budget_total) - BigInt(m.budget))} USDC paid to ${m.payee.account}.`));
    return;
  }
  if (name === 'Held') { box.replaceChildren(h('p', { class: 'ps-detail' }, `Held for the funder: ${fields.reason}.`)); return; }
  if (name === 'Reviewing') { box.replaceChildren(checking('The judge and the verifier are reviewing…')); return; }
  if (name !== 'HardFactsPassed') {
    box.replaceChildren(h('p', { class: 'ps-caption' }, m.certified_pct > 0 ? `${m.certified_pct}% certified so far; claim your next PR for the rest` : 'after the claim'));
    return;
  }
  const out = h('div');
  const send = h('button', { class: 'ps-button', type: 'button' }, 'Ask for the review');
  send.addEventListener('click', async () => {
    send.disabled = true;
    const r = await step(out, 'review', { milestone_id: Number(id) }, ['Reviewing'], 'The judge reviews, then buys the verifier\'s signed opinion (20–90 s)…');
    send.disabled = false;
    if (!r) return;
    const result = r.events.find((e) => e.event === 'review_result');
    const paid = r.events.find((e) => e.event === 'milestone_paid' || e.event === 'payout_started');
    const held = r.events.find((e) => e.event === 'held');
    const retry = r.events.find((e) => e.event === 'review_retry');
    out.replaceChildren(...[
      result ? h('p', { class: 'ps-detail' }, `Judge: ${result.judge}. Verifier: ${result.verifier}.`) : null,
      paid ? h('p', { class: 'ps-body' }, `${paid.pct}% certified: ${usdc(paid.amount)} USDC to ${paid.to}.`) : null,
      held ? h('p', { class: 'ps-body' }, `Held for the funder: ${held.reason}.`) : null,
      retry ? h('p', { class: 'ps-body' }, `No verdict this time; ${retry.attempts_left} attempts left. Ask again.`) : null,
      txLink(r.tx)].filter(Boolean));
    if (paid) load();
  });
  box.replaceChildren(h('p', { class: 'ps-detail' }, 'The lower of the two shares is paid. An objection, or shares more than 30 points apart, holds the money for the funder.'), h('div', { class: 'ps-actions' }, send), out);
}

async function load() {
  const m = await getMilestone(Number(id));
  if (!m) { $('#flow').replaceChildren(problem(`There is no milestone #${id} on ${CONTRACT}.`)); return; }
  const [name, fields] = stateOf(m);
  $('#crumb').textContent = `CONTRIBUTOR · MILESTONE #${id}`;
  $('#view-link').href = `milestone.html?id=${id}`;
  $('#summary').replaceChildren(
    h('dt', {}, 'Repository'), h('dd', {}, `${m.repo_owner}/${m.repo_name} · ${m.base_branch}`),
    h('dt', {}, 'Milestone'), h('dd', { class: 'ps-detail' }, m.milestone_text),
    h('dt', {}, 'Budget'), h('dd', {}, `${usdc(m.budget_total)} USDC · ${m.certified_pct}% certified`),
    h('dt', {}, 'State'), h('dd', {}, name));
  bindStep(m, await getBindNonce(Number(id)));
  claimStep(m, name, fields);
  reviewStep(m, name, fields);
}

if (id !== null && /^\d+$/.test(id)) {
  $('#flow').hidden = false;
  load().catch((e) => $('main').append(problem(`Couldn't read the contract: ${e.message}`)));
} else {
  $('#pick').hidden = false;
  $('#pick').addEventListener('submit', (ev) => { ev.preventDefault(); location.search = `?id=${$('#pick-id').value}`; });
}
