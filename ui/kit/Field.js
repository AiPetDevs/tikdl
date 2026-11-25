// Field: text input (TextField), multi-line monospace (TextArea), and a read-only path with a picker (PathField).
import { h, DesktopOnly } from './dom.js';
import { Button } from './Button.js';

export function TextField({ value = '', readonly = false, placeholder, id }) {
  return h('input', { class: 'tk-field', value, readOnly: readonly, placeholder, id, spellcheck: 'false' });
}

export function TextArea({ placeholder, id }) {
  return h('textarea', { class: 'tk-field tk-field--area', placeholder, id, spellcheck: 'false' });
}

export function PathField({ value = '', pickText = 'Папка…', onPick }) {
  const input = TextField({ value, readonly: true });
  const pick = DesktopOnly(Button({ text: pickText, onClick: onPick }));
  const el = h('div', { class: 'tk-line' }, input, pick);
  return { el, get value() { return input.value; }, set value(v) { input.value = v; input.title = v; } };
}
