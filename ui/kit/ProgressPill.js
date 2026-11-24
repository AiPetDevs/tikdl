// ProgressPill: a capsule filled to `fraction` with the numbers inside on the right ("39.8 МБ из 62.9 МБ · 8 МБ/с").
// fraction null = unknown: no fake 0 %. tone 'idle' | 'run' | 'done' | 'fail'. No animation on the value.
// LinkPill: the same capsule holding a long value (a link) on the left and a short note on the right.
import { h } from './dom.js';

export function ProgressPill({ fraction = null, text = '—', tone = 'idle', hint } = {}) {
  const fill = h('div', { class: 'tk-pill__fill' });
  const main = h('span', { class: 'tk-pill__main' });
  const el = h('div', { class: 'tk-pill', role: 'progressbar' }, fill, h('div', { class: 'tk-pill__text' }, main));
  const set = p => {
    el.className = `tk-pill tk-pill--${p.tone || 'idle'}`;
    const f = p.fraction == null ? 0 : Math.max(0, Math.min(1, p.fraction));
    fill.style.width = `${f * 100}%`;
    main.textContent = p.text || '—';
    el.title = p.hint || p.text || '';
    if (p.fraction == null) el.removeAttribute('aria-valuenow'); else el.setAttribute('aria-valuenow', Math.round(f * 100));
  };
  set({ fraction, text, tone, hint });
  return { el, set };
}

export function LinkPill({ value, note }) {
  return h('div', { class: 'tk-pill', title: value },
    h('div', { class: 'tk-pill__text' },
      h('span', { class: 'tk-pill__lead', text: value }),
      note ? h('span', { class: 'tk-pill__main', text: note }) : null));
}
