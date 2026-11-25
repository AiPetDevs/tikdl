// Screen "Раздать": pick a file or folder, list of shares (link and actions on the same line).
import { show } from '../kit/dom.js';
import { Panel, Line, Text, Stack } from '../kit/Layout.js';
import { Button } from '../kit/Button.js';
import { Switch } from '../kit/Switch.js';
import { IconButton } from '../kit/IconButton.js';
import { ProgressPill, LinkPill } from '../kit/ProgressPill.js';
import { TransferRow, RowList } from '../kit/TransferRow.js';
import * as st from '../state.js';

const ACTIONS = {
  copy: { icon: 'copy', help: 'Скопировать ссылку', doneIcon: 'check' },
  stop: { icon: 'x', help: 'Остановить раздачу', arm: 'Нажмите ещё раз: ссылка перестанет работать' },
  renew: { icon: 'renew', help: 'Новая ссылка на те же файлы — прежняя перестанет работать', arm: 'Нажмите ещё раз: прежняя ссылка перестанет работать' },
  drop: { icon: 'x', help: 'Убрать из списка' },
};

function shareRow(sh) {
  const v = st.shareView(sh);
  const body = v.link ? LinkPill({ value: v.link, note: v.note }) : ProgressPill(v.pill).el;
  const actions = v.actions.map(k => IconButton({ ...ACTIONS[k], onClick: () => st.shareActions[k](sh.id) }));
  return TransferRow({ kind: 'up', title: v.title, hint: v.hint, chip: v.chip, body, actions }).el;
}

export function ShareScreen() {
  const status = Text('', 'bad');
  const empty = Text('Пока ничего не раздаётся', 'empty');
  const pick = Panel({},
    Line(Button({ text: 'Раздать файлы…', variant: 'primary', wide: true, id: 'shFile', onClick: () => st.share(false) }),
      Button({ text: 'Раздать папку…', variant: 'primary', wide: true, id: 'shDir', onClick: () => st.share(true) })),
    Switch({ label: 'Одноразовая ссылка', id: 'shOnce', onChange: st.setOnce,
      hint: 'Для одного устройства: скачать сможет только то, что откроет ссылку первым; после получения ссылка сгорает. В ссылке нет имени, размера и постоянного кода вашего компьютера' }).el,
    Text('Или перетащите файлы и папки в окно. Окно можно закрыть: TikDL останется в строке меню и продолжит раздавать.', 'dim'), status);
  const list = RowList();
  st.on('shares', () => {
    const s = st.get();
    const rows = s.shares.map(shareRow);
    if (s.preparing) {
      const v = st.preparingView(s.preparing);
      rows.unshift(TransferRow({ kind: 'up', title: v.title, hint: v.hint, chip: v.chip, body: ProgressPill(v.pill).el }).el);
    }
    list.replaceChildren(...rows);
    show(empty, rows.length === 0);
    const err = st.shareErrorView();
    status.textContent = err; show(status, !!err);
  });
  show(status, false);
  return Stack(pick, list, empty);
}
