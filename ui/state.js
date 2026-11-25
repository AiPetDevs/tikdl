// state: data for the screens and everything derived from it. Screens read views from here and call actions;
// they never call api directly and never compute.
import { api, platform } from './api.js';
export { platform };

const subs = {};
export const on = (topic, cb) => { (subs[topic] ||= []).push(cb); };
const emit = (topic, ...a) => (subs[topic] || []).forEach(cb => cb(...a));

const S = {
  netError: '',
  dest: '',
  linkError: '',
  downloads: new Map(),   // id -> {id, link, dest, name, total, bytes, speed, state, message}
  shares: [],
  preparing: null,        // {done, total} while a share is being hashed
  shareError: '',         // why the last share was refused; stays until the next share starts
  listError: '',          // the share list could not be loaded
  once: false,            // the next share gets a one-time link
  queue: [],
  confirm: null,          // {link, title, meta, peek}
  dragging: false,        // files are being dragged over the window
};
export const get = () => S;

// ---------- formatting (one place) ----------
const UNITS = ['Б', 'КБ', 'МБ', 'ГБ', 'ТБ'];
export function bytes(n) {
  if (n == null) return '—';
  let i = 0; while (n >= 1024 && i < 4) { n /= 1024; i++; }
  return `${n.toFixed(i ? 1 : 0)} ${UNITS[i]}`;
}
const short = id => `${id.slice(0, 16)}…`;
const baseName = path => path.split(/[\\/]/).filter(Boolean).pop() || path;
/** A path as the user names it in a message: its last part; a disk root ("C:\\", "/") as is. */
const pathLabel = path => { const b = baseName(path); return /:$/.test(b) ? path : b; };
const DAY = 86400;
/** "через 3 дн." / "через 5 ч" — when a one-time link stops by itself. */
function until(expires) {
  const left = expires - Date.now() / 1000;
  if (left <= 0) return 'скоро';
  return left >= DAY ? `через ${Math.round(left / DAY)} дн.` : `через ${Math.max(1, Math.round(left / 3600))} ч`;
}
const files = n => `${n} ${n % 10 === 1 && n % 100 !== 11 ? 'файл' : n % 10 >= 2 && n % 10 <= 4 && (n % 100 < 10 || n % 100 >= 20) ? 'файла' : 'файлов'}`;

// ---------- downloads ----------
/** What a download row shows: title, chip, pill, and which actions it has. Unknown size = "передано N", not 0 %. */
function amountOf(d) {
  if (d.total != null) return `${bytes(Math.min(d.bytes || 0, d.total))} из ${bytes(d.total)}`;
  return d.bytes ? `передано ${bytes(d.bytes)}` : '';
}
const BY_STATE = {
  running: (d, c) => d.message
    ? { chip: { text: 'подключение', tone: 'dim' }, pill: { fraction: c.fraction, text: c.amount || d.message, tone: 'run' }, actions: ['pause', 'remove'] }
    : { chip: null, pill: { fraction: c.fraction, text: `${c.amount || '—'} · ${bytes(d.speed || 0)}/с`, tone: 'run' }, actions: ['pause', 'remove'] },
  paused: (d, c) => ({ chip: { text: 'на паузе', tone: 'warn', hint: 'Скачанное сохранено — ▶ продолжит с того же места, даже после перезапуска' },
    pill: { fraction: c.fraction, text: c.amount || '—', tone: 'idle' }, actions: ['resume', 'remove'] }),
  error: (d, c) => ({ chip: { text: 'сбой', tone: 'bad', hint: d.message },
    pill: { fraction: c.fraction, text: d.message || 'ошибка', tone: 'fail', hint: d.message }, actions: ['resume', 'remove'] }),
  done: d => ({ hint: d.message, chip: { text: 'готово', tone: 'ok' }, pill: { fraction: 1, text: bytes(d.total ?? d.bytes), tone: 'done' }, actions: platform === 'android' ? ['forget'] : ['reveal', 'forget'] }),
  queued: () => ({ chip: { text: 'в очереди', tone: 'dim' }, pill: { fraction: null, text: '—', tone: 'idle' }, actions: ['remove'] }),
};
export function downloadView(d) {
  const title = d.name || short(d.id);
  const c = { amount: amountOf(d), fraction: d.total ? (d.bytes || 0) / d.total : null };
  const v = (BY_STATE[d.state] || BY_STATE.queued)(d, c);
  return { title, ...v, hint: v.hint || `${title} → ${d.dest}` };
}

function upsert(p) {
  const d = S.downloads.get(p.id) || { id: p.id };
  for (const k of ['link', 'dest', 'name', 'state', 'message']) if (k in p) d[k] = p[k];
  if (p.total != null) d.total = p.total;
  // a pause event carries no byte count: keep what we had
  if (p.bytes || d.bytes == null) d.bytes = p.bytes || d.bytes || 0;
  d.speed = p.speed || 0;
  const isNew = !S.downloads.has(p.id);
  S.downloads.set(p.id, d);
  emit(isNew ? 'download:new' : 'download', d);
}

async function run(id, link, dest) {
  upsert({ id, state: 'running', message: 'Подключение…' });
  try { await api.startDownload(link, dest); } catch (e) { upsert({ id, state: 'error', message: String(e) }); }
}

export async function startDownload(link) {
  S.linkError = ''; emit('form');
  let info;
  try { info = await api.inspectLink(link); } catch (e) { S.linkError = String(e); emit('form'); return false; }
  upsert({ id: info.id, link, dest: S.dest, name: info.name, total: info.size, state: 'running', message: 'Подключение…' });
  await run(info.id, link, S.dest);
  return true;
}

export const actions = {
  pause: id => api.pauseDownload(id),
  resume: id => { const d = S.downloads.get(id); return run(id, d.link, d.dest); },
  remove: async id => { await api.removeDownload(id); S.downloads.delete(id); emit('download:gone', id); },
  forget: id => actions.remove(id),
  reveal: id => { const d = S.downloads.get(id); if (d?.message) api.reveal(d.message); },
};

export async function pickDest() {
  const d = await api.pickFolder();
  if (d) { S.dest = d; emit('form'); }
}

// ---------- incoming links: only after the user confirms ----------
async function nextLink() {
  if (S.confirm || !S.queue.length) return;
  const link = S.queue.shift();
  try {
    const info = await api.inspectLink(link);
    if (info.once) {
      S.confirm = { link, once: true, title: 'Одноразовая ссылка',
        meta: `Имя и размер скрыты. Скачать сможет одно устройство: «Что внутри?» и «Скачать» закрепят ссылку за этим компьютером${info.me ? ` (его код ${info.me})` : ''} · в папку ${S.dest}` };
    } else {
      const size = info.size != null ? `${bytes(info.size)} (со слов отправителя)` : 'размер неизвестен';
      S.confirm = { link, title: info.name || short(info.id), meta: `${size} · от узла ${info.from.slice(0, 10)}… · в папку ${S.dest}` };
    }
  } catch (e) { S.linkError = String(e); emit('form'); return nextLink(); }
  emit('confirm', S.confirm);
}
/** "Что внутри?": only on the user's request the node contacts the sender before the decision. */
export async function peekConfirm() {
  const c = S.confirm; if (!c) return;
  c.peek = { state: 'loading' }; emit('confirm', c);
  try { c.peek = { state: 'done', ...await api.peekLink(c.link, S.dest) }; }
  catch (e) { c.peek = { state: 'error', error: String(e) }; }
  if (S.confirm === c) emit('confirm', c);
}
const MAX_FILES_SHOWN = 8;
/** What the confirm card shows after "Что внутри?". */
export function peekView(p) {
  if (!p) return null;
  if (p.state === 'loading') return { status: { text: 'Спрашиваем у отправителя…', tone: 'dim' }, files: [] };
  if (p.state === 'error') return { status: { text: p.error, tone: 'bad' }, files: [] };
  const list = p.files.slice(0, MAX_FILES_SHOWN).map(([name, size]) => ({ name, size: bytes(size) }));
  const more = p.files.length - list.length;
  const free = p.free != null ? ` · свободно ${bytes(p.free)}` : '';
  const status = p.space_error ? { text: p.space_error, tone: 'bad' } : { text: `${files(p.files.length)} · ${bytes(p.total)}${free}`, tone: 'dim' };
  return { status, files: list, more: more > 0 ? `и ещё ${files(more)}` : '' };
}
export function answerConfirm(yes) {
  const c = S.confirm; S.confirm = null; emit('confirm', null);
  if (yes && c) startDownload(c.link);
  nextLink();
}
async function pullLinks() { S.queue.push(...await api.takePendingLinks()); nextLink(); }

// ---------- shares ----------
/** A one-time share by its stage: waiting (link to hand out) → claimed by a device → received (burned) | expired. */
const ONCE = {
  waiting: (sh, c) => sh.link
    ? { chip: { text: 'одноразовая · ждёт', tone: 'dim', hint: 'Скачать сможет одно устройство — то, что откроет ссылку первым. После получения ссылка сгорает' },
      link: shareLink(sh), note: `${c.size} · сгорит ${until(sh.once.expires)}`, actions: ['copy', 'renew', 'stop'] }
    : { chip: { text: 'одноразовая', tone: 'dim' }, pill: { fraction: null, text: 'подключение к сети…', tone: 'idle' }, actions: ['stop'] },
  claimed: (sh, c) => ({ chip: { text: 'закреплена', tone: 'warn', hint: `Ссылку открыло устройство ${sh.once.device} — скачать может только оно. Не тот код? «Новая ссылка» отменит эту` },
    pill: { fraction: null, text: `устройство ${sh.once.device} · ${c.size}`, tone: 'run' }, actions: ['renew', 'stop'] }),
  used: sh => ({ chip: { text: 'получено', tone: 'ok' },
    pill: { fraction: 1, text: `получено устройством ${sh.once.device} — ссылка сгорела`, tone: 'done' }, actions: ['drop'] }),
  expired: () => ({ chip: { text: 'срок истёк', tone: 'warn', hint: 'Ссылка перестала работать сама. «Новая ссылка» даст новую на те же файлы' },
    pill: { fraction: null, text: 'ссылка истекла', tone: 'idle' }, actions: ['renew', 'drop'] }),
};

/** What a share row shows. */
export function shareView(sh) {
  const skipped = sh.skipped ? ` · пропущено скрытых: ${sh.skipped}` : '';
  const hint = `${sh.source} — ${files(sh.files)} · ${bytes(sh.total)}${skipped}`;
  const title = sh.name || baseName(sh.source);
  const size = `${files(sh.files)} · ${bytes(sh.total)}`;
  // A one-time link that is already received or expired stays what it is, even if its source files changed since.
  const final = sh.once && (sh.once.state === 'used' || sh.once.state === 'expired');
  if (sh.once && (sh.state === 'active' || (final && sh.state === 'changed'))) return { title, hint, ...(ONCE[sh.once.state] || ONCE.expired)(sh, { size }) };
  if (sh.state === 'active') return { title, hint, chip: { text: 'раздаётся', tone: 'ok' }, link: shareLink(sh), note: size, actions: ['copy', 'stop'] };
  // Made by an older version from a folder TikDL no longer shares (system, whole home, program keys).
  if (sh.state === 'forbidden') return { title, hint, chip: { text: 'запрещено', tone: 'bad', hint: sh.why || 'Системные и личные папки TikDL не раздаёт — уберите эту раздачу' }, link: null, pill: { fraction: 1, text: `не раздаётся: ${sh.why || 'такие папки TikDL не раздаёт'}`, tone: 'fail', hint: sh.why }, actions: ['drop'] };
  return { title, hint, chip: { text: 'файлы изменились', tone: 'bad', hint: 'Исходные файлы изменились или удалены — раздайте заново' }, link: null, pill: { fraction: 1, text: 'не раздаётся — раздайте заново', tone: 'fail' }, actions: ['drop'] };
}

/** Reload the share list. Never touches shareError: a refusal stays visible until the next share starts. */
export async function refreshShares() {
  try { S.shares = await api.listShares(); S.listError = ''; } catch (e) { S.listError = String(e); }
  emit('shares');
}
/** The red line under the share buttons: the last refusal, or a list error. */
export const shareErrorView = () => S.shareError || S.listError;
export function setOnce(on) { S.once = !!on; emit('shares'); }
/** The link to hand out: https (clickable in messengers) when the build has a landing page, else tikdl://. */
const shareLink = sh => sh.web_link || sh.link;

/** Share several files/folders one after another (picker with several items, or dropped onto the window).
 *  Errors (e.g. "это системная папка — TikDL её не раздаёт") are shown after the list refresh,
 *  because refreshShares() clears the error line on success. */
export async function shareMany(paths) {
  if (!paths.length) return;
  const errors = [];
  const once = S.once;
  S.shareError = ''; emit('shares');
  for (const path of paths) {
    S.preparing = { done: 0, total: null, path }; emit('shares');
    // each refusal shows at once (the list refresh below no longer wipes it)
    try { await api.sharePath(path, once); } catch (e) { errors.push(`${pathLabel(path)}: ${e}`); S.shareError = errors.join('; '); }
    S.preparing = null;
    await refreshShares();
  }
}
export async function share(directory) { await shareMany(await api.pickPaths(directory)); }
export function preparingView(p) {
  return { title: baseName(p.path || ''), hint: p.path, chip: { text: 'подготовка', tone: 'dim', hint: 'Подсчёт хешей — для больших файлов это занимает время' },
    pill: { fraction: p.total ? p.done / p.total : null, text: p.total ? `${p.done} из ${files(p.total)}` : 'подсчёт хешей…', tone: 'run' } };
}
export const shareActions = {
  copy: id => { const sh = S.shares.find(s => s.id === id); return api.copy(sh ? shareLink(sh) : ''); },
  stop: async id => { try { await api.stopShare(id); } catch (e) { S.shareError = String(e); } await refreshShares(); },
  drop: id => shareActions.stop(id),
  renew: async id => { try { await api.renewShare(id); } catch (e) { S.shareError = String(e); } await refreshShares(); },
};

// ---------- start ----------
export async function boot() {
  api.on('progress', upsert);
  api.onDragDrop(p => {
    const over = p.type === 'enter' || p.type === 'over';
    if (over !== S.dragging) { S.dragging = over; emit('drag', over); }
    if (p.type === 'drop' && p.paths?.length) { emit('goto', 'share'); shareMany(p.paths); }
  });
  api.on('share-progress', p => { if (S.preparing) { S.preparing = { ...S.preparing, ...p }; emit('shares'); } });
  api.on('node-error', e => { S.netError = `Сеть не запустилась: ${e}`; emit('net'); });
  api.on('node-ready', () => { S.netError = ''; emit('net'); refreshShares(); });
  api.on('shares-changed', () => refreshShares());
  S.dest = await api.defaultDownloadDir(); emit('form');
  await api.on('links-pending', pullLinks);   // subscribe first, then take what has piled up
  await pullLinks();
  for (const r of await api.listDownloads()) upsert({ ...r, speed: 0 });
  refreshShares();
}
