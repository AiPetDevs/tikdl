// TransferRow: one transfer. Line 1: kind icon, title, StatusChip. Line 2: body (ProgressPill / LinkPill) and the
// row's IconButtons on the same line. RowList: rows one under another with 1 px separators.
import { h } from './dom.js';
import { Icon } from './icons.js';
import { StatusChip } from './StatusChip.js';

export function TransferRow({ kind = 'down', title, hint, chip, body, actions = [] }) {
  const head = h('div', { class: 'tk-row__head' },
    h('span', { class: 'tk-row__kind' }, Icon(kind, true)),
    h('span', { class: 'tk-row__title', text: title, title: hint || title }),
    StatusChip(chip));
  const acts = h('div', { class: 'tk-row__actions' }, actions);
  const el = h('div', { class: 'tk-row' }, head, h('div', { class: 'tk-row__body' }, body, acts));
  const set = p => {
    if (p.title != null) { head.children[1].textContent = p.title; head.children[1].title = p.hint || p.title; }
    const old = head.querySelector('.tk-chip'); const chipEl = StatusChip(p.chip);
    if (old) old.remove(); if (chipEl) head.append(chipEl);
    if (p.actions) acts.replaceChildren(...p.actions);
  };
  return { el, set };
}

export function RowList() {
  return h('div', { class: 'tk-list' });
}
