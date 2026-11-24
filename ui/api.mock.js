// api.mock: a stand-in backend for the browser (showcase, ui_smoke). Never loaded inside the Tauri app.
const subs = {};
const emit = (ev, payload) => (subs[ev] || []).forEach(cb => cb({ payload }));
const MB = 1024 * 1024;
const downloads = [
  { id: 'a1', link: 'tikdl://get/a1', dest: '/Users/me/Downloads', name: 'Отпуск 2026', state: 'paused', bytes: 76 * MB, total: 500 * MB, message: null },
  { id: 'a2', link: 'tikdl://get/a2', dest: '/Users/me/Downloads', name: 'Проект.zip', state: 'done', bytes: 30 * MB, total: 30 * MB, message: '/Users/me/Downloads/Проект.zip' },
];
let shares = [
  { root: 'r1', name: 'Фото', source: '/Users/me/Pictures/Фото', link: 'tikdl://get/blobacahq3x2demo', files: 12, skipped: 2, total: 640 * MB, state: 'active' },
];
const handlers = {
  inspect_link: ({ link }) => link.startsWith('tikdl://') ? { id: 'n' + link.length, name: 'Новая папка', size: 62 * MB, from: 'k7p2demonodeid' } : Promise.reject('это не ссылка TikDL'),
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
  share_path: () => null,
  list_shares: () => shares,
  stop_share: ({ root }) => { shares = shares.filter(s => s.root !== root); },
};
export const mock = {
  invoke: async (cmd, args = {}) => handlers[cmd](args),
  listen: async (ev, cb) => { (subs[ev] ||= []).push(cb); return () => {}; },
  pick: async () => null,
  reveal: async () => null,
};
