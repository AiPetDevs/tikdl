// state: data for the screens and everything derived from it. Screens read views from here and call actions;
// they never call api directly and never compute.
import { api } from './api.js';

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
  shareError: '',
  queue: [],
  confirm: null,          // {link, info}
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
  done: d => ({ hint: d.message, chip: { text: 'готово', tone: 'ok' }, pill: { fraction: 1, text: bytes(d.total ?? d.bytes), tone: 'done' }, actions: ['reveal', 'forget'] }),
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
    const size = info.size != null ? `${bytes(info.size)} (со слов отправителя)` : 'размер неизвестен';
    S.confirm = { link, title: info.name || short(info.id), meta: `${size} · от узла ${info.from.slice(0, 10)}… · в папку ${S.dest}` };
  } catch (e) { S.linkError = String(e); emit('form'); return nextLink(); }
  emit('confirm', S.confirm);
}
export function answerConfirm(yes) {
  const c = S.confirm; S.confirm = null; emit('confirm', null);
  if (yes && c) startDownload(c.link);
  nextLink();
}
async function pullLinks() { S.queue.push(...await api.takePendingLinks()); nextLink(); }

// ---------- shares ----------
/** What a share row shows. */
export function shareView(sh) {
  const skipped = sh.skipped ? ` · пропущено скрытых: ${sh.skipped}` : '';
  const hint = `${sh.source} — ${files(sh.files)} · ${bytes(sh.total)}${skipped}`;
  if (sh.state === 'active') return { title: sh.name || baseName(sh.source), hint, chip: { text: 'раздаётся', tone: 'ok' }, link: sh.link, note: `${files(sh.files)} · ${bytes(sh.total)}`, actions: ['copy', 'stop'] };
  return { title: sh.name || baseName(sh.source), hint, chip: { text: 'файлы изменились', tone: 'bad', hint: 'Исходные файлы изменились или удалены — раздайте заново' }, link: null, pill: { fraction: 1, text: 'не раздаётся — раздайте заново', tone: 'fail' }, actions: ['drop'] };
}

export async function refreshShares() {
  try { S.shares = await api.listShares(); S.shareError = ''; } catch (e) { S.shareError = String(e); }
  emit('shares');
}
export async function share(directory) {
  const path = await api.pickPath(directory);
  if (!path) return;
  S.preparing = { done: 0, total: null, path }; S.shareError = ''; emit('shares');
  try { await api.sharePath(path); } catch (e) { S.shareError = String(e); }
  S.preparing = null;
  await refreshShares();
}
export function preparingView(p) {
  return { title: baseName(p.path || ''), hint: p.path, chip: { text: 'подготовка', tone: 'dim', hint: 'Подсчёт хешей — для больших файлов это занимает время' },
    pill: { fraction: p.total ? p.done / p.total : null, text: p.total ? `${p.done} из ${files(p.total)}` : 'подсчёт хешей…', tone: 'run' } };
}
export const shareActions = {
  copy: root => api.copy(S.shares.find(s => s.root === root)?.link || ''),
  stop: async root => { try { await api.stopShare(root); } catch (e) { S.shareError = String(e); } await refreshShares(); },
  drop: root => shareActions.stop(root),
};

// ---------- start ----------
export async function boot() {
  api.on('progress', upsert);
  api.on('share-progress', p => { if (S.preparing) { S.preparing = { ...S.preparing, ...p }; emit('shares'); } });
  api.on('node-error', e => { S.netError = `Сеть не запустилась: ${e}`; emit('net'); });
  api.on('node-ready', () => { S.netError = ''; emit('net'); refreshShares(); });
  S.dest = await api.defaultDownloadDir(); emit('form');
  await api.on('links-pending', pullLinks);   // subscribe first, then take what has piled up
  await pullLinks();
  for (const r of await api.listDownloads()) upsert({ ...r, speed: 0 });
  refreshShares();
}
