// api: the only place that talks to the Rust side (Tauri invoke/listen, dialogs, Finder, clipboard).
// Without Tauri (kit showcase, ui_smoke in a plain browser) it falls back to an in-memory mock.
const T = window.__TAURI__;
// Phone: no folder pickers (Android's picker gives files only), no Finder, screen under the status bar.
export const platform = /Android/i.test(navigator.userAgent) ? 'android' : 'desktop';
document.documentElement.dataset.platform = platform;
const backend = T ? {
  invoke: T.core.invoke,
  listen: T.event.listen,
  pick: (directory, multiple = false) => T.dialog.open({ directory, multiple }),
  reveal: path => T.opener.revealItemInDir(path),
  onDragDrop: cb => T.webview.getCurrentWebview().onDragDropEvent(({ payload }) => cb(payload)),
} : (await import('./api.mock.js')).mock;

export const api = {
  inspectLink: link => backend.invoke('inspect_link', { link }),
  peekLink: (link, dest) => backend.invoke('peek_link', { link, dest }),
  defaultDownloadDir: () => backend.invoke('default_download_dir'),
  takePendingLinks: () => backend.invoke('take_pending_links'),
  listDownloads: () => backend.invoke('list_downloads'),
  startDownload: (link, dest) => backend.invoke('start_download', { link, dest }),
  pauseDownload: id => backend.invoke('pause_download', { id }),
  removeDownload: id => backend.invoke('remove_download', { id }),
  sharePath: (path, once = false) => backend.invoke('share_path', { path, once }),
  listShares: () => backend.invoke('list_shares'),
  stopShare: id => backend.invoke('stop_share', { id }),
  renewShare: id => backend.invoke('renew_share', { id }),
  pickFolder: () => backend.pick(true),
  pickPaths: async directory => { const r = await backend.pick(directory, true); return r == null ? [] : [].concat(r); },
  onDragDrop: cb => backend.onDragDrop(cb),
  reveal: path => backend.reveal(path),
  copy: text => navigator.clipboard.writeText(text),
  on: (event, cb) => backend.listen(event, ({ payload }) => cb(payload)),
};
