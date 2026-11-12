// The landing page: a live caption and the AI decisions, read straight from the chain.
import { loadIndex, stats, decisions } from './chain.mjs';
import { $, h, shell, usdc, paidAmount, ago, verdictText, loading, retryLabel } from './ui.mjs';

shell('home', { landing: true });

const ROTATE_MS = 7000;

function render(d) {
  const m = d.milestone;
  const outcome = d.amount
    ? h('span', { class: 'ps-state ps-state-paid' }, `CERTIFIED ${d.pct}%`)
    : d.held
      ? h('span', { class: 'ps-state ps-state-held' }, 'HELD FOR THE FUNDER')
      : h('span', { class: 'ps-state ps-state-open' }, `${retryLabel(d)}, RETRYING`);
  const reason = d.reasons[0];
  return h('div', {},
    h('div', { class: 'ps-decision-line ps-caption' },
      h('a', { href: `milestone.html?id=${m.id}` }, `${m.repo_owner}/${m.repo_name}`), '·', `PR #${d.pr}`, '·', outcome,
      d.amount ? paidAmount(d.amount) : null),
    reason ? h('p', { class: 'ps-quote' }, `“${reason}”`) : null,
    h('p', { class: 'ps-caption' }, `JUDGE ${verdictText(d.judge)} · VERIFIER ${verdictText(d.verifier)}`));
}

async function main() {
  $('#decision').replaceChildren(loading('READING THE CHAIN', { inline: true }));
  const index = await loadIndex();
  const s = stats(index);
  $('#caption').textContent = `NEAR TESTNET · ${s.milestones} MILESTONES · ${usdc(s.paid)} USDC PAID BY THE CONTRACT`;
  const list = decisions(index).slice(0, 6);
  if (!list.length) { $('#decision').replaceChildren(h('p', { class: 'ps-detail' }, 'No reviews yet.')); return; }
  let at = 0;
  const dots = list.map((_, i) => h('button', { type: 'button', 'aria-label': `Decision ${i + 1}`, onclick: () => { at = i; show(); } }));
  $('#dots').replaceChildren(...dots);
  const show = () => {
    $('#decision').replaceChildren(render(list[at]));
    $('#decision-age').textContent = ago(list[at].ns).toUpperCase();
    dots.forEach((d, i) => d.setAttribute('aria-current', String(i === at)));
  };
  show();
  if (!matchMedia('(prefers-reduced-motion: reduce)').matches) setInterval(() => { at = (at + 1) % list.length; show(); }, ROTATE_MS);
}

main().catch(() => { $('#decision').replaceChildren(h('p', { class: 'ps-detail' }, 'Could not read the chain right now. Every decision is also on the milestone pages.')); });
