// Layout: Page, Stack, Line, Panel (surface with a border; accent for something that waits for an answer), Text.
import { h } from './dom.js';

export const Page = (...kids) => h('main', { class: 'tk-page tk-stack' }, ...kids);
export const Stack = (...kids) => h('div', { class: 'tk-stack' }, ...kids);
export const Line = (...kids) => h('div', { class: 'tk-line' }, ...kids);
export const Panel = ({ accent = false, role, label } = {}, ...kids) =>
  h('section', { class: `tk-panel${accent ? ' tk-panel--accent' : ''}`, role, 'aria-label': label }, ...kids);

/** Text: tone 'dim' | 'bad' | 'strong' | 'empty' (centred note for an empty list). */
const TONES = { dim: 'tk-text--dim', bad: 'tk-text--bad', strong: 'tk-text--strong', empty: 'tk-text--dim tk-text--center' };
export function Text(text = '', tone) {
  return h('div', { class: TONES[tone] || '', text });
}
/** Change a Text's tone in place (status lines that turn into errors). */
export function retone(el, tone) { el.className = TONES[tone] || ''; }
