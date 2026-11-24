// IconButton: an icon action on the row it belongs to. With `arm` it is destructive: the first press arms it
// (red, the warning becomes its hint), the second press within 4 s runs it. With `doneIcon` the icon turns into it
// for a moment after the action succeeded (copy → check).
import { h } from './dom.js';
import { Icon } from './icons.js';

const ARM_MS = 4000, DONE_MS = 1500;

export function IconButton({ icon, help, arm, doneIcon, onClick }) {
  let armed = false, timer = 0;
  const el = h('button', { type: 'button', class: 'tk-icon-btn', title: help, 'aria-label': help }, Icon(icon, true));
  const disarm = () => { armed = false; clearTimeout(timer); el.classList.remove('tk-icon-btn--armed'); el.title = help; };
  el.addEventListener('click', ev => {
    ev.stopPropagation();
    if (arm && !armed) {
      armed = true; el.classList.add('tk-icon-btn--armed'); el.title = arm;
      timer = setTimeout(disarm, ARM_MS);
      return;
    }
    disarm();
    Promise.resolve(onClick?.()).then(() => {
      if (!doneIcon) return;
      el.replaceChildren(Icon(doneIcon, true));
      setTimeout(() => el.replaceChildren(Icon(icon, true)), DONE_MS);
    }, () => {});
  });
  el.dataset.icon = icon;
  return el;
}
