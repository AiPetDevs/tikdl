// Button: a text button. variant 'primary' | 'plain'; wide stretches it on its line.
import { h } from './dom.js';

export function Button({ text, variant = 'plain', wide = false, onClick, id }) {
  return h('button', {
    type: 'button', id, text,
    class: `tk-btn${variant === 'primary' ? ' tk-btn--primary' : ''}${wide ? ' tk-grow' : ''}`,
    on: onClick ? { click: onClick } : undefined,
  });
}
