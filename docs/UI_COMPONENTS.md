# Реестр компонентов интерфейса TikDL

Правила: ui-architect. Слои: `ui/tokens.css` → `ui/kit/*` → `ui/screens/*`; данные: `ui/api.js` → `ui/state.js` → экраны.
Витрина: `ui/index.html#/kit` (все состояния). Перед новым элементом найти готовый здесь; вариант — параметр, не копия.

| Компонент | Файл | Что это | Параметры / состояния |
|---|---|---|---|
| h, show | `kit/dom.js` | единственный построитель элементов кита; скрыть/показать классом | — |
| Icon | `kit/icons.js` | значки 16×16 цветом текста | pause play x reveal copy check down up |
| Button | `kit/Button.js` | текстовая кнопка | variant primary/plain, wide |
| IconButton | `kit/IconButton.js` | действие значком на строке | arm (разрушительное: второе нажатие), doneIcon (copy → check) |
| StatusChip | `kit/StatusChip.js` | состояние одним словом справа от имени | tone dim/warn/ok/bad, hint |
| ProgressPill, LinkPill | `kit/ProgressPill.js` | капсула с данными внутри; неизвестное — «—», не 0 % | tone idle/run/done/fail; LinkPill: value + note |
| TransferRow, RowList | `kit/TransferRow.js` | строка передачи: имя + ярлык; капсула + значки на одной строке; список с линиями 1 px | kind down/up, chip, body, actions |
| Tabs | `kit/Tabs.js` | переключатель разделов | items, value, onChange |
| TextField, TextArea, PathField | `kit/Field.js` | поля ввода; путь только для чтения с выбором папки | readonly, placeholder |
| Page, Stack, Line, Panel, Text | `kit/Layout.js` | раскладка и текст | Panel accent; Text dim/bad/strong/empty |
| Showcase | `kit/showcase.js` | витрина всех компонентов во всех состояниях | `#/kit` |

Экраны: `screens/get.js` (GetScreen: подтверждение входящей ссылки, форма, загрузки), `screens/share.js` (ShareScreen: выбор, раздачи).
Данные: `state.js` (downloadView, shareView, preparingView — что показывает строка; действия), `api.js` (Tauri; в браузере — `api.mock.js`).
