// Switch: on/off for a mode that applies to what the user does next (e.g. "one-time link" for the next share).
// The label sits right after the switch; the hint explains what "on" means. Returns {el, value}.
import { h } from './dom.js';

export function Switch({ label, hint, value = false, onChange, id }) {
  const input = h('input', { type: 'checkbox', role: 'switch', class: 'tk-switch__input', id, checked: value,
    on: { change: () => onChange?.(input.checked) } });
  const el = h('label', { class: 'tk-switch', title: hint || null }, input, h('span', { class: 'tk-switch__track', 'aria-hidden': 'true' }), h('span', { text: label }));
  return { el, get value() { return input.checked; }, set value(v) { input.checked = !!v; } };
}
