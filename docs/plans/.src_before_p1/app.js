const { invoke } = window.__TAURI__.core;
const { listen } = window.__TAURI__.event;
const dialog = window.__TAURI__.dialog;
const $ = id => document.getElementById(id);
const jobs = {};   // id -> {link, el}

function fmt(n){ if(n==null) return "?"; const u=["Б","КБ","МБ","ГБ","ТБ"]; let i=0; while(n>=1024&&i<4){n/=1024;i++;} return n.toFixed(i?1:0)+" "+u[i]; }
function esc(s){ return String(s).replace(/[&<>"]/g,c=>({"&":"&amp;","<":"&lt;",">":"&gt;",'"':"&quot;"}[c])); }

$("tabGet").onclick = () => { $("tabGet").classList.add("on"); $("tabShare").classList.remove("on"); $("pGet").classList.remove("hidden"); $("pShare").classList.add("hidden"); };
$("tabShare").onclick = () => { $("tabShare").classList.add("on"); $("tabGet").classList.remove("on"); $("pShare").classList.remove("hidden"); $("pGet").classList.add("hidden"); };

invoke("default_download_dir").then(d => $("dest").value = d);
$("pick").onclick = async () => { const d = await dialog.open({ directory:true }); if (d) $("dest").value = d; };

async function start(link){
  $("linkErr").textContent = "";
  let info;
  try { info = await invoke("inspect_link", { link }); } catch(e){ $("linkErr").textContent = e; return; }
  let j = jobs[info.id];
  if (!j) {
    const el = document.createElement("div");
    el.className = "card";
    el.innerHTML = `<div class="name">${esc(info.name || info.id.slice(0,16)+"…")}</div>
      <div class="bar"><div></div></div>
      <div class="muted st">Ожидание…</div>
      <div class="row"><button class="ghost act">Пауза</button><button class="ghost open hidden">Показать</button></div>`;
    $("jobs").prepend(el);
    j = jobs[info.id] = { link, el, dest: $("dest").value, state:"running", path:null };
    el.querySelector(".act").onclick = async () => {
      if (j.state === "running") await invoke("pause_download", { id: info.id });
      else { j.state = "running"; el.querySelector(".act").textContent = "Пауза"; await invoke("start_download", { link: j.link, dest: j.dest }); }
    };
    el.querySelector(".open").onclick = () => j.path && window.__TAURI__.opener.revealItemInDir(j.path);
  }
  await invoke("start_download", { link, dest: j.dest });
}

$("go").onclick = () => { const v = $("link").value.trim(); if (v) start(v); };

listen("progress", ({ payload: p }) => {
  const j = jobs[p.id]; if (!j) return;
  const el = j.el, st = el.querySelector(".st"), act = el.querySelector(".act");
  j.state = p.state;
  el.classList.toggle("done", p.state === "done");
  if (p.state === "running") {
    if (p.message) { st.textContent = p.message; return; }
    const pct = p.total ? Math.min(100, p.bytes / p.total * 100) : 0;
    el.querySelector(".bar > div").style.width = pct + "%";
    st.textContent = `${fmt(p.bytes)} из ${fmt(p.total)} · ${fmt(p.speed)}/с`;
    act.textContent = "Пауза";
  } else if (p.state === "paused") {
    st.textContent = "На паузе — скачанное сохранено"; act.textContent = "Продолжить";
  } else if (p.state === "error") {
    st.innerHTML = `<span class="err">${esc(p.message)}</span>`; act.textContent = "Повторить";
  } else if (p.state === "done") {
    el.querySelector(".bar > div").style.width = "100%";
    st.textContent = "Готово: " + p.message; j.path = p.message;
    act.classList.add("hidden"); el.querySelector(".open").classList.remove("hidden");
  }
});

// Ссылка извне (браузер, мессенджер) — только после подтверждения пользователя (аудит P0-1).
// До нажатия «Скачать» узел никуда не подключается: inspect_link лишь разбирает строку.
let pending = null;
async function confirmLink(link){
  $("tabGet").click();
  $("link").value = link;
  $("linkErr").textContent = "";
  let info;
  try { info = await invoke("inspect_link", { link }); } catch(e){ $("linkErr").textContent = e; return; }
  pending = link;
  $("cfName").textContent = info.name || (info.id.slice(0,16) + "…");
  const size = info.size != null ? fmt(info.size) + " (со слов отправителя)" : "размер неизвестен";
  $("cfMeta").textContent = `${size} · от узла ${info.from.slice(0,10)}… · в папку ${$("dest").value}`;
  $("confirm").classList.remove("hidden");
  $("cfYes").focus();
}
$("cfYes").onclick = () => { const l = pending; pending = null; $("confirm").classList.add("hidden"); if (l) start(l); };
$("cfNo").onclick = () => { pending = null; $("confirm").classList.add("hidden"); $("link").value = ""; };

listen("open-link", ({ payload }) => confirmLink(payload));

async function share(directory){
  const path = await dialog.open({ directory });
  if (!path) return;
  $("shStatus").textContent = "Подготовка (подсчёт хешей), для больших файлов это может занять время…";
  try {
    const sh = await invoke("share_path", { path });
    const link = sh.link;
    $("shStatus").textContent = "";
    const el = document.createElement("div");
    el.className = "card";
    const skipped = sh.skipped ? ` · пропущено скрытых и служебных: ${sh.skipped}` : "";
    el.innerHTML = `<div class="name">${esc(path)}</div>
      <div class="muted">Файлов: ${sh.files} · ${fmt(sh.total)}${esc(skipped)}</div>
      <textarea readonly>${esc(link)}</textarea>
      <div class="row"><button class="ghost cp">Скопировать ссылку</button></div>`;
    el.querySelector(".cp").onclick = () => { navigator.clipboard.writeText(link); el.querySelector(".cp").textContent = "Скопировано"; };
    $("shares").prepend(el);
  } catch(e) { $("shStatus").innerHTML = `<span class="err">${esc(e)}</span>`; }
}
$("shFile").onclick = () => share(false);
$("shDir").onclick = () => share(true);
