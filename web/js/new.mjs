// The funder's flow: look the repo and contributor up on GitHub (the contract pins numeric
// ids, not names), then create and fund in one wallet prompt.
import { CONTRACT, TOKEN, PROJECTS, VERIFIER_PUBLIC_KEY, VERIFIER_FEE, CREATE_DEPOSIT, TGAS } from './config.mjs';
import { nextMilestoneId, tokenBalance, getConfig } from './chain.mjs';
import { $, h, shell, amount, usdc, toUnits, problem } from './ui.mjs';
import * as wallet from './wallet.mjs';

shell('new');
const form = $('#form');
const f = (name) => form.elements[name];
const GAS_100 = (100n * TGAS).toString();
const MIN_TEXT = 80;
let repo = null;
let contributor = null;
let builds = null;
let minDeadlineS = 600;

async function github(path) {
  const r = await fetch(`https://api.github.com/${path}`, { signal: AbortSignal.timeout(8000) });
  if (r.status === 404) return null;
  if (!r.ok) throw new Error(`GitHub answered ${r.status}`);
  return r.json();
}

async function lookRepo() {
  const v = f('repo').value.trim().replace(/^https:\/\/github\.com\//, '').replace(/\/$/, '');
  repo = null;
  if (!/^[\w.-]+\/[\w.-]+$/.test(v)) { $('#repo-note').textContent = ''; return; }
  try {
    const r = await github(`repos/${v}`);
    if (!r) { $('#repo-note').textContent = 'Not found (or private: the checker reads public repos only here).'; return; }
    repo = { id: r.id, owner: r.owner.login, name: r.name };
    $('#repo-note').textContent = `GitHub id ${r.id}${r.private ? ' · private' : ''}`;
    if (!f('branch').value) f('branch').value = r.default_branch;
  } catch (e) { $('#repo-note').textContent = e.message; }
}

async function lookContributor() {
  const v = f('contributor').value.trim().replace(/^@/, '');
  contributor = null;
  if (!/^[A-Za-z\d-]{1,39}$/.test(v)) { $('#contributor-note').textContent = ''; return; }
  try {
    const u = await github(`users/${v}`);
    if (!u) { $('#contributor-note').textContent = 'No such GitHub user.'; return; }
    contributor = { id: u.id, login: u.login };
    $('#contributor-note').textContent = `GitHub id ${u.id}. Only this account's gist can bind the payout, and only its PRs count.`;
  } catch (e) { $('#contributor-note').textContent = e.message; }
}

function amounts() {
  const budget = toUnits(f('budget').value);
  const attempts = Number(f('attempts').value);
  const fee = Number.isInteger(attempts) ? BigInt(VERIFIER_FEE) * BigInt(attempts) : null;
  return { budget, attempts, fee };
}

function refresh() {
  const { budget, fee } = amounts();
  $('#fee-note').textContent = fee !== null ? `Review fee ${usdc(fee)} USDC (${usdc(VERIFIER_FEE)} per attempt). Unspent fee comes back to you.` : '';
  $('#total').replaceChildren(budget !== null && fee !== null ? amount(budget + fee, 'xl') : h('span', { class: 'ps-caption' }, 'enter a budget and attempts'));
  const n = f('text').value.trim().length;
  $('#text-note').classList.toggle('ps-label', n > 0 && n < MIN_TEXT);
}

function problems() {
  const out = [];
  const { budget, attempts } = amounts();
  if (!repo) out.push('Look up a public repository (owner/name).');
  if (!contributor) out.push("Enter the contributor's GitHub username.");
  if (!f('branch').value.trim()) out.push('Enter the base branch.');
  const checks = f('checks').value.split(',').map((c) => c.trim()).filter(Boolean);
  if (checks.length < 1 || checks.length > 8) out.push('Name 1 to 8 required checks.');
  if (!f('text').value.trim()) out.push('Describe the milestone.');
  if (budget === null || budget < 1_000_000n) out.push('The budget must be at least 1 USDC.');
  if (!Number.isInteger(attempts) || attempts < 1 || attempts > 25) out.push('Review attempts must be 1 to 25.');
  const deadline = Date.parse(f('deadline').value);
  if (!Number.isFinite(deadline) || deadline / 1000 < Date.now() / 1000 + minDeadlineS + 60) out.push(`The deadline must be at least ${Math.ceil(minDeadlineS / 60) + 1} minutes away.`);
  return out;
}

function pins() {
  return {
    checker: { project_id: PROJECTS.checker, wasm_hash: builds.checker },
    binder: { project_id: PROJECTS.binder, wasm_hash: builds.binder },
    judge: { project_id: PROJECTS.judge, wasm_hash: builds['ai-review'] },
    verifier: { project_id: PROJECTS.verifier, wasm_hash: builds['ai-review'], fee: VERIFIER_FEE, public_key: VERIFIER_PUBLIC_KEY },
  };
}

async function submit(ev) {
  ev.preventDefault();
  const out = $('#out');
  const list = problems();
  if (list.length) { out.replaceChildren(problem('Fix these first:', list.join(' '))); return; }
  const funder = wallet.current();
  const { budget, attempts, fee } = amounts();
  const total = budget + fee;
  const have = BigInt((await tokenBalance(funder).catch(() => '0')) ?? '0');
  if (have < total) { out.replaceChildren(problem(`${funder} holds ${usdc(have)} USDC; this needs ${usdc(total)}.`, `Test USDC is the token ${TOKEN.id}.`)); return; }
  const id = await nextMilestoneId();
  const params = {
    repo_id: repo.id, repo_owner: repo.owner, repo_name: repo.name, base_branch: f('branch').value.trim(),
    payee_github_id: contributor.id, milestone_text: f('text').value.trim(),
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
    location.href = `milestone.html?id=${id}`;
  } catch (e) {
    out.replaceChildren(problem(e.message || String(e), 'If only the first transaction went through, the milestone exists in Funding: deposit to it again, or refund it after the deadline.'));
  }
}

async function init() {
  const local = new Date(Date.now() + 7 * 86400_000 - new Date().getTimezoneOffset() * 60_000);
  f('deadline').value = local.toISOString().slice(0, 16);
  builds = Object.fromEntries((await (await fetch('builds.json')).json()).map((b) => [b.project, b.sha256]));
  minDeadlineS = (await getConfig().catch(() => null))?.min_deadline_seconds ?? minDeadlineS;
  f('repo').addEventListener('change', lookRepo);
  f('contributor').addEventListener('change', lookContributor);
  form.addEventListener('input', refresh);
  form.addEventListener('submit', submit);
  $('#connect').addEventListener('click', () => (wallet.current() ? wallet.disconnect() : wallet.connect()).catch((e) => $('#out').replaceChildren(problem(e.message || String(e)))));
  wallet.onAccount((acct) => {
    $('#connect').textContent = acct ? `${acct} · disconnect` : 'Connect wallet';
    $('#submit').disabled = !acct;
    $('#submit-note').textContent = acct ? `signing as ${acct}` : 'connect a wallet first';
  });
  wallet.restore().catch(() => {});
  refresh();
}

init().catch((e) => $('main').append(problem(`Couldn't start: ${e.message}`)));
