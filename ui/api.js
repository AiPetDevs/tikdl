// api: the only place that talks to the Rust side (Tauri invoke/listen, dialogs, Finder, clipboard).
// Without Tauri (kit showcase, ui_smoke in a plain browser) it falls back to an in-memory mock.
const T = window.__TAURI__;
const backend = T ? {
  invoke: T.core.invoke,
  listen: T.event.listen,
  pick: directory => T.dialog.open({ directory }),
  reveal: path => T.opener.revealItemInDir(path),
} : (await import('./api.mock.js')).mock;

export const api = {
  inspectLink: link => backend.invoke('inspect_link', { link }),
  defaultDownloadDir: () => backend.invoke('default_download_dir'),
  takePendingLinks: () => backend.invoke('take_pending_links'),
  listDownloads: () => backend.invoke('list_downloads'),
  startDownload: (link, dest) => backend.invoke('start_download', { link, dest }),
  pauseDownload: id => backend.invoke('pause_download', { id }),
  removeDownload: id => backend.invoke('remove_download', { id }),
  sharePath: path => backend.invoke('share_path', { path }),
  listShares: () => backend.invoke('list_shares'),
  stopShare: root => backend.invoke('stop_share', { root }),
  pickFolder: () => backend.pick(true),
  pickPath: directory => backend.pick(directory),
  reveal: path => backend.reveal(path),
  copy: text => navigator.clipboard.writeText(text),
  on: (event, cb) => backend.listen(event, ({ payload }) => cb(payload)),
};
