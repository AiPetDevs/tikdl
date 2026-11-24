// app: the window. Tabs on top (the app name is in the title bar, not repeated here), one screen at a time.
// #/kit opens the kit showcase instead.
import { h, show } from './kit/dom.js';
import { Page, Text } from './kit/Layout.js';
import { Tabs } from './kit/Tabs.js';

if (location.hash === '#/kit') {
  const { Showcase } = await import('./kit/showcase.js');
  document.body.append(Showcase());
} else {
  const st = await import('./state.js');
  const { GetScreen } = await import('./screens/get.js');
  const { ShareScreen } = await import('./screens/share.js');
  const net = Text('', 'bad'); show(net, false);
  const screens = { get: GetScreen(), share: ShareScreen() };
  const select = id => { for (const [k, el] of Object.entries(screens)) show(el, k === id); if (id === 'share') st.refreshShares(); };
  const tabs = Tabs({ items: [{ id: 'get', text: 'Скачать' }, { id: 'share', text: 'Раздать' }], value: 'get', onChange: select });
  st.on('confirm', c => { if (c) { tabs.select('get'); select('get'); } });
  st.on('net', () => { net.textContent = st.get().netError; show(net, !!st.get().netError); });
  select('get');
  document.body.append(Page(net, tabs.el, h('div', {}, screens.get, screens.share)));
  await st.boot();
}
