// The app page: live numbers, a getting-started checklist that ticks itself, and every
// milestone with filters.
import { CONTRACT, TOKEN, NEAR_FAUCET, NEAR_WALLET, USDC_MINT_UNITS, TOKEN_STORAGE_DEPOSIT, TGAS, OUTLAYER_DOCS, RELAYER } from './config.mjs';
import { loadIndex, readMilestones, cachedIndex, stats, stateOf, tokenBalance, nearBalance } from './chain.mjs';
import { $, h, shell, amount, usdc, loading, stateChip, problem } from './ui.mjs';
import * as wallet from './wallet.mjs';

shell('app');
const GAS_30 = (30n * TGAS).toString();
const ONE_NEAR = 10n ** 24n;
let index = null;
let filter = 'all';
let account = null;

function statCell(value, label) {
  return h('div', { class: 'ps-stat' }, typeof value === 'string' ? h('span', { class: 'ps-stat-value ps-num' }, value) : value, h('span', { class: 'ps-caption' }, label));
}

function renderStats() {
  const s = stats(index);
  $('#caption').textContent = `NEAR TESTNET · ${CONTRACT.toUpperCase()} · ${s.milestones} MILESTONES`;
  $('#stats').replaceChildren(h('div', { class: 'ps-stats' },
    statCell(String(s.milestones), 'Milestones'),
    statCell(amount(s.escrowed, 'xl'), 'USDC escrowed'),
    statCell(amount(s.paid, 'xl'), 'USDC paid to contributors'),
    statCell(String(s.reviews), 'AI reviews'),
    statCell(String(s.secondOpinions), 'Second opinions bought'),
    statCell(String(s.refusals), 'Refused or held')));
}

async function github() {
  try {
    const r = await fetch('/api/github/me', { credentials: 'same-origin' });
    return r.ok ? r.json() : null;
  } catch { return null; }
}

async function mintUsdc(out) {
  out.replaceChildren(h('p', { class: 'ps-detail' }, 'Confirm in your wallet: register with the token, then mint 10 test USDC to yourself.'));
  try {
    await wallet.send([{ receiverId: TOKEN.id, calls: [
      { method: 'storage_deposit', args: { account_id: account, registration_only: true }, gas: GAS_30, deposit: TOKEN_STORAGE_DEPOSIT },
      { method: 'mint', args: { account_id: account, amount: USDC_MINT_UNITS }, gas: GAS_30, deposit: '0' },
    ] }]);
    steps();
  } catch (e) { out.replaceChildren(problem(e.message || String(e))); }
}

async function steps() {
  const hidden = (() => { try { return localStorage.getItem('ps-start-hidden') === '1'; } catch { return false; } })();
  $('#start').hidden = hidden;
  if (hidden) return;
  const [near, usdcBal, gh] = account ? await Promise.all([nearBalance(account), tokenBalance(account).catch(() => '0'), github()]) : [null, '0', await github()];
  const mine = account && index ? index.milestones.some((m) => m.funder === account) : false;
  const out = h('div');
  const items = [
    { done: !!account, label: 'Connect a wallet', note: 'This account becomes the funder of any milestone you create.',
      action: account ? null : h('button', { class: 'ps-button', type: 'button', onclick: () => wallet.connect() }, '[ Connect wallet ]'),
      extra: account ? null : h('a', { class: 'ps-caption', href: NEAR_WALLET, target: '_blank', rel: 'noopener' }, 'No testnet account? Create one ↗') },
    { done: near !== null && near >= ONE_NEAR / 2n, label: 'Get test NEAR', note: 'Pays for gas and storage. Half a NEAR is plenty.',
      action: h('a', { class: 'ps-button', href: NEAR_FAUCET, target: '_blank', rel: 'noopener' }, '[ NEAR faucet ↗ ]') },
    { done: BigInt(usdcBal || '0') >= 1_000_000n, label: 'Get test USDC', note: `${TOKEN.id} mints to anyone, so this is one wallet click.`,
      action: account ? h('button', { class: 'ps-button', type: 'button', onclick: () => mintUsdc(out) }, '[ Get 10 test USDC ]') : null },
    { done: !!gh, label: 'Connect GitHub', note: gh ? `Signed in as ${gh.login}.` : 'Pick a repository and create a branch from the form. Optional: you can type them instead.',
      action: gh ? null : h('a', { class: 'ps-button', href: '/api/github/login?next=/app.html' }, '[ Connect GitHub ]') },
    { done: mine, label: 'Create a milestone', note: 'Budget, deadline, what "done" means. One wallet prompt creates and funds it.',
      action: h('a', { class: 'ps-button', href: 'new.html' }, '[ New milestone ]') },
  ];
  const next = items.findIndex((i) => !i.done);
  const left = items.filter((i) => !i.done).length;
  $('#start-title').textContent = left ? `Getting started · ${left} step${left === 1 ? '' : 's'} left` : 'Getting started · all done';
  $('#steps').replaceChildren(...items.map((it, i) => h('li', { 'data-state': it.done ? 'done' : i === next ? 'next' : 'todo' },
    h('span', { class: 'ps-label' }, `${i + 1}. ${it.label}`), h('span', { class: 'ps-caption' }, it.note),
    i === next && (it.action || it.extra) ? h('div', { class: 'ps-actions' }, it.action, it.extra) : null)), h('li', { class: 'ps-hide-marker' }, out));
}

function matches(m) {
  const [name] = stateOf(m);
  switch (filter) {
    case 'open': return ['Funding', 'Funded', 'Checking', 'HardFactsPassed', 'Reviewing', 'Approved'].includes(name);
    case 'paid': return name === 'Paid' || m.certified_pct > 0;
    case 'held': return name === 'Held' || name === 'PayoutFailed';
    case 'mine': return account && (m.funder === account || m.payee?.account === account);
    default: return true;
  }
}

function renderTable() {
  const kinds = [['all', 'All'], ['open', 'Open'], ['paid', 'Paid'], ['held', 'Held'], ['mine', 'Mine']];
  const count = (k) => index.milestones.filter((m) => { const f = filter; filter = k; const r = matches(m); filter = f; return r; }).length;
  $('#filters').replaceChildren(...kinds.map(([k, label]) => h('button', { class: 'ps-filter', type: 'button', 'aria-pressed': String(filter === k), disabled: k === 'mine' && !account, onclick: () => { filter = k; renderTable(); } }, `${label} ${k === 'mine' && !account ? '' : count(k)}`)));
  const rows = [...index.milestones].reverse().filter(matches).map((m) => {
    const [name] = stateOf(m);
    const paid = BigInt(m.budget_total) - BigInt(m.budget);
    return h('tr', { 'data-href': `milestone.html?id=${m.id}`, onclick: () => { location.href = `milestone.html?id=${m.id}`; } },
      h('td', { class: 'ps-caption' }, `#${m.id}`),
      h('td', {}, h('a', { href: `milestone.html?id=${m.id}` }, `${m.repo_owner}/${m.repo_name}`), h('div', { class: 'ps-caption' }, `${m.base_branch}${m.parts.length ? ` · ${m.parts.length} paid PR${m.parts.length === 1 ? '' : 's'}` : ''}`)),
      h('td', { class: 'ps-hide-sm ps-detail' }, m.milestone_text.length > 90 ? `${m.milestone_text.slice(0, 90)}…` : m.milestone_text),
      h('td', {}, stateChip(name, m)),
      h('td', { class: 'ps-num' }, `${usdc(paid)} of ${usdc(m.budget_total)}`, h('span', { class: 'ps-caption' }, ' USDC')));
  });
  $('#table').replaceChildren(rows.length
    ? h('table', { class: 'ps-table' }, h('thead', {}, h('tr', {}, h('th', {}, '#'), h('th', {}, 'Repository'), h('th', { class: 'ps-hide-sm' }, 'Milestone'), h('th', {}, 'State'), h('th', {}, 'Released of budget'))), h('tbody', {}, ...rows))
    : h('p', { class: 'ps-detail' }, filter === 'mine' ? 'No milestones funded by or paying this account yet.' : 'None.'));
}

function renderStack() {
  const row = (name, text, tag, href) => h('div', { class: 'ps-stack-row' },
    href ? h('a', { href, target: '_blank', rel: 'noopener' }, name) : h('span', {}, name), h('p', { class: 'ps-detail' }, text), h('span', { class: `ps-tag${tag === 'NEXT' ? ' ps-tag-next' : ''}` }, tag));
  $('#stack').replaceChildren(
    row('NEAR testnet', `One contract, ${CONTRACT}, holds every milestone. No admin, no setter, no upgrade method; its code hash matches the reproducible build.`, 'LIVE', `https://testnet.nearblocks.io/address/${CONTRACT}`),
    row('OutLayer (Intel TDX)', 'Runs the checker, binder, judge and verifier, each pinned by wasm hash. Every run has a TDX quote anyone can check with scripts/verify-execution.py.', 'LIVE', OUTLAYER_DOCS),
    row('NEAR AI Cloud', 'GLM for the judge and Qwen for the verifier, in TEEs. Each response is signed, and the signature is checked inside the enclave.', 'LIVE', 'https://cloud.near.ai/'),
    row('In-enclave signing keys', "The verifier signs its verdict with a key only its enclave holds; the contract checks it with ed25519_verify.", 'LIVE'),
    row('USDC (NEP-141)', `${TOKEN.id}: budgets, fees and payouts. Six decimals. Test tokens anyone can mint.`, 'LIVE', `https://testnet.nearblocks.io/address/${TOKEN.id}`),
    row('Relayer', `${RELAYER.replace('https://', '')}: sends bind, claim and review for contributors without NEAR. Fixed deposits, state checked before signing.`, 'LIVE', `${RELAYER}/health`),
    row('NEAR Intents', 'Pay contributors on any chain, and fund milestones from any chain.', 'NEXT'),
    row('Open operators', 'Anyone runs the judge and verifier for a fixed fee and keeps what they do not spend.', 'NEXT'));
}

async function main() {
  $('#table').replaceChildren(loading('READING THE CHAIN'));
  $('#start-hide').addEventListener('click', () => { try { localStorage.setItem('ps-start-hidden', '1'); } catch { /* fine */ } $('#start').hidden = true; });
  renderStack();
  wallet.onAccount((acct) => { account = acct; if (index) { renderTable(); steps(); } });
  wallet.restore().catch(() => {});
  $('#stats').replaceChildren(loading('COUNTING', { inline: true }));
  const indexP = loadIndex();
  indexP.catch(() => {}); // handled where it is awaited, below
  // The table needs only the milestones, which are a few view calls; the numbers need every event.
  index = cachedIndex() ?? { milestones: await readMilestones(), txs: null };
  renderTable();
  steps();
  if (index.txs) { renderStats(); return; }
  // Keep the milestones just read live; the shared index can be a rebuild behind.
  try { index = { ...(await indexP), milestones: index.milestones }; } catch (e) { $('#stats').replaceChildren(problem(`Couldn't add up the events: ${e.message}`, 'The table below is read live from the contract.', 'NUMBERS UNAVAILABLE')); return; }
  renderStats();
  renderTable();
}

main().catch((e) => $('#table').replaceChildren(problem(`Couldn't read the chain: ${e.message}`)));
