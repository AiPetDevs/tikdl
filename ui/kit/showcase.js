// showcase: every kit component in every state (#/kit). Data goes through state.js views, the same as the screens.
import { h } from './dom.js';
import { Page, Panel, Line, Text, Stack } from './Layout.js';
import { Button } from './Button.js';
import { IconButton } from './IconButton.js';
import { StatusChip } from './StatusChip.js';
import { ProgressPill, LinkPill } from './ProgressPill.js';
import { TransferRow, RowList } from './TransferRow.js';
import { Tabs } from './Tabs.js';
import { Switch } from './Switch.js';
import { TextArea, TextField, PathField } from './Field.js';
import { FileList } from './FileList.js';
import { DropOverlay } from './DropOverlay.js';
import { downloadView, shareView, preparingView, peekView } from '../state.js';

const MB = 1024 * 1024;
const section = (title, ...kids) => Stack(h('h2', { class: 'tk-show__h', text: title }), ...kids);
const label = t => h('div', { class: 'tk-show__label', text: t });
const ICONS = { pause: 'pause', resume: 'play', remove: 'x', forget: 'x', reveal: 'reveal', copy: 'copy', stop: 'x', drop: 'x', renew: 'renew' };
const acts = (keys, arm) => keys.map(k => IconButton({ icon: ICONS[k], help: k, arm: arm && k === 'remove' ? 'Нажмите ещё раз' : undefined, doneIcon: k === 'copy' ? 'check' : undefined }));
const dlRow = d => { const v = downloadView(d); return TransferRow({ kind: 'down', title: v.title, hint: v.hint, chip: v.chip, body: ProgressPill(v.pill).el, actions: acts(v.actions, true) }).el; };
const shRow = s => { const v = shareView(s); const body = v.link ? LinkPill({ value: v.link, note: v.note }) : ProgressPill(v.pill).el; return TransferRow({ kind: 'up', title: v.title, chip: v.chip, body, actions: acts(v.actions) }).el; };

export function Showcase() {
  const base = { id: 'f3a9c1d2e4b5a6978877665544332211', link: 'tikdl://get/x', dest: '/Users/me/Downloads', name: 'Отпуск 2026' };
  const dl = RowList();
  dl.append(
    dlRow({ ...base, state: 'running', message: 'Подключение…', bytes: 0, total: 62.9 * MB }),
    dlRow({ ...base, state: 'running', bytes: 39.8 * MB, total: 62.9 * MB, speed: 8 * MB }),
    dlRow({ ...base, name: null, state: 'running', bytes: 12 * MB, total: null, speed: 3 * MB }),
    dlRow({ ...base, state: 'paused', bytes: 76.2 * MB, total: 500 * MB }),
    dlRow({ ...base, state: 'error', bytes: 5 * MB, total: 62.9 * MB, message: 'не удалось скачать: раздача остановлена, её файлы изменились или раздающий не в сети' }),
    dlRow({ ...base, state: 'done', bytes: 62.9 * MB, total: 62.9 * MB, message: '/Users/me/Downloads/Отпуск 2026' }),
    dlRow({ ...base, state: 'queued', bytes: 0, total: null }));
  const sh = RowList();
  sh.append(
    TransferRow({ kind: 'up', ...preparingView({ path: '/Users/me/Видео', done: 3, total: 12 }), body: ProgressPill(preparingView({ done: 3, total: 12 }).pill).el }).el,
    shRow({ root: 'r1', source: '/Users/me/Pictures/Фото', link: 'tikdl://get/blobacahq3x2kq7demolinkvalue', files: 12, skipped: 2, total: 640 * MB, state: 'active' }),
    shRow({ root: 'r2', source: '/Users/me/Документы/Отчёт.pdf', link: '', files: 1, skipped: 0, total: 2 * MB, state: 'changed' }),
    shRow({ root: 'r4', source: '/Users/me', link: '', files: 3, skipped: 0, total: 9 * MB, state: 'forbidden', why: 'это вся домашняя папка — раздайте нужную папку внутри неё' }));
  const soon = Date.now() / 1000 + 3 * 86400;
  const once = (state, extra = {}) => shRow({ root: 'o', source: '/Users/me/Документы/Договор.pdf', link: '', files: 1, skipped: 0, total: 3 * MB, state: 'active', once: { state, device: '3f9a0c1d', expires: soon }, ...extra });
  const oneTime = RowList();
  oneTime.append(once('waiting', { link: 'tikdl://get/onceaacq2demolinkvalue' }), once('waiting'), once('claimed'), once('used'), once('expired'));
  const tabs = Tabs({ items: [{ id: 'a', text: 'Скачать' }, { id: 'b', text: 'Раздать' }] });

  return Page(
    h('h1', { class: 'tk-show__h', text: 'Кит TikDL — витрина' }),
    section('Tabs', tabs.el),
    section('Switch', Switch({ label: 'Одноразовая ссылка', hint: 'выкл' }).el, Switch({ label: 'Одноразовая ссылка', value: true, hint: 'вкл' }).el),
    section('Button', Line(Button({ text: 'Скачать', variant: 'primary', wide: true }), Button({ text: 'Папка…' }), Button({ text: 'Отмена' }))),
    section('IconButton', Line(...['pause', 'play', 'reveal', 'copy', 'renew'].map(i => IconButton({ icon: i, help: i })),
      IconButton({ icon: 'x', help: 'с подтверждением', arm: 'Нажмите ещё раз' })), label('✕ — первое нажатие взводит (красный), второе выполняет')),
    section('StatusChip', Line(...[['подключение', 'dim'], ['на паузе', 'warn'], ['готово', 'ok'], ['сбой', 'bad']].map(([text, tone]) => StatusChip({ text, tone })))),
    section('ProgressPill', label('idle / run / done / fail / размер неизвестен'),
      ProgressPill({ fraction: 0.15, text: '76.2 МБ из 500.0 МБ' }).el, ProgressPill({ fraction: 0.63, text: '39.8 МБ из 62.9 МБ · 8.0 МБ/с', tone: 'run' }).el,
      ProgressPill({ fraction: 1, text: '62.9 МБ', tone: 'done' }).el, ProgressPill({ fraction: 0.1, text: 'ошибка', tone: 'fail' }).el,
      ProgressPill({ fraction: null, text: 'передано 12.0 МБ · 3.0 МБ/с', tone: 'run' }).el),
    section('LinkPill', LinkPill({ value: 'tikdl://get/blobacahq3x2kq7demolinkvalue', note: '12 файлов · 640.0 МБ' })),
    section('TransferRow — загрузки (все состояния)', dl),
    section('TransferRow — раздачи', sh),
    section('TransferRow — одноразовые раздачи: ждёт (ссылка / подключение) · закреплена · получено · срок истёк', oneTime),
    section('FileList — «Что внутри?»', FileList(peekView({ state: 'done', total: 70 * MB, free: 5 * 1024 * MB, space_error: null,
      files: Array.from({ length: 10 }, (_, i) => [`Отпуск/IMG_${1000 + i}.jpg`, 7 * MB]) })).el,
      Text(peekView({ state: 'done', total: 70 * MB, free: 20 * MB, space_error: 'не хватает места на диске (/Volumes/USB): нужно 134.0 МБ, свободно 20.0 МБ', files: [] }).status.text, 'bad')),
    section('DropOverlay', label('поверх окна, пока над ним тащат файлы (здесь — в рамке)'), (() => { const d = DropOverlay({ text: 'Отпустите — начнём раздавать' }); d.set(true); d.el.classList.add('tk-show__inline'); return d.el; })()),
    section('Field', TextArea({ placeholder: 'Вставьте ссылку…' }), TextField({ value: 'обычное поле' }), PathField({ value: '/Users/me/Downloads' }).el),
    section('Panel / Text', Panel({}, Text('Обычный текст'), Text('Приглушённый', 'dim'), Text('Ошибка', 'bad')), Panel({ accent: true }, Text('Ждёт ответа', 'strong')), Text('Пока ничего не раздаётся', 'empty')),
  );
}
