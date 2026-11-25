// icons: inline SVG, 16×16 grid, drawn with currentColor (follows the text colour of its button).
const NS = 'http://www.w3.org/2000/svg';
const PATHS = {
  pause: [['rect', { x: 4, y: 3, width: 3, height: 10, rx: 0.8, fill: 'currentColor' }], ['rect', { x: 9, y: 3, width: 3, height: 10, rx: 0.8, fill: 'currentColor' }]],
  play: [['path', { d: 'M5 3.2v9.6c0 .5.5.8.9.5l7.2-4.8a.6.6 0 0 0 0-1L5.9 2.7c-.4-.3-.9 0-.9.5z', fill: 'currentColor' }]],
  x: [['path', { d: 'M4 4l8 8M12 4l-8 8', stroke: 'currentColor', 'stroke-width': 1.6, 'stroke-linecap': 'round' }]],
  reveal: [['circle', { cx: 7, cy: 7, r: 4, stroke: 'currentColor', 'stroke-width': 1.5, fill: 'none' }], ['path', { d: 'M10 10l3.5 3.5', stroke: 'currentColor', 'stroke-width': 1.6, 'stroke-linecap': 'round' }]],
  copy: [['rect', { x: 5.5, y: 5.5, width: 7.5, height: 7.5, rx: 1.5, stroke: 'currentColor', 'stroke-width': 1.4, fill: 'none' }], ['path', { d: 'M10.5 3.5V3A1.5 1.5 0 0 0 9 1.5H4A1.5 1.5 0 0 0 2.5 3v5A1.5 1.5 0 0 0 4 9.5h.5', stroke: 'currentColor', 'stroke-width': 1.4, fill: 'none' }]],
  check: [['path', { d: 'M3.5 8.5l3 3 6-7', stroke: 'currentColor', 'stroke-width': 1.7, fill: 'none', 'stroke-linecap': 'round', 'stroke-linejoin': 'round' }]],
  down: [['path', { d: 'M8 2.5v8M4.5 7.5L8 11l3.5-3.5M3 13.5h10', stroke: 'currentColor', 'stroke-width': 1.5, fill: 'none', 'stroke-linecap': 'round', 'stroke-linejoin': 'round' }]],
  renew: [['path', { d: 'M12.5 6A5 5 0 1 0 13 9.5', stroke: 'currentColor', 'stroke-width': 1.5, fill: 'none', 'stroke-linecap': 'round' }], ['path', { d: 'M13 2.5V6H9.5', stroke: 'currentColor', 'stroke-width': 1.5, fill: 'none', 'stroke-linecap': 'round', 'stroke-linejoin': 'round' }]],
  up: [['path', { d: 'M8 11V3M4.5 6L8 2.5 11.5 6M3 13.5h10', stroke: 'currentColor', 'stroke-width': 1.5, fill: 'none', 'stroke-linecap': 'round', 'stroke-linejoin': 'round' }]],
};

/** Icon(name, small?) → <svg>. Names: pause play x reveal copy check down up renew. */
export function Icon(name, small = false) {
  const svg = document.createElementNS(NS, 'svg');
  svg.setAttribute('viewBox', '0 0 16 16');
  svg.setAttribute('aria-hidden', 'true');
  svg.setAttribute('class', small ? 'tk-icon tk-icon--small' : 'tk-icon');
  for (const [tag, attrs] of PATHS[name] || []) {
    const n = document.createElementNS(NS, tag);
    for (const [k, v] of Object.entries(attrs)) n.setAttribute(k, v);
    svg.append(n);
  }
  return svg;
}
