// Screen "Скачать": incoming-link confirm, link form, list of downloads. Composition of the kit + state only.
import { show } from '../kit/dom.js';
import { Panel, Line, Text, Stack } from '../kit/Layout.js';
import { Button } from '../kit/Button.js';
import { IconButton } from '../kit/IconButton.js';
import { TextArea, PathField } from '../kit/Field.js';
import { ProgressPill } from '../kit/ProgressPill.js';
import { TransferRow, RowList } from '../kit/TransferRow.js';
import * as st from '../state.js';

const ACTIONS = {
  pause: { icon: 'pause', help: 'Пауза — продолжить можно когда угодно, даже после перезапуска' },
  resume: { icon: 'play', help: 'Продолжить — докачает только недостающее' },
  remove: { icon: 'x', help: 'Отменить и убрать', arm: 'Нажмите ещё раз: загрузка отменится, недокачанное удалится' },
  reveal: { icon: 'reveal', help: 'Показать в Finder' },
  forget: { icon: 'x', help: 'Убрать из списка (файлы на диске останутся)' },
};
const buttons = (id, keys) => keys.map(k => IconButton({ ...ACTIONS[k], onClick: () => st.actions[k](id) }));

export function GetScreen() {
  // incoming link: waits for an explicit answer
  const cfName = Text('', 'strong'), cfMeta = Text('', 'dim');
  const confirm = Panel({ accent: true, role: 'alertdialog', label: 'Подтверждение' },
    Text('Пришла ссылка на скачивание. Скачать?', 'dim'), cfName, cfMeta,
    Line(Button({ text: 'Скачать', variant: 'primary', wide: true, id: 'cfYes', onClick: () => st.answerConfirm(true) }),
      Button({ text: 'Отмена', id: 'cfNo', onClick: () => st.answerConfirm(false) })));
  show(confirm, false);
  st.on('confirm', c => {
    show(confirm, !!c); if (!c) return;
    cfName.textContent = c.title; cfMeta.textContent = c.meta;
    confirm.querySelector('#cfNo').focus();   // the link brings the window forward: a stray Return must not start a download
  });

  const link = TextArea({ placeholder: 'Вставьте ссылку или тикет, который вам прислали (tikdl://…)', id: 'link' });
  const dest = PathField({ onPick: st.pickDest });
  const err = Text('', 'bad');
  const go = async () => { const v = link.value.trim(); if (v && await st.startDownload(v)) link.value = ''; };
  const form = Panel({}, link, dest.el, Line(Button({ text: 'Скачать', variant: 'primary', wide: true, id: 'go', onClick: go })), err);
  st.on('form', () => { dest.value = st.get().dest; err.textContent = st.get().linkError; show(err, !!st.get().linkError); });
  show(err, false);

  const list = RowList();
  const rows = new Map();   // id -> {row, pill, keys}
  const paint = d => {
    const v = st.downloadView(d);
    let r = rows.get(d.id);
    if (!r) {
      const pill = ProgressPill(v.pill);
      r = { pill, row: TransferRow({ kind: 'down', title: v.title, hint: v.hint, chip: v.chip, body: pill.el, actions: buttons(d.id, v.actions) }), keys: v.actions.join() };
      rows.set(d.id, r); list.prepend(r.row.el);
      return;
    }
    r.pill.set(v.pill);
    const keys = v.actions.join();
    r.row.set({ title: v.title, hint: v.hint, chip: v.chip, actions: keys === r.keys ? undefined : buttons(d.id, v.actions) });
    r.keys = keys;
  };
  st.on('download:new', paint);
  st.on('download', paint);
  st.on('download:gone', id => { rows.get(id)?.row.el.remove(); rows.delete(id); });

  return Stack(confirm, form, list);
}
