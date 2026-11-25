// api.mock: a stand-in backend for the browser (showcase, ui_smoke). Never loaded inside the Tauri app.
const subs = {};
const emit = (ev, payload) => (subs[ev] || []).forEach(cb => cb({ payload }));
const MB = 1024 * 1024;
const downloads = [
  { id: 'a1', link: 'tikdl://get/a1', dest: '/Users/me/Downloads', name: 'Отпуск 2026', state: 'paused', bytes: 76 * MB, total: 500 * MB, message: null },
  { id: 'a2', link: 'tikdl://get/a2', dest: '/Users/me/Downloads', name: 'Проект.zip', state: 'done', bytes: 30 * MB, total: 30 * MB, message: '/Users/me/Downloads/Проект.zip' },
];
let shares = [
  { id: 'once-5be1a0c3d2e4f607', root: 'r3', name: 'Договор.pdf', source: '/Users/me/Документы/Договор.pdf', link: 'tikdl://get/onceaacq2demolinkvalue', files: 1, skipped: 0, total: 3 * MB, state: 'active', once: { state: 'waiting', device: null, expires: Date.now() / 1000 + 3 * 86400 } },
  { id: 'r1', root: 'r1', name: 'Фото', source: '/Users/me/Pictures/Фото', link: 'tikdl://get/blobacahq3x2demo', files: 12, skipped: 2, total: 640 * MB, state: 'active', web_link: 'https://example.github.io/tikdl/get/#blobacahq3x2demo' },
];
const handlers = {
  peek_link: () => ({ total: 62 * MB, name: 'Новая папка', files: [['Новая папка/a.jpg', 40 * MB], ['Новая папка/b.mov', 22 * MB]], free: 120 * 1024 * MB, space_error: null }),
  inspect_link: ({ link }) => !link.startsWith('tikdl://') ? Promise.reject('это не ссылка TikDL')
    : link.startsWith('tikdl://get/once') ? { id: 'once-' + link.length, name: null, size: null, from: 'k7p2demonodeid', once: true, me: '3f9a0c1d' }
      : { id: 'n' + link.length, name: 'Новая папка', size: 62 * MB, from: 'k7p2demonodeid', once: false, me: null },
  default_download_dir: () => '/Users/me/Downloads',
  take_pending_links: () => [],
  list_downloads: () => downloads,
  start_download: ({ link }) => {
    const id = 'n' + link.length; let bytes = 0;
    const t = setInterval(() => {
      bytes += 8 * MB;
      if (bytes >= 62 * MB) { clearInterval(t); emit('progress', { id, state: 'done', bytes: 62 * MB, total: 62 * MB, message: '/Users/me/Downloads/Новая папка' }); }
      else emit('progress', { id, state: 'running', bytes, total: 62 * MB, speed: 8 * MB });
    }, 400);
    return null;
  },
  pause_download: ({ id }) => emit('progress', { id, state: 'paused', bytes: 0, total: null }),
  remove_download: () => null,
  share_path: ({ path }) => /^\/(System|usr|etc)?$/.test(path) ? Promise.reject('это системная папка — TikDL её не раздаёт') : null,
  renew_share: () => null,
  list_shares: () => shares,
  stop_share: ({ id }) => { shares = shares.filter(s => s.id !== id); },
};
export const mock = {
  invoke: async (cmd, args = {}) => handlers[cmd](args),
  listen: async (ev, cb) => { (subs[ev] ||= []).push(cb); return () => {}; },
  pick: async () => null,
  reveal: async () => null,
  onDragDrop: async () => () => {},
};
