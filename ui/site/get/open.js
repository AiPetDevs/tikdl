// Turns https://…/get/#<ticket>?size=…&name=… into tikdl://get/<ticket>?… and opens the app.
// The ticket lives after "#": browsers never send it to the server that hosts this page.
(function () {
  var rest = location.hash.slice(1);
  var open = document.getElementById('open');
  var state = document.getElementById('state');
  // On GitHub Pages (<owner>.github.io/<repo>/get/) the download link points to the project's releases.
  var repo = location.pathname.split('/').filter(Boolean)[0];
  if (/\.github\.io$/.test(location.hostname) && repo) {
    document.getElementById('get').href = 'https://github.com/' + location.hostname.split('.')[0] + '/' + repo + '/releases/latest';
  }
  if (!/^(blob|once)[a-z0-9]+/i.test(rest)) {
    state.textContent = 'В ссылке нет тикета TikDL — попросите прислать её ещё раз.';
    open.classList.add('pg__hidden');
    return;
  }
  var target = 'tikdl://get/' + rest;
  open.href = target;
  window.tikdlTarget = target;
  location.href = target;
  setTimeout(function () { state.textContent = 'Если TikDL не открылся — нажмите кнопку.'; }, 1500);
})();
