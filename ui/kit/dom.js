// dom: the one element builder of the kit. h('div', {class, text, title, on: {click}}, ...children)
export function h(tag, props = {}, ...kids) {
  const el = document.createElement(tag);
  for (const [k, v] of Object.entries(props)) {
    if (v == null || v === false) continue;
    if (k === 'class') el.className = v;
    else if (k === 'text') el.textContent = v;
    else if (k === 'on') for (const [ev, fn] of Object.entries(v)) el.addEventListener(ev, fn);
    else if (k in el && typeof v !== 'string') el[k] = v;
    else el.setAttribute(k, v === true ? '' : v);
  }
  el.append(...kids.flat().filter(x => x != null && x !== false));
  return el;
}

/** Show or hide with the kit's hidden class (never inline styles). */
export function show(el, on) { el.classList.toggle('tk-hidden', !on); return el; }
