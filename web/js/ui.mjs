// Shared page chrome and small helpers. Same language as the Arc site: a ruled masthead
// strip, a dashed-rule footer, the four-bar mark in currentColor, green only for released USDC.
import { TOKEN, ARC_SITE, REPO, EXPLORER_TX, CONTRACT, NEAR_FAUCET } from './config.mjs';

export const $ = (sel, root = document) => root.querySelector(sel);

/// Replaces an element's children, skipping the empty entries a conditional leaves behind.
export function fill(el, ...children) {
  el.replaceChildren(...children.flat().filter((c) => c != null && c !== false));
}

/// Builds DOM from a tag, attributes and children. Text is always set as text, never HTML.
export function h(tag, attrs = {}, ...children) {
  const el = document.createElement(tag);
  for (const [k, v] of Object.entries(attrs)) {
    if (v === false || v == null) continue;
    if (k === 'class') el.className = v;
    else if (k.startsWith('on')) el.addEventListener(k.slice(2), v);
    else el.setAttribute(k, v === true ? '' : v);
  }
  for (const c of children.flat()) if (c != null && c !== false) el.append(c instanceof Node ? c : String(c));
  return el;
}

const MARK = `<svg viewBox="0 0 64 64" width="20" height="20" shape-rendering="crispEdges" aria-hidden="true" focusable="false">
<rect x="24" y="48" width="34" height="10" fill="currentColor"/><rect x="18" y="34" width="34" height="10" fill="currentColor"/>
<rect x="13" y="21" width="32" height="8" fill="none" stroke="currentColor" stroke-width="2"/>
<rect x="8" y="8" width="32" height="8" fill="none" stroke="currentColor" stroke-width="2"/></svg>`;

function storedTheme() {
  try { return localStorage.getItem('ps-theme'); } catch { return null; }
}

function setTheme(t) {
  document.documentElement.dataset.theme = t;
  try { localStorage.setItem('ps-theme', t); } catch { /* private mode: still works for this page */ }
}

/// Nav and footer on every page. `active` names the current page's link. The landing page
/// gets Arc's landing strip: the wordmark, the chain switch and one way in, no wallet.
export function shell(active, { landing = false } = {}) {
  const saved = storedTheme();
  if (saved === 'light' || saved === 'dark') document.documentElement.dataset.theme = saved;
  const link = (href, label, key) => h('a', { href, class: `ps-nav-link${key === active ? ' ps-nav-link-active' : ''}` }, label);
  const mark = h('a', { href: './', class: 'ps-nav-mark', 'aria-label': 'ProofStream on NEAR, home' });
  mark.innerHTML = `${MARK}<span>PROOFSTREAM</span>`;
  const isDark = () => (document.documentElement.dataset.theme || (matchMedia('(prefers-color-scheme: dark)').matches ? 'dark' : 'light')) === 'dark';
  const toggle = h('button', { class: 'ps-theme-toggle', type: 'button', 'aria-label': 'Switch light or dark theme' }, isDark() ? '☼' : '☾');
  toggle.addEventListener('click', () => { setTheme(isDark() ? 'light' : 'dark'); toggle.textContent = isDark() ? '☼' : '☾'; });
  const chain = h('span', { class: 'ps-chain-switch', role: 'group', 'aria-label': 'Which chain' },
    h('a', { href: ARC_SITE, class: 'ps-chip ps-chip-link' }, 'ARC'),
    h('span', { class: 'ps-chip ps-chip-current', 'aria-current': 'true' }, 'NEAR'));
  const nav = h('nav', { class: 'ps-nav' });
  if (landing) {
    nav.append(h('div', { class: 'ps-nav-inner' }, mark, h('div', { class: 'ps-nav-actions ps-nav-actions-landing' }, chain, h('a', { class: 'ps-button', href: 'app.html' }, '[ GO TO APP ]'), toggle)));
  } else {
    const wallet = h('button', { class: 'ps-button ps-wallet-btn', type: 'button', id: 'nav-wallet' }, '[ CONNECT WALLET ]');
    const menu = h('button', { class: 'ps-nav-menu', type: 'button', 'aria-expanded': 'false' }, '[ MENU ]');
    menu.addEventListener('click', () => {
      const open = nav.classList.toggle('ps-nav-open');
      menu.setAttribute('aria-expanded', String(open));
      menu.textContent = open ? '[ CLOSE ]' : '[ MENU ]';
    });
    nav.append(h('div', { class: 'ps-nav-inner' }, mark, menu,
      h('div', { class: 'ps-nav-links' }, link('app.html', 'MILESTONES', 'app'), link('new.html', 'NEW MILESTONE', 'new'), link('claim.html', 'CLAIM', 'claim'), link('docs.html', 'HOW IT WORKS', 'docs')),
      h('div', { class: 'ps-nav-actions' }, chain, wallet, toggle)));
    import('./wallet.mjs').then((w) => {
      wallet.addEventListener('click', () => (w.current() ? w.disconnect() : w.connect()).catch(() => {}));
      w.onAccount((acct) => { wallet.textContent = acct ? `[ ${short(acct, 18)} ]` : '[ CONNECT WALLET ]'; wallet.title = acct ? `${acct}. Click to disconnect.` : ''; });
      w.restore().catch(() => {});
    });
  }
  document.body.prepend(nav);
  document.body.append(h('footer', { class: 'ps-footer ps-page' },
    h('div', { class: 'ps-footer-links ps-caption' },
      h('a', { href: 'docs.html' }, 'HOW IT WORKS'),
      h('a', { href: NEAR_FAUCET, target: '_blank', rel: 'noopener' }, 'GET TEST NEAR ↗'),
      h('a', { href: `https://testnet.nearblocks.io/address/${CONTRACT}`, target: '_blank', rel: 'noopener' }, 'CONTRACT ↗'),
      h('a', { href: REPO, target: '_blank', rel: 'noopener' }, 'SOURCE ↗'),
      h('a', { href: ARC_SITE }, 'PROOFSTREAM ON ARC ↗')),
    h('p', { class: 'ps-caption' }, `NEAR TESTNET · ${CONTRACT} · NOTHING HERE IS REAL MONEY`)));
}

/// The loading state: the mark's bars filling top to bottom, in a loop.
export function loading(word = 'PROOFSTREAM', { inline = false } = {}) {
  return h('div', { class: `ps-loading${inline ? ' ps-loading-inline' : ''}`, role: 'status', 'aria-label': 'Loading' },
    h('div', { class: 'ps-loading-mark', 'aria-hidden': 'true' }, h('span'), h('span'), h('span'), h('span')),
    h('p', { class: 'ps-loading-word' }, word));
}

/// The mark beside a page title, as on the Arc app.
export function titleMark() {
  return h('div', { class: 'ps-title-mark', 'aria-hidden': 'true' }, h('span'), h('span'), h('span'), h('span'));
}

/// Why a review paid nothing and retried: the judge's own run failed, or the judge answered
/// and only the verifier's second opinion was lost.
export function retryLabel(d) {
  return /^Failed/.test(d.judge ?? '') ? 'RUN FAILED' : 'SECOND OPINION LOST';
}

/// A state name as a chip. Green only when money was released.
export function stateChip(name, m) {
  const paid = name === 'Paid' || (m && m.certified_pct > 0 && name === 'Funded');
  const cls = name === 'Paid' ? 'ps-state-paid' : name === 'Held' || name === 'PayoutFailed' ? 'ps-state-held'
    : name === 'Refunded' ? 'ps-state-closed' : 'ps-state-open';
  const label = name === 'Funded' && m?.certified_pct > 0 ? `${m.certified_pct}% PAID` : name.replace(/([a-z])([A-Z])/g, '$1 $2');
  return h('span', { class: `ps-state ${paid && name !== 'Paid' ? 'ps-state-paid' : cls}` }, label);
}

/// Accepts "Progress(60)", "Objects(\"unclear\")" or "Failed" and says it in words.
export function verdictText(v) {
  if (!v) return 'no verdict';
  const p = /^Progress\((\d+)\)$/.exec(v);
  if (p) return `${p[1]}% done`;
  const o = /^Objects\("?([a-z_]+)"?\)$/.exec(v);
  if (o) return o[1].replaceAll('_', ' ');
  return 'no verdict';
}

/// Token units → "1.20" style text, never floating point.
export function usdc(units) {
  const v = BigInt(units ?? 0);
  const scale = 10n ** BigInt(TOKEN.decimals);
  const whole = v / scale;
  const frac = (v % scale).toString().padStart(TOKEN.decimals, '0').replace(/0+$/, '').padEnd(2, '0');
  return `${whole}.${frac}`;
}

/// "1.2" → token units, or null if it isn't a plain positive decimal.
export function toUnits(text) {
  const m = /^\s*(\d{1,12})(?:\.(\d{0,6}))?\s*$/.exec(text ?? '');
  if (!m) return null;
  return BigInt(m[1]) * 10n ** BigInt(TOKEN.decimals) + BigInt((m[2] || '').padEnd(TOKEN.decimals, '0'));
}

/// USDC that reached a contributor, as a green chip: green only for released USDC.
export const paidAmount = (units) => h('span', { class: 'ps-state ps-state-paid' }, `${usdc(units)} USDC`);

export function amount(units, size = 'm') {
  const [whole, frac] = usdc(units).split('.');
  return h('span', { class: `ps-amount ps-amount-${size} ps-num` },
    h('span', { class: 'ps-amount-whole' }, whole), h('span', { class: 'ps-amount-fraction' }, `.${frac}`),
    h('span', { class: 'ps-amount-suffix' }, TOKEN.symbol));
}

export const txLink = (hash, label = 'TX ↗') => h('a', { href: EXPLORER_TX(hash), class: 'ps-chip ps-chip-link', target: '_blank', rel: 'noopener' }, label);

export function short(s, n = 8) {
  return s && s.length > n + 2 ? `${s.slice(0, n)}…` : s;
}

/// A copy-to-clipboard chip. `text` is a string, or a function read at click time.
export function copyChip(text, label = 'COPY') {
  const chip = h('button', { type: 'button', class: 'ps-chip' }, label);
  chip.addEventListener('click', async () => {
    try { await navigator.clipboard.writeText(typeof text === 'function' ? text() : text); chip.textContent = 'COPIED'; chip.classList.add('ps-chip-copied'); } catch { chip.textContent = 'SELECT AND COPY'; }
    setTimeout(() => { chip.textContent = label; chip.classList.remove('ps-chip-copied'); }, 1600);
  });
  return chip;
}

/// The error treatment: an inverted panel. The palette has no red.
export function problem(message, detail, title = 'REFUSED') {
  return h('div', { class: 'ps-invert', role: 'alert' }, h('p', { class: 'ps-label' }, title), h('p', { class: 'ps-body' }, message), detail ? h('p', { class: 'ps-caption ps-invert-detail' }, detail) : null);
}

export function sectionRule(label) {
  return h('div', { class: 'ps-section-rule' }, h('h2', { class: 'ps-label' }, label));
}

export function ago(ns) {
  const s = Math.max(0, Math.round((Date.now() - Number(BigInt(ns) / 1_000_000n)) / 1000));
  if (s < 60) return `${s}s ago`;
  if (s < 3600) return `${Math.floor(s / 60)}m ago`;
  if (s < 86400) return `${Math.floor(s / 3600)}h ago`;
  return `${Math.floor(s / 86400)}d ago`;
}

/// Waits until `check` returns something truthy, every `everyMs`, at most `maxMs`.
export async function until(check, { everyMs = 3000, maxMs = 120000 } = {}) {
  const end = Date.now() + maxMs;
  while (Date.now() < end) {
    const v = await check().catch(() => null);
    if (v) return v;
    await new Promise((r) => setTimeout(r, everyMs));
  }
  return null;
}
