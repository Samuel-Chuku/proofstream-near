// The funder's flow, in five sections that tick themselves off: wallet, repository (picked
// through GitHub, or typed), contributor, terms, then a list of exactly what is still missing
// and one wallet prompt that creates and funds the milestone.
import { CONTRACT, TOKEN, PROJECTS, VERIFIER_PUBLIC_KEY, VERIFIER_FEE, CREATE_DEPOSIT, TGAS, NEAR_FAUCET, USDC_MINT_UNITS, TOKEN_STORAGE_DEPOSIT } from './config.mjs';
import { nextMilestoneId, tokenBalance, nearBalance, getConfig } from './chain.mjs';
import { $, h, shell, amount, usdc, toUnits, problem } from './ui.mjs';
import * as wallet from './wallet.mjs';

shell('new');
const form = $('#form');
const f = (name) => form.elements[name];
const GAS_100 = (100n * TGAS).toString();
const GAS_30 = (30n * TGAS).toString();
const MIN_TEXT = 80;
const state = { account: null, near: null, usdc: 0n, gh: null, repos: null, repo: null, branches: [], branch: '', contributor: null, builds: null, minDeadlineS: 600 };

async function api(path, init) {
  const r = await fetch(path, { credentials: 'same-origin', ...init });
  const body = await r.json().catch(() => ({}));
  if (!r.ok) throw Object.assign(new Error(body.error || `HTTP ${r.status}`), { status: r.status, body });
  return body;
}

async function publicGithub(path) {
  const r = await fetch(`https://api.github.com/${path}`, { signal: AbortSignal.timeout(8000) });
  if (r.status === 404) return null;
  if (!r.ok) throw new Error(`GitHub answered ${r.status}`);
  return r.json();
}

// ---------------------------------------------------------------- 1. wallet

async function renderWallet() {
  const box = $('#wallet-body');
  if (!state.account) {
    box.replaceChildren(h('p', { class: 'ps-detail' }, 'This account becomes the funder: only it can release, reject or refund.'),
      h('div', { class: 'ps-actions' }, h('button', { class: 'ps-button', type: 'button', onclick: () => wallet.connect() }, '[ Connect wallet ]')));
    return;
  }
  [state.near, state.usdc] = await Promise.all([nearBalance(state.account), tokenBalance(state.account).then((v) => BigInt(v || '0')).catch(() => 0n)]);
  const out = h('div');
  const mint = async () => {
    out.replaceChildren(h('p', { class: 'ps-detail' }, 'Confirm in your wallet: register with the token, then mint 10 test USDC.'));
    try {
      await wallet.send([{ receiverId: TOKEN.id, calls: [
        { method: 'storage_deposit', args: { account_id: state.account, registration_only: true }, gas: GAS_30, deposit: TOKEN_STORAGE_DEPOSIT },
        { method: 'mint', args: { account_id: state.account, amount: USDC_MINT_UNITS }, gas: GAS_30, deposit: '0' }] }]);
      await renderWallet(); refresh();
    } catch (e) { out.replaceChildren(problem(e.message || String(e))); }
  };
  box.replaceChildren(
    h('dl', { class: 'ps-sheet' }, h('dt', {}, 'Funder'), h('dd', {}, state.account),
      h('dt', {}, 'NEAR'), h('dd', {}, state.near === null ? '?' : `${(Number(state.near / 10n ** 21n) / 1000).toFixed(3)} NEAR`),
      h('dt', {}, 'USDC'), h('dd', {}, `${usdc(state.usdc)} USDC`)),
    h('div', { class: 'ps-actions' },
      h('a', { class: 'ps-button', href: NEAR_FAUCET, target: '_blank', rel: 'noopener' }, '[ Test NEAR ↗ ]'),
      h('button', { class: 'ps-button', type: 'button', onclick: mint }, '[ Get 10 test USDC ]'),
      h('button', { class: 'ps-button', type: 'button', onclick: () => wallet.disconnect() }, '[ Disconnect ]')), out);
}

// ---------------------------------------------------------------- 2. repository

async function loadGithub() {
  try { state.gh = await api('/api/github/me'); } catch { state.gh = null; }
  if (!state.gh) return;
  try { state.repos = await api('/api/github/repos'); } catch (e) { state.repos = { repos: [], installations: [], error: e.message }; }
}

async function chooseRepo(fullName) {
  state.repo = null; state.branches = []; state.branch = '';
  if (!fullName) return renderRepo();
  try {
    const r = await publicGithub(`repos/${fullName}`);
    if (!r) { state.repoError = 'Not found.'; return renderRepo(); }
    if (r.private) { state.repoError = 'Private repositories are not supported: the checker reads public repositories only.'; return renderRepo(); }
    state.repoError = null;
    state.repo = { id: r.id, owner: r.owner.login, name: r.name, fullName: r.full_name, defaultBranch: r.default_branch };
    state.branch = r.default_branch;
    state.branches = state.gh ? (await api(`/api/github/branches?repo=${encodeURIComponent(r.full_name)}`).catch(() => ({ branches: [] }))).branches : [];
  } catch (e) { state.repoError = e.message; }
  renderRepo(); refresh();
}

function renderRepo() {
  const box = $('#repo-body');
  const parts = [];
  if (state.gh) {
    const repos = (state.repos?.repos || []).filter((r) => !r.private);
    const select = h('select', { class: 'ps-input', onchange: (ev) => chooseRepo(ev.target.value) },
      h('option', { value: '' }, repos.length ? 'Choose a repository' : 'No public repositories granted yet'),
      ...repos.map((r) => h('option', { value: r.fullName, selected: state.repo?.fullName === r.fullName }, r.fullName)));
    const installs = state.repos?.installations || [];
    parts.push(h('p', { class: 'ps-detail' }, `Signed in to GitHub as ${state.gh.login}. Only repositories you granted the ProofStream app appear here.`),
      h('label', { class: 'ps-field' }, h('span', { class: 'ps-caption' }, 'Repository'), select),
      h('div', { class: 'ps-actions' },
        h('a', { class: 'ps-chip ps-chip-link', href: installs[0]?.settingsUrl || '/api/github/login?install=1&next=/new.html' }, 'A REPOSITORY IS MISSING? GRANT IT ↗'),
        h('a', { class: 'ps-chip ps-chip-link', href: '/api/github/logout?next=/new.html' }, 'SIGN OUT OF GITHUB')));
  } else {
    const typed = h('input', { class: 'ps-input', placeholder: 'owner/name', autocomplete: 'off', spellcheck: 'false', value: state.repo?.fullName || '' });
    typed.addEventListener('change', () => chooseRepo(typed.value.trim().replace(/^https:\/\/github\.com\//, '').replace(/\/$/, '')));
    parts.push(h('div', { class: 'ps-actions' }, h('a', { class: 'ps-button', href: '/api/github/login?next=/new.html' }, '[ Connect GitHub ]'), h('span', { class: 'ps-caption' }, 'to pick a repository and create a branch, or type it below')),
      h('label', { class: 'ps-field' }, h('span', { class: 'ps-caption' }, 'Repository (public, owner/name)'), typed));
  }
  if (state.repoError) parts.push(h('p', { class: 'ps-label' }, state.repoError));
  if (state.repo) {
    parts.push(h('p', { class: 'ps-caption' }, `GITHUB ID ${state.repo.id} · THE CONTRACT PINS THE ID, NOT THE NAME`));
    let branchInput;
    if (state.branches.length) {
      branchInput = h('select', { class: 'ps-input', onchange: (ev) => { state.branch = ev.target.value; refresh(); } }, ...state.branches.map((b) => h('option', { value: b, selected: b === state.branch }, b)));
    } else {
      branchInput = h('input', { class: 'ps-input', value: state.branch, autocomplete: 'off' });
      branchInput.addEventListener('input', () => { state.branch = branchInput.value.trim(); refresh(); });
    }
    parts.push(h('label', { class: 'ps-field' }, h('span', { class: 'ps-caption' }, 'Merge into (base branch)'), branchInput,
      h('span', { class: 'ps-caption' }, 'Only pull requests merged into this branch count.')));
    if (state.gh) {
      const name = h('input', { class: 'ps-input', placeholder: 'milestone/1', autocomplete: 'off' });
      const out = h('div');
      parts.push(h('details', { class: 'ps-section' }, h('summary', {}, h('span', { class: 'ps-caption' }, 'Create a new branch for this milestone')),
        h('div', { class: 'ps-section-body' }, h('div', { class: 'ps-actions' }, name,
          h('button', { class: 'ps-button', type: 'button', onclick: async () => {
            out.replaceChildren(h('p', { class: 'ps-detail' }, 'Creating…'));
            try {
              await api('/api/github/branch', { method: 'POST', headers: { 'Content-Type': 'application/json' }, body: JSON.stringify({ repo: state.repo.fullName, branch: name.value.trim(), from: state.repo.defaultBranch }) });
              state.branches = [...state.branches, name.value.trim()].sort(); state.branch = name.value.trim(); renderRepo(); refresh();
            } catch (e) { out.replaceChildren(problem(e.status === 403 ? 'The ProofStream app has no write access to this repository. Grant contents: write, or create the branch on GitHub.' : e.message)); }
          } }, '[ Create branch ]')), h('p', { class: 'ps-caption' }, `FROM ${state.repo.defaultBranch.toUpperCase()}`), out)));
    }
  }
  box.replaceChildren(...parts);
}

// ---------------------------------------------------------------- 3. contributor

async function lookContributor() {
  const v = f('contributor').value.trim().replace(/^@/, '');
  state.contributor = null;
  if (!/^[A-Za-z\d-]{1,39}$/.test(v)) { refresh(); return; }
  try {
    const u = await publicGithub(`users/${v}`);
    state.contributor = u ? { id: u.id, login: u.login } : null;
    $('#contributor-note').textContent = u ? `GitHub id ${u.id}. Only this account's gist can bind the payout account, and only its pull requests count.` : 'No such GitHub user.';
  } catch (e) { $('#contributor-note').textContent = e.message; }
  refresh();
}

// ---------------------------------------------------------------- 4. terms and 5. create

function amounts() {
  const budget = toUnits(f('budget').value);
  const attempts = Number(f('attempts').value);
  const fee = Number.isInteger(attempts) && attempts > 0 ? BigInt(VERIFIER_FEE) * BigInt(attempts) : null;
  return { budget, attempts, fee, total: budget !== null && fee !== null ? budget + fee : null };
}

function blockers() {
  const out = { wallet: [], repo: [], who: [], terms: [] };
  const { budget, attempts, total } = amounts();
  if (!state.account) out.wallet.push('Connect a wallet: it becomes the funder.');
  else if (total !== null && state.usdc < total) out.wallet.push(`Your wallet holds ${usdc(state.usdc)} USDC; this needs ${usdc(total)}. Use "Get 10 test USDC".`);
  if (!state.repo) out.repo.push('Choose a public repository.');
  if (!state.branch) out.repo.push('Name the branch work must merge into.');
  if (!state.contributor) out.who.push("Name the contributor's GitHub account.");
  if (budget === null || budget < 1_000_000n) out.terms.push('The budget must be at least 1 USDC.');
  if (!Number.isInteger(attempts) || attempts < 1 || attempts > 25) out.terms.push('Reviews allowed must be 1 to 25.');
  const checks = f('checks').value.split(',').map((c) => c.trim()).filter(Boolean);
  if (checks.length < 1 || checks.length > 8) out.terms.push('Name 1 to 8 required checks.');
  const text = f('text').value.trim();
  if (!text) out.terms.push('Write the milestone: it is what both agents judge against.');
  const deadline = Date.parse(f('deadline').value);
  if (!Number.isFinite(deadline) || deadline / 1000 < Date.now() / 1000 + state.minDeadlineS + 60) out.terms.push(`The deadline must be at least ${Math.ceil(state.minDeadlineS / 60) + 1} minutes away.`);
  return out;
}

function refresh() {
  const { fee, total } = amounts();
  $('#fee-note').textContent = fee !== null ? `Review fee ${usdc(fee)} USDC: ${usdc(VERIFIER_FEE)} per review for the second opinion. Unspent fee comes back to you.` : '';
  $('#total').replaceChildren(total !== null ? amount(total, 'xl') : h('span', { class: 'ps-caption' }, 'enter a budget and the reviews allowed'));
  const n = f('text').value.trim().length;
  $('#text-note').classList.toggle('ps-label', n > 0 && n < MIN_TEXT);
  const b = blockers();
  const sections = { wallet: '#s-wallet', repo: '#s-repo', who: '#s-who', terms: '#s-terms' };
  for (const [k, sel] of Object.entries(sections)) $(sel).querySelector('[data-done]').textContent = b[k].length ? '' : '✓';
  const all = [...b.wallet, ...b.repo, ...b.who, ...b.terms];
  $('#blockers').replaceChildren(all.length
    ? h('div', { class: 'ps-blockers', role: 'status' }, h('p', { class: 'ps-label' }, `${all.length} thing${all.length === 1 ? '' : 's'} stopping this milestone`), h('ul', {}, ...all.map((x) => h('li', { class: 'ps-detail' }, x))))
    : h('p', { class: 'ps-caption' }, 'NOTHING IS MISSING'));
  $('#tx-steps').replaceChildren(
    h('li', { 'data-state': all.length ? 'todo' : 'next' }, h('span', { class: 'ps-label' }, '1. Create the milestone'), h('span', { class: 'ps-caption' }, 'from your wallet, with 0.1 NEAR for its storage; the unused part comes back')),
    h('li', { 'data-state': 'todo' }, h('span', { class: 'ps-label' }, '2. Deposit the budget and fee'), h('span', { class: 'ps-caption' }, 'same wallet prompt; nothing is claimable until it lands')));
  $('#submit').disabled = all.length > 0;
}

function openNext() {
  const b = blockers();
  const order = [['wallet', '#s-wallet'], ['repo', '#s-repo'], ['who', '#s-who'], ['terms', '#s-terms']];
  const first = order.find(([k]) => b[k].length);
  if (first) $(first[1]).open = true;
}

function pins() {
  const bd = state.builds;
  return {
    checker: { project_id: PROJECTS.checker, wasm_hash: bd.checker },
    binder: { project_id: PROJECTS.binder, wasm_hash: bd.binder },
    judge: { project_id: PROJECTS.judge, wasm_hash: bd['ai-review'] },
    verifier: { project_id: PROJECTS.verifier, wasm_hash: bd['ai-review'], fee: VERIFIER_FEE, public_key: VERIFIER_PUBLIC_KEY },
  };
}

async function submit(ev) {
  ev.preventDefault();
  refresh();
  if ($('#submit').disabled) return;
  const out = $('#out');
  const { budget, attempts, fee, total } = amounts();
  const id = await nextMilestoneId();
  const params = {
    repo_id: state.repo.id, repo_owner: state.repo.owner, repo_name: state.repo.name, base_branch: state.branch,
    payee_github_id: state.contributor.id, milestone_text: f('text').value.trim(),
    required_checks: f('checks').value.split(',').map((c) => c.trim()).filter(Boolean),
    budget: budget.toString(), fee_budget: fee.toString(),
    deadline_seconds: Math.floor(Date.parse(f('deadline').value) / 1000),
    max_paid_attempts: attempts, diff_cap_bytes: null, allow_self_merge: f('self').checked, pins: pins(),
  };
  out.replaceChildren(h('p', { class: 'ps-detail' }, `Confirm in your wallet: create milestone #${id}, then deposit ${usdc(total)} USDC into it.`));
  try {
    await wallet.send([
      { receiverId: CONTRACT, calls: [{ method: 'create_milestone', args: { params }, gas: GAS_100, deposit: CREATE_DEPOSIT.toString() }] },
      { receiverId: TOKEN.id, calls: [{ method: 'ft_transfer_call', args: { receiver_id: CONTRACT, amount: total.toString(), msg: JSON.stringify({ milestone_id: id }) }, gas: GAS_100, deposit: '1' }] },
    ]);
    try { sessionStorage.clear(); } catch { /* fine */ }
    location.href = `milestone.html?id=${id}`;
  } catch (e) {
    out.replaceChildren(problem(e.message || String(e), 'If only the first transaction went through, the milestone exists in Funding: deposit to it again from the milestone page, or refund it after the deadline.'));
  }
}

async function init() {
  const local = new Date(Date.now() + 7 * 86400_000 - new Date().getTimezoneOffset() * 60_000);
  f('deadline').value = local.toISOString().slice(0, 16);
  state.builds = Object.fromEntries((await (await fetch('builds.json')).json()).map((b) => [b.project, b.sha256]));
  state.minDeadlineS = (await getConfig().catch(() => null))?.min_deadline_seconds ?? state.minDeadlineS;
  f('contributor').addEventListener('change', lookContributor);
  form.addEventListener('input', refresh);
  form.addEventListener('submit', submit);
  wallet.onAccount(async (acct) => { state.account = acct; await renderWallet(); refresh(); openNext(); });
  wallet.restore().catch(() => {});
  await loadGithub();
  renderRepo();
  refresh();
  openNext();
}

init().catch((e) => $('main').append(problem(`Couldn't start: ${e.message}`)));
