// Layout: Page, Stack, Line, Panel (surface with a border; accent for something that waits for an answer), Text.
import { h } from './dom.js';

export const Page = (...kids) => h('main', { class: 'tk-page tk-stack' }, ...kids);
export const Stack = (...kids) => h('div', { class: 'tk-stack' }, ...kids);
export const Line = (...kids) => h('div', { class: 'tk-line' }, ...kids);
export const Panel = ({ accent = false, role, label } = {}, ...kids) =>
  h('section', { class: `tk-panel${accent ? ' tk-panel--accent' : ''}`, role, 'aria-label': label }, ...kids);

/** Text: tone 'dim' | 'bad' | 'strong' | 'empty' (centred note for an empty list). */
export function Text(text = '', tone) {
  const cls = { dim: 'tk-text--dim', bad: 'tk-text--bad', strong: 'tk-text--strong', empty: 'tk-text--dim tk-text--center' }[tone] || '';
  return h('div', { class: cls, text });
}
