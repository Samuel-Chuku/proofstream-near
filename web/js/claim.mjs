// The contributor's flow, all through the relayer: bind a payout account with a public gist,
// claim with a merged PR, then ask for the review. Each step waits for the chain and says why
// if the contract refused.
import { CONTRACT } from './config.mjs';
import { getMilestone, getBindNonce, relay, txEvents, stateOf, loadIndex, readMilestones } from './chain.mjs';
import { $, h, shell, usdc, copyChip, problem, until, txLink, stateChip } from './ui.mjs';
import * as wallet from './wallet.mjs';

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

/// The last transaction each step sent from this page, so its link stays after it confirms.
const sent = {};

/// A step's transaction, from what this page sent or else from the chain index. The shared
/// index can trail the chain by a rebuild, so it is asked again until the transaction shows up.
const STEP_EVENTS = {
  bind_payee: ['payee_bound'],
  claim: ['hard_facts_passed'],
  claim_started: ['claim_started'],
  review: ['milestone_paid', 'payout_started', 'certified'],
  review_started: ['review_started'],
};
const TX_LOOKUPS = 12;
const TX_LOOKUP_MS = 10000;
let indexP = null;
async function attachTx(el, method) {
  const own = sent[method === 'claim_started' ? 'claim' : method === 'review_started' ? 'review' : method];
  if (own) { el.append(' ', txLink(own)); return; } // the step's own transaction, sent from this page
  for (let i = 0; i < TX_LOOKUPS; i++) {
    if (i > 0 && !el.isConnected) return; // redrawn since: the new line looks for itself
    try {
      indexP = i === 0 && indexP ? indexP : loadIndex({ fresh: i > 0 });
      const index = await indexP;
      const t = index.txs.find((x) => x.events.some((e) => e.milestone_id === Number(id) && STEP_EVENTS[method].includes(e.event)));
      if (t) { el.append(' ', txLink(t.hash)); return; }
    } catch { /* no index this time */ }
    await new Promise((r) => setTimeout(r, TX_LOOKUP_MS));
  }
}
function doneLine(method, text) {
  const line = h('p', { class: 'ps-detail ps-done-line' }, text);
  attachTx(line, method);
  return line;
}
/// A step still running: the green working strip, its text, and its transaction once known.
function working(text, method) {
  const line = checking(text);
  if (method) attachTx(line, method);
  return line;
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
  sent[method] = tx;
  box.replaceChildren(checking(waitingText), txLink(tx));
  const m = await until(async () => {
    const now = await getMilestone(Number(id));
    const [name] = stateOf(now);
    return busyStates.includes(name) || (method === 'bind_payee' && now.binding_in_flight) ? null : now;
  }, { everyMs: 3000, maxMs: 150000 });
  const events = await until(() => txEvents(tx).then((e) => (e.length ? e : null)), { everyMs: 3000, maxMs: 30000 }) || [];
  return { tx, m, events };
}

/// Who is signed in with GitHub on this site, if anyone: 'in' (with login and numeric id), 'out',
/// or 'off' when the site has no GitHub sign-in (the manual gist steps still work).
async function githubSession() {
  try {
    const r = await fetch('/api/github/me', { credentials: 'same-origin' });
    if (r.status === 200) {
      const me = await r.json();
      if (me && me.login && Number.isSafeInteger(me.id)) return { state: 'in', ...me };
    }
    if (r.status === 401) return { state: 'out' };
  } catch { /* no functions here (local server): manual steps only */ }
  return { state: 'off' };
}

async function bindStep(m, nonce) {
  const box = $('#bind');
  if (m.payee) {
    $('#step-bind').classList.add('ps-step-done');
    box.replaceChildren(doneLine('bind_payee', `Payouts go to ${m.payee.account}, set by gist ${m.payee.gist_id}.`));
    return;
  }
  const gh = await githubSession();
  const here = `${location.pathname}${location.search}`;
  const account = h('input', { class: 'ps-input', id: 'acct', placeholder: 'your-name.testnet', autocomplete: 'off', spellcheck: 'false' });
  const file = h('pre', { class: 'ps-code' });
  const gist = h('input', { class: 'ps-input', placeholder: 'https://gist.github.com/you/…', autocomplete: 'off' });
  const out = h('div');
  const content = () => JSON.stringify({ contract: CONTRACT, milestone_id: Number(id), nonce, account: account.value.trim() }, null, 2);
  const render = () => { file.textContent = content(); };
  account.addEventListener('input', render);
  render();
  const validAccount = () => {
    const acct = account.value.trim();
    if (NAMED.test(acct) || IMPLICIT.test(acct)) return acct;
    out.replaceChildren(problem('That isn\'t a NEAR account id.'));
    return null;
  };
  const bindWith = async (gid, buttons) => {
    buttons.forEach((b) => { b.disabled = true; });
    const r = await step(out, 'bind_payee', { milestone_id: Number(id), gist_id: gid }, [], 'The binder is reading your gist inside TDX…');
    buttons.forEach((b) => { b.disabled = false; });
    if (!r) return;
    const refused = r.events.find((e) => e.event === 'bind_refused');
    if (refused) { out.replaceChildren(problem(explain(refused.reason)), txLink(r.tx)); return; }
    load();
  };

  const send = h('button', { class: 'ps-button', type: 'button' }, 'Bind');
  const quick = h('button', { class: 'ps-button ps-button-primary', type: 'button' }, '[ Bind with GitHub ]');
  send.addEventListener('click', async () => {
    if (!validAccount()) return;
    const gid = (gist.value.match(/[0-9a-f]{20,64}/i) || [])[0]?.toLowerCase();
    if (!gid) { out.replaceChildren(problem('Paste the gist\'s URL or id.')); return; }
    bindWith(gid, [send, quick]);
  });
  quick.addEventListener('click', async () => {
    const acct = validAccount();
    if (!acct) return;
    quick.disabled = true;
    out.replaceChildren(checking('Posting your bind gist on GitHub…'));
    let res;
    try {
      res = await fetch('/api/github/gist', { method: 'POST', credentials: 'same-origin', headers: { 'Content-Type': 'application/json' }, body: JSON.stringify({ milestone_id: Number(id), nonce, account: acct }) });
    } catch { res = null; }
    quick.disabled = false;
    const body = res ? await res.json().catch(() => ({})) : {};
    if (!res || !res.ok || !body.id) {
      const again = body.error === 'gist_permission' || body.error === 'not_signed_in';
      out.replaceChildren(problem(again ? 'GitHub needs you to sign in again so this site may post the gist.' : 'GitHub didn\'t create the gist.',
        again ? h('a', { href: `/api/github/login?next=${encodeURIComponent(here)}` }, 'Sign in with GitHub again') : 'Post it yourself with the steps below.', 'NOT YET'));
      return;
    }
    out.replaceChildren(h('p', { class: 'ps-caption' }, 'GIST POSTED · ', h('a', { href: body.url, target: '_blank', rel: 'noopener' }, 'view it ↗')));
    bindWith(body.id, [send, quick]);
  });

  let fast = null;
  if (gh.state === 'in' && gh.id === m.payee_github_id) {
    fast = [h('p', { class: 'ps-detail' }, `2. Signed in as ${gh.login}. One click posts the bind gist as you and binds the account:`), h('div', { class: 'ps-actions' }, quick)];
  } else if (gh.state === 'in') {
    fast = [h('p', { class: 'ps-detail' }, `2. You're signed in as ${gh.login}, but this milestone pays GitHub id ${m.payee_github_id}. `,
      h('a', { href: `/api/github/logout?next=${encodeURIComponent(here)}` }, 'Sign out'), ' and sign in as that account.')];
  } else if (gh.state === 'out') {
    fast = [h('p', { class: 'ps-detail' }, '2. Sign in as the milestone\'s GitHub account, and the site posts the bind gist for you:'),
      h('div', { class: 'ps-actions' }, h('a', { class: 'ps-button ps-button-primary', href: `/api/github/login?next=${encodeURIComponent(here)}` }, '[ Sign in with GitHub ]'))];
  }
  const manual = [
    h('p', { class: 'ps-detail' }, 'Create a ', h('strong', {}, 'public'), ' gist on GitHub, signed in as the milestone\'s contributor, with one file named ', h('code', {}, 'proofstream-bind.json'), ' holding exactly:'),
    file, h('div', { class: 'ps-actions' }, copyChip(content, 'COPY FILE'), h('a', { class: 'ps-chip ps-chip-link', href: 'https://gist.github.com/', target: '_blank', rel: 'noopener' }, 'NEW GIST ↗')),
    h('p', { class: 'ps-detail' }, 'Paste the gist\'s URL:'), h('div', { class: 'ps-field' }, gist),
    h('div', { class: 'ps-actions' }, send)];
  box.replaceChildren(
    h('p', { class: 'ps-detail' }, '1. The NEAR account to be paid (it must exist; a 64-hex implicit account is created for you):'),
    h('div', { class: 'ps-field' }, account),
    ...(fast ?? []),
    fast
      ? h('details', { class: 'ps-section' }, h('summary', {}, h('span', { class: 'ps-caption' }, 'OR POST THE GIST YOURSELF')), ...manual)
      : h('div', {}, h('p', { class: 'ps-detail' }, '2. Post the gist yourself:'), ...manual),
    out);
}

/// The manual way, folded away: the relayer claims and asks for the review by itself, so these
/// are only a fallback (a PR that names no milestone when several fit, or auto-claim being off).
function byHand(label, ...body) {
  return h('details', { class: 'ps-section' }, h('summary', {}, h('span', { class: 'ps-caption' }, label)), ...body);
}

function claimStep(m, name, fields) {
  const box = $('#claim');
  if (!m.payee) { box.replaceChildren(h('p', { class: 'ps-caption' }, 'after binding')); return; }
  if (['HardFactsPassed', 'Reviewing', 'Held', 'PayingOut', 'Paid', 'PayoutFailed'].includes(name)) {
    $('#step-claim').classList.add('ps-step-done');
    box.replaceChildren(doneLine('claim', fields.pr ? `PR #${fields.pr} passed the hard facts: merged, right author, checks passed.` : 'Done.'));
    return;
  }
  if (name === 'Checking') { box.replaceChildren(working(`The checker is reading PR #${fields.pr} inside TDX…`, 'claim_started')); return; }
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
  const rules = h('p', { class: 'ps-detail' }, name === 'Approved'
    ? `Both agents certified PR #${fields.pr} before it was merged. Merge it at commit ${fields.head_sha.slice(0, 8)}, then claim it again to be paid.`
    : `The PR must be merged into ${m.base_branch} of ${m.repo_owner}/${m.repo_name}, by someone else or with an independent approval${m.allow_self_merge ? ' (self-merge is allowed here)' : ''}, after this milestone was created, with ${m.required_checks.join(', ')} passing.`);
  const manual = [h('div', { class: 'ps-actions' }, h('span', { class: 'ps-caption' }, 'PR #'), pr, send), out];
  if (name === 'Approved') { box.replaceChildren(rules, ...manual); return; }
  box.replaceChildren(
    working('Waiting for a merged PR: the relayer claims it within about 15 s of the merge.'),
    rules,
    h('p', { class: 'ps-caption' }, `With more than one open milestone in this repo, write "ProofStream #${id}" in the PR description.`),
    byHand('CLAIM BY HAND', ...manual));
}

function reviewStep(m, name, fields) {
  const box = $('#review');
  if (name === 'Paid' || name === 'PayingOut') {
    $('#step-review').classList.add('ps-step-done');
    box.replaceChildren(doneLine('review', `Certified ${m.certified_pct}%. ${usdc(BigInt(m.budget_total) - BigInt(m.budget))} USDC paid to ${m.payee.account}.`));
    return;
  }
  if (name === 'Held') { box.replaceChildren(h('p', { class: 'ps-detail' }, `Held for the funder: ${fields.reason}.`)); return; }
  if (name === 'Reviewing') { box.replaceChildren(working('The judge reviews, then buys the verifier\'s signed opinion (20–90 s)…', 'review_started')); return; }
  if (name !== 'HardFactsPassed') {
    box.replaceChildren(h('p', { class: 'ps-caption' }, m.certified_pct > 0 ? `${m.certified_pct}% certified so far; the next merged PR is claimed for the rest` : 'after the claim'));
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
  box.replaceChildren(
    working(m.paid_attempts_used > 0 ? 'The last review lost its second opinion; the relayer asks again…' : 'The relayer asks the two agents for their review…'),
    h('p', { class: 'ps-detail' }, 'The lower of the two shares is paid. An objection, or shares more than 30 points apart, holds the money for the funder.'),
    byHand('ASK BY HAND', h('div', { class: 'ps-actions' }, send), out));
}

/// While the relayer might act on its own (auto-claim, then the review), re-read the milestone
/// every few seconds and redraw when its state changes.
const WATCH_MS = 5000;
let watching = null;
function watchState(m, name) {
  clearInterval(watching);
  if (!m.payee || !['Funded', 'Checking', 'HardFactsPassed', 'Reviewing', 'PayingOut'].includes(name)) return;
  watching = setInterval(async () => {
    const now = await getMilestone(Number(id)).catch(() => null);
    if (now && stateOf(now)[0] !== name) { clearInterval(watching); load(); }
  }, WATCH_MS);
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
  await bindStep(m, await getBindNonce(Number(id)));
  claimStep(m, name, fields);
  reviewStep(m, name, fields);
  watchState(m, name);
}

/// Every milestone paying the connected wallet, or the signed-in GitHub account, each linking to its page.
async function myClaims() {
  const box = $('#mine-list');
  const gh = await githubSession();
  const show = async (acct) => {
    const who = [acct, gh.state === 'in' ? `GitHub ${gh.login}` : null].filter(Boolean);
    if (!who.length) { box.replaceChildren(h('p', { class: 'ps-caption' }, 'Connect a wallet or sign in with GitHub to list your claims.')); return; }
    box.replaceChildren(h('p', { class: 'ps-caption' }, `READING THE CONTRACT FOR ${who.join(' · ')}…`));
    let all;
    // The shared index picks the candidates in one request; each is then read live, since the
    // index can trail the chain (a payout or a bind a minute old). Milestones newer than the
    // index, and unbound ones that may have just been bound, are candidates too.
    try {
      indexP ??= loadIndex();
      const listed = (await indexP).milestones;
      const ids = listed.filter((m) => !m.payee || m.payee.account === acct || (gh.state === 'in' && m.payee_github_id === gh.id)).map((m) => m.id);
      for (let next = listed.length; next < listed.length + 20; next++) {
        if (!(await getMilestone(next))) break;
        ids.push(next);
      }
      all = (await Promise.all(ids.map(async (i) => ({ id: i, ...(await getMilestone(i)) })))).filter((m) => m.repo_owner);
    } catch {
      try { all = await readMilestones(); } catch (e) { box.replaceChildren(problem(`Couldn't read the contract: ${e.message}`)); return; }
    }
    const mine = all.filter((m) => (acct && m.payee?.account === acct) || (gh.state === 'in' && m.payee_github_id === gh.id)).reverse();
    if (!mine.length) { box.replaceChildren(h('p', { class: 'ps-caption' }, `No milestones pay ${who.join(' or ')} yet.`)); return; }
    const row = (m) => {
      const [name] = stateOf(m);
      const go = () => { location.href = `milestone.html?id=${m.id}`; };
      return h('tr', { 'data-href': `milestone.html?id=${m.id}`, onclick: go },
        h('td', {}, h('a', { href: `milestone.html?id=${m.id}` }, `#${m.id}`)),
        h('td', { class: 'ps-hide-sm' }, `${m.repo_owner}/${m.repo_name}`),
        h('td', {}, stateChip(name, m)),
        h('td', {}, `${usdc(BigInt(m.budget_total) - BigInt(m.budget))} of ${usdc(m.budget_total)} USDC`),
        h('td', {}, m.payee.account));
    };
    box.replaceChildren(h('table', { class: 'ps-table' },
      h('thead', {}, h('tr', {}, h('th', {}, '#'), h('th', { class: 'ps-hide-sm' }, 'Repository'), h('th', {}, 'State'), h('th', {}, 'Paid'), h('th', {}, 'Pays'))),
      h('tbody', {}, ...mine.map(row))));
  };
  let last;
  wallet.onAccount((acct) => { if (acct !== last) { last = acct; show(acct); } });
}
myClaims();

if (id !== null && /^\d+$/.test(id)) {
  $('#flow').hidden = false;
  load().catch((e) => $('main').append(problem(`Couldn't read the contract: ${e.message}`)));
} else {
  $('#pick').hidden = false;
  $('#pick').addEventListener('submit', (ev) => { ev.preventDefault(); location.search = `?id=${$('#pick-id').value}`; });
}
