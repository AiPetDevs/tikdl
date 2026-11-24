// Tabs: a segmented switch. items [{id, text}]; onChange(id). Returns {el, select(id)}.
import { h } from './dom.js';

export function Tabs({ items, value, onChange }) {
  const btns = items.map(it => h('button', {
    type: 'button', role: 'tab', class: 'tk-tab', text: it.text, id: `tab-${it.id}`,
    on: { click: () => { select(it.id); onChange?.(it.id); } },
  }));
  const el = h('div', { class: 'tk-tabs', role: 'tablist' }, btns);
  function select(id) { items.forEach((it, i) => btns[i].setAttribute('aria-selected', String(it.id === id))); }
  select(value ?? items[0].id);
  return { el, select };
}
