// StatusChip: the state of a row in one word. tone 'dim' | 'warn' | 'ok' | 'bad'. Null text → no chip.
import { h } from './dom.js';

export function StatusChip(chip) {
  if (!chip || !chip.text) return null;
  return h('span', { class: `tk-chip tk-chip--${chip.tone || 'dim'}`, text: chip.text, title: chip.hint || null });
}
