// DropOverlay: covers the window while files are dragged over it ("Отпустите — начнём раздавать").
import { h, show } from './dom.js';
import { Icon } from './icons.js';

export function DropOverlay({ text }) {
  const el = h('div', { class: 'tk-drop', 'aria-hidden': 'true' },
    h('div', { class: 'tk-drop__box' }, Icon('up'), h('div', { text })));
  show(el, false);
  return { el, set: on => show(el, on) };
}
