// Shared page chrome and small helpers. Same language as the Arc site: a ruled masthead
// strip, a dashed-rule footer, the four-bar mark in currentColor, green only for released USDC.
import { TOKEN, ARC_SITE, REPO, EXPLORER_TX, CONTRACT } from './config.mjs';

export const $ = (sel, root = document) => root.querySelector(sel);

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

/// Nav and footer on every page. `active` names the current page's link.
export function shell(active) {
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
  document.body.prepend(h('nav', { class: 'ps-nav' }, h('div', { class: 'ps-nav-inner' }, mark,
    h('div', { class: 'ps-nav-links' }, link('./', 'HOME', 'home'), link('new.html', 'NEW MILESTONE', 'new'), link('milestone.html', 'MILESTONES', 'milestone'), link('claim.html', 'CLAIM', 'claim')),
    h('div', { class: 'ps-nav-actions' }, chain, toggle))));
  document.body.append(h('footer', { class: 'ps-footer ps-page' },
    h('p', { class: 'ps-caption' }, `NEAR TESTNET · ${CONTRACT} · NOTHING HERE IS REAL MONEY`),
    h('div', { class: 'ps-footer-links ps-caption' },
      h('a', { href: REPO }, 'SOURCE ↗'),
      h('a', { href: `https://testnet.nearblocks.io/address/${CONTRACT}` }, 'CONTRACT ↗'),
      h('a', { href: ARC_SITE }, 'PROOFSTREAM ON ARC ↗'))));
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
export function problem(message, detail) {
  return h('div', { class: 'ps-invert', role: 'alert' }, h('p', { class: 'ps-label' }, 'REFUSED'), h('p', { class: 'ps-body' }, message), detail ? h('p', { class: 'ps-caption ps-invert-detail' }, detail) : null);
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
