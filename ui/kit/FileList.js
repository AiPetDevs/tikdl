// FileList: what is inside a transfer — name on the left, size on the right, one line each; `more` = "и ещё N".
import { h } from './dom.js';

export function FileList({ files = [], more = '' } = {}) {
  const el = h('ul', { class: 'tk-files' });
  const set = p => {
    el.replaceChildren(...(p.files || []).map(f => h('li', { class: 'tk-files__row', title: f.name },
      h('span', { class: 'tk-files__name', text: f.name }), h('span', { class: 'tk-files__size', text: f.size }))));
    if (p.more) el.append(h('li', { class: 'tk-files__more', text: p.more }));
    el.classList.toggle('tk-hidden', !el.children.length);
  };
  set({ files, more });
  return { el, set };
}
