#!/usr/bin/env node
// ui_check.mjs — interface cleanliness gate (skill ui-architect). Node >= 18, no dependencies.
//   node scripts/ui_check.mjs                   check; exit 1 on new violations
//   node scripts/ui_check.mjs --self-test       every rule must catch its bad sample
//   node scripts/ui_check.mjs --init            first setup: write ui-check.json and the baseline (today's debt)
//   node scripts/ui_check.mjs --write-baseline  record the debt after paying part of it (the debt may only shrink)
// Settings: ui-check.json in the project root. A project with interface files and no ui-check.json is red: the gate is
// not optional. Changing ui-check.json or the baseline is the owner's decision (with agent-skills-kit: its menu).
// Do not weaken the script: a rule change is a line in the report.
import fs from 'node:fs';
import path from 'node:path';
import crypto from 'node:crypto';
import { execFileSync, spawnSync } from 'node:child_process';
import { fileURLToPath } from 'node:url';
import os from 'node:os';
import process from 'node:process';   // explicit: project linters without Node globals pass this file
import console from 'node:console';

const DEF = {
  roots: ['.'],
  ext: ['.css', '.scss', '.html', '.htm', '.js', '.jsx', '.ts', '.tsx', '.mjs', '.vue', '.svelte'],
  exclude: ['node_modules', 'dist', 'build', 'out', 'backups', '.git', '.harness', 'coverage', 'vendor', 'ui_check.mjs', 'ui_smoke.mjs'],
  tokenFiles: [],      // token files: literals allowed
  componentDirs: [],   // UI kit folders: raw controls allowed (in a single file: the ui-kit:start ... ui-kit:end zone)
  rawControls: ['button', 'input', 'select', 'textarea'],
  allowPx: [0, 1],
  dupMinLines: 6,
  registry: '',        // component registry; every kit file must be named in it
  baseline: 'ui-check.baseline.json',
  notUi: [],           // [{path, why}]: interface files that are not product code (mockups, reports); why >= 10 characters
};
// What counts as an interface file for the coverage check (server .ts/.js are not interface).
const UI_EXT = ['.css', '.scss', '.sass', '.less', '.html', '.htm', '.jsx', '.tsx', '.vue', '.svelte'];
// Never product interface: generated or third-party, and the template's own service folders.
const NOT_PRODUCT = ['node_modules', 'dist', 'build', 'out', 'backups', '.git', '.harness', 'coverage', 'vendor', 'shots',
  'test-results', 'playwright-report', 'bible', 'records', 'UPDATE', 'REVERSE_ENGINEERING'];
// --init puts these top folders into notUi (the owner sees the list); everything else with interface files is a root.
const INIT_NOT_UI = /^(mockups?|design|docs|research|reports?|prototypes?|examples?|samples?|fixtures|e2e|tests?)$/i;
// Rules added after the first baseline format: an old baseline does not know their debt (see --write-baseline).
const NEW_IN = { 'raw-interactive': 2 };
const BASELINE_V = 2;
const HINT = 'Rules: skill ui-architect (layers, kit, showcase). ui-check.json and the baseline change only by the owner.';
const RULES = ['literal-color', 'literal-size', 'literal-z-index', 'important', 'inline-style', 'raw-control',
  'raw-interactive', 'unknown-token', 'duplicate-selector', 'duplicate-block', 'registry'];

const ALLOW = /ui-check-allow:\s*(.{10,}?)\s*(\*\/\}?|-->)?\s*$/;   // an exception only with a reason of >= 10 characters
const COLOR = /(#[0-9a-fA-F]{3,8}\b|\b(?:rgba?|hsla?|oklch|oklab|lab|lch|hwb)\(|:\s*(?:white|black|red|green|blue|gray|grey|orange|yellow|purple)\b)/g;
const SELECTOR_CTX = /(querySelector(All)?|getElementById|closest|matches|href\s*=|location\.hash|url)\s*\(?\s*['"`]?[^'"`]*$/;
const PX = /(?<![\w$.-])(\d+(?:\.\d+)?)px\b/g;
const VAR_USE = /var\(\s*(--[\w-]+)/g, VAR_DEF = /(--[\w-]+)\s*:/g;
const TOKEN_DEFS = /--[\w-]+\s*:[^;}]*/g;
// A clickable div/span/li, or one dressed as a button: no keyboard, no focus, no role. The kit Button instead.
const RAW_INTERACTIVE = /<(div|span|li|td)\b[^>]*?\s[oO]n(?:[cC]lick|[mM]ouse(?:[dD]own|[uU]p)|[pP]ointer(?:[dD]own|[uU]p))\s*=|<(div|span|li)\b[^>]*?\sclass(?:Name)?\s*=\s*["'{`][^"'}`>]*\b(?:btn|button)\b/;

const norm = s => s.trim()
  .replace(/\$\{[^}]*\}/g, 'V')                         // interpolations
  .replace(/(['"`])(?:\\.|(?!\1).)*\1/g, 'S')            // strings
  .replace(/>([^<>{}]+)</g, '>T<')                       // text between tags
  .replace(/\{[^{}]*\}(?=\s*<\/)/g, 'V')                 // {expression} in JSX
  .replace(/\b\d+(\.\d+)?\b/g, 'N').replace(/\s+/g, ' ');

// ---------- core: files = [{rel, text}] -> {issues, allows} ----------
export function scan(files, C) {
  const inList = (r, list) => list.some(d => r === d || r.startsWith(d.replace(/\/$/, '') + '/'));
  const issues = [], allows = [], defined = new Set(), used = [];
  const add = (rule, file, i, text, msg) => issues.push({ rule, file, line: i + 1, text: String(text).trim().slice(0, 160), msg });
  for (const f of files) for (const m of f.text.matchAll(VAR_DEF)) defined.add(m[1]);

  for (const f of files) {
    const r = f.rel, lines = f.text.split(/\r?\n/);
    const isToken = inList(r, C.tokenFiles), isKit = inList(r, C.componentDirs);
    const isCssFile = /\.(css|scss)$/.test(r);
    let kitZone = false, inStyle = isCssFile, depth = 0, atDepth = [];
    const selectors = new Map();
    lines.forEach((ln, i) => {
      if (/ui-kit:start/.test(ln)) kitZone = true;
      if (/ui-kit:end/.test(ln)) kitZone = false;
      if (!isCssFile && /<style\b/i.test(ln)) { inStyle = true; depth = 0; atDepth = []; }
      for (const m of ln.matchAll(VAR_USE)) used.push({ name: m[1], file: r, i, ln });
      const al = ALLOW.exec(ln) || (i > 0 && ALLOW.exec(lines[i - 1]));
      const hit = (rule, msg) => al ? allows.push({ rule, file: r, line: i + 1, why: al[1].trim() }) : add(rule, r, i, ln, msg);
      const t = ln.trim();
      const comment = t.startsWith('//') || t.startsWith('*') || t.startsWith('/*') || t.startsWith('<!--');

      // CSS structure: one selector - one definition (outside @media/@container)
      if (inStyle) {
        const css = ln.replace(/\/\*.*?\*\//g, '');
        for (const part of css.split(/(?<=[{}])/)) {
          const open = part.indexOf('{');
          if (open >= 0) {
            const sel = part.slice(0, open).trim().replace(/\s+/g, ' ');
            if (sel && !sel.startsWith('@') && !atDepth.includes(true) && depth === 0 && sel !== ':root' && !/^\[data-theme/.test(sel)) {
              if (selectors.has(sel)) hit('duplicate-selector', `selector "${sel}" already defined at line ${selectors.get(sel)} - one definition per class`);
              else selectors.set(sel, i + 1);
            }
            atDepth.push(sel.startsWith('@')); depth++;
          }
          if (part.includes('}')) { depth = Math.max(0, depth - 1); atDepth.pop(); }
        }
        if (/!important/.test(css) && !comment) hit('important', '!important - a sign of a style war; fix specificity/order');
      }
      if (!isCssFile && /<\/style>/i.test(ln)) inStyle = false;
      if (comment) return;

      const body = ln.replace(TOKEN_DEFS, '');           // token definitions are not checked
      if (!isToken) {
        for (const m of body.matchAll(COLOR)) {
          if (SELECTOR_CTX.test(body.slice(0, m.index))) continue;
          hit('literal-color', `color "${m[0].replace(/^:\s*/, '')}" is not a token - use var(--...)`); break;
        }
        if (!/^\s*@(media|container)/.test(ln)) for (const m of body.matchAll(PX)) {
          if (C.allowPx.includes(Number(m[1]))) continue;
          hit('literal-size', `size ${m[0]} is not a token - use var(--space-...)/var(--size-...)`); break;
        }
        if (/z-?index['"]?\s*[:=]\s*['"]?-?\d{2,}/i.test(body)) hit('literal-z-index', 'numeric z-index - add a layer scale to the tokens');
      }
      const sHtml = /\bstyle\s*=\s*"([^"]*)"/.exec(ln) || /\bstyle\s*=\s*'([^']*)'/.exec(ln);
      if (sHtml && sHtml[1].trim() && !/\$\{|\{\{/.test(sHtml[1])) hit('inline-style', 'static inline style - move it into a component class');
      const sJsx = /\bstyle\s*=\s*\{\{([^}]*)\}\}/.exec(ln);
      if (sJsx && /(^|,)\s*[\w$]+\s*:\s*(['"][^'"]*['"]|\d+(\.\d+)?)\s*(,|$)/.test(sJsx[1].trim())) hit('inline-style', 'literal in style={{...}} - move it into a class/token');
      if (C.rawControls.length && !isKit && !kitZone) {
        // case-sensitive: <Button> is the kit component, <button> is the raw element (2026-11-19, Chat_light)
        const m = new RegExp(`<(${C.rawControls.join('|')})\\b`).exec(ln);
        if (m) hit('raw-control', `raw <${m[1]}> outside the UI kit - use a kit component`);
      }
      if (!isKit && !kitZone) {
        const m = RAW_INTERACTIVE.exec(ln);
        if (m) hit('raw-interactive', `clickable <${m[1] || m[2]}> - use the kit Button (keyboard, focus, role)`);
      }
    });
  }
  for (const u of used) if (!defined.has(u.name)) add('unknown-token', u.file, u.i, u.ln, `token ${u.name} is not defined anywhere`);

  // duplicates: windows of N meaningful lines (strings, numbers, text, interpolations normalized - catches copies with other labels)
  const seen = new Map();
  for (const f of files) {
    const sig = [];
    f.text.split(/\r?\n/).forEach((ln, i) => {
      const n = norm(ln);
      if (n.length < 4 || /^[{}()[\];,<>/ ]*$/.test(n) || /^(\/\/|\*|\/\*|import |export \{)/.test(n)) return;
      sig.push({ n, i });
    });
    for (let k = 0; k + C.dupMinLines <= sig.length; k++) {
      const w = sig.slice(k, k + C.dupMinLines);
      if (new Set(w.map(x => x.n)).size < Math.ceil(C.dupMinLines / 2)) continue;  // data rows are not a duplicate
      const h = crypto.createHash('sha1').update(w.map(x => x.n).join('\n')).digest('hex');
      if (!seen.has(h)) seen.set(h, []);
      seen.get(h).push({ file: f.rel, from: w[0].i });
    }
  }
  const reported = new Set();
  for (const [, locs] of seen) {
    const uniq = locs.filter((l, j) => locs.findIndex(o => o.file === l.file && Math.abs(o.from - l.from) < C.dupMinLines) === j);
    if (uniq.length < 2) continue;
    const tag = uniq.map(l => `${l.file}#${Math.floor(l.from / (C.dupMinLines * 3))}`).join('|');
    if (reported.has(tag)) continue; reported.add(tag);
    const where = uniq.map(l => `${l.file}:${l.from + 1}`).join(', ');
    issues.push({ rule: 'duplicate-block', file: uniq[0].file, line: uniq[0].from + 1, text: where,
      msg: `${uniq.length} copies of one block (${where}) - extract a shared component/function` });
  }
  if (C.registry && C.componentDirs.length) {
    const reg = files.find(f => f.rel === C.registry)?.text ?? C.__registryText ?? '';
    if (!reg) issues.push({ rule: 'registry', file: C.registry, line: 1, text: '', msg: 'component registry is missing' });
    for (const f of files) {
      if (!inList(f.rel, C.componentDirs) || /\.(css|scss)$/.test(f.rel) || /(^|\/)index\.|\.test\.|\.stories\./.test(f.rel)) continue;
      const name = path.basename(f.rel).replace(/\.[^.]+$/, '');
      if (reg && !reg.includes(name)) issues.push({ rule: 'registry', file: f.rel, line: 1, text: name, msg: `component ${name} is not described in ${C.registry}` });
    }
  }
  return { issues, allows };
}

// ---------- self-test: every rule has a bad sample it must catch ----------
const BAD = {
  'literal-color': ['a.css', '.x { color: #ff0000; }'],
  'literal-size': ['a.css', '.x { padding: 12px; }'],
  'literal-z-index': ['a.css', '.x { z-index: 100; }'],
  'important': ['a.css', '.x { color: var(--c) !important; }\n:root{--c: #000}'],
  'inline-style': ['a.html', '<div style="margin-top: 6px">x</div>'],
  'raw-control': ['a.html', '<button class="b">Ok</button>'],
  'raw-interactive': ['a.tsx', '<span className="btn primary" onClick={go}>Run</span>'],
  'unknown-token': ['a.css', '.x { color: var(--acent); }'],
  'duplicate-selector': ['a.css', '.panel { color: var(--c); }\n.panel { padding: var(--s); }\n:root{--c:#000;--s:8px}'],
  'duplicate-block': ['a.js', [1, 2].map(k => `function render${k}(p){\n  const a = p.v${k};\n  const b = el('row');\n  b.title = "Label ${k}";\n  b.append(slider(a, 0, 10));\n  b.append(resetBtn(a));\n  panel.append(b);\n  return b;\n}`).join('\n')],
  'registry': ['ui/Slider.tsx', 'export const Slider = () => null;'],
};
const GOOD = ['a.html', '<style>\n:root { --bg: #1b1b1b; --space-2: 8px; }\n.panel { background: var(--bg); padding: var(--space-2); }\n@media (max-width: 900px) { .panel { padding: 0; } }\n</style>\n<script>\nconst el = document.querySelector("#add");\nconst m = `<div style="transform: translate(${x}px, ${y}px)"></div>`;\n/* ui-kit:start */\nconst Kit = { btn: (t) => `<button class="btn">${t}</button>` };\n/* ui-kit:end */\nnode.style.left = x + "px";\n</script>'];
const GOOD_TSX = ['Run.tsx', 'export const Run = ({ go }) => <Button variant="primary" onClick={go}>Run</Button>;\nexport const F = () => <Input value="" />;'];
function selfTest() {
  let ok = true;
  for (const rule of RULES) {
    const [rel, text] = BAD[rule];
    const C = { ...DEF, componentDirs: rule === 'registry' ? ['ui'] : [], registry: rule === 'registry' ? 'docs/UI.md' : '', __registryText: '# kit\n- Button' };
    const got = scan([{ rel, text }], C).issues.some(x => x.rule === rule);
    console.log(`${got ? 'PASS' : 'FAIL'}  ${rule} catches its bad sample`); ok &&= got;
  }
  const clean = [...scan([{ rel: GOOD[0], text: GOOD[1] }], DEF).issues, ...scan([{ rel: GOOD_TSX[0], text: GOOD_TSX[1] }], DEF).issues];
  console.log(`${clean.length ? 'FAIL' : 'PASS'}  clean sample has no violations${clean.length ? ': ' + clean.map(x => x.rule + ':' + x.line).join(', ') : ''}`);
  ok &&= !clean.length;
  ok = projectTests() && ok;
  process.exit(ok ? 0 : 1);
}

// The gate must not be switched off by its own settings (2026-11-19, Chat_light: no ui-check.json - nothing ran;
// a typo in roots - 0 files - green; delete and rewrite the baseline - green).
function projectTests() {
  if (spawnSync('git', ['--version']).status !== 0) { console.log('SKIP  project cases (no git)'); return true; }
  const me = fileURLToPath(import.meta.url);
  let ok = true;
  const made = [];
  const project = (files) => {
    const d = fs.mkdtempSync(path.join(os.tmpdir(), 'uicheck-')); made.push(d);
    for (const [f, t] of Object.entries(files)) { fs.mkdirSync(path.dirname(path.join(d, f)), { recursive: true }); fs.writeFileSync(path.join(d, f), t); }
    const git = (...a) => spawnSync('git', ['-c', 'user.name=t', '-c', 'user.email=t@t', ...a], { cwd: d });
    git('init', '-q'); git('add', '-A'); git('commit', '-qm', 'base');
    return { d, git, write: (f, t) => { fs.mkdirSync(path.dirname(path.join(d, f)), { recursive: true }); fs.writeFileSync(path.join(d, f), t); },
      run: (...a) => spawnSync(process.execPath, [me, ...a], { cwd: d, encoding: 'utf8' }).status };
  };
  const expect = (name, got, want) => { const pass = got === want; console.log(`${pass ? 'PASS' : 'FAIL'}  ${name} (exit ${got})`); ok &&= pass; };
  const app = { 'web/src/App.tsx': 'export const App = () => <main className="app">Hi</main>;\n', 'web/src/app.css': '.app { color: #123456; }\n' };
  const clean = { ...app, 'web/src/app.css': '.app { color: var(--c); }\n:root { --c: #000; }\n' };

  let p = project(app);
  expect('interface files without ui-check.json are red', p.run(), 1);
  expect('--init sets the gate up', p.run('--init'), 0);
  expect('after --init the old debt does not block', p.run(), 0);
  p.write('web/src/b.css', '.b { color: #abcdef; }\n');
  expect('a new violation is red', p.run(), 1);

  p = project({ ...clean, 'ui-check.json': JSON.stringify({ roots: ['web/source'] }) });
  expect('roots that match no files are red', p.run(), 1);

  p = project({ 'mockup/m.html': '<p>m</p>\n' }); p.run('--init');
  expect('only mockups: green until product interface appears', p.run(), 0);
  p.write('web/src/app.css', '.app { color: var(--c); }\n:root { --c: #000; }\n');
  expect('then the first product interface file is red', p.run(), 1);

  p = project({ ...clean, 'admin/x.css': '.x { color: var(--c); }\n:root { --c: #000; }\n', 'ui-check.json': JSON.stringify({ roots: ['web'] }) });
  expect('interface files outside roots are red', p.run(), 1);
  p.write('ui-check.json', JSON.stringify({ roots: ['web', 'admin'] }));
  expect('the same project with full roots is green', p.run(), 0);

  p = project({ ...app, 'mockup/m.html': '<p>m</p>\n', 'ui-check.json': JSON.stringify({ roots: ['web'], notUi: [{ path: 'mockup', why: 'approved picture, not code' }] }) });
  p.run('--write-baseline');
  expect('notUi with a reason covers a folder', p.run(), 0);

  p = project(app); p.run('--init'); p.git('add', '-A'); p.git('commit', '-qm', 'gate');
  fs.rmSync(path.join(p.d, 'ui-check.baseline.json'));
  p.write('web/src/b.css', '.b { color: #abcdef; }\n');
  expect('a committed baseline cannot be deleted and rewritten', p.run('--write-baseline'), 1);

  p = project(app); p.run('--init');
  p.write('ui-check.json', JSON.stringify({ ...JSON.parse(fs.readFileSync(path.join(p.d, 'ui-check.json'), 'utf8')), tokenFiles: ['web/src'] }));
  expect('settings changed after the baseline are red', p.run(), 1);
  expect('and the baseline is not rewritten over them', p.run('--write-baseline'), 1);

  p = project(app); p.run('--init');
  p.write('web/src/app.css', '.app { color: #654321; }\n');
  expect('a changed value in an old debt line is a new violation', p.run(), 1);
  for (const d of made) fs.rmSync(d, { recursive: true, force: true });
  return ok;
}

// ---------- project ----------
const sha = s => crypto.createHash('sha1').update(s).digest('hex').slice(0, 16);
const under = (r, d) => d === '.' || r === d || r.startsWith(d.replace(/\/$/, '') + '/');

function listUi(ROOT) {
  let tracked = null;
  try { tracked = execFileSync('git', ['ls-files', '-z', '--cached', '--others', '--exclude-standard'], { cwd: ROOT, encoding: 'utf8', stdio: ['ignore', 'pipe', 'ignore'] }).split('\0').filter(Boolean); }
  catch { tracked = null; }
  if (!tracked) {
    tracked = [];
    const walk = d => { for (const e of fs.readdirSync(path.join(ROOT, d), { withFileTypes: true })) {
      const r = d ? `${d}/${e.name}` : e.name;
      if (NOT_PRODUCT.includes(e.name)) continue;
      if (e.isDirectory()) walk(r); else tracked.push(r);
    } };
    walk('');
  }
  return tracked.filter(r => UI_EXT.includes(path.extname(r).toLowerCase()) && !r.split('/').some(seg => NOT_PRODUCT.includes(seg)));
}

function loadConfig(ROOT) {
  const cfgPath = path.join(ROOT, 'ui-check.json');
  if (!fs.existsSync(cfgPath)) return null;
  const own = JSON.parse(fs.readFileSync(cfgPath, 'utf8'));
  const C = { ...DEF, ...own };
  // what decides which code is checked; the baseline remembers it
  C.__print = sha(JSON.stringify(['roots', 'ext', 'exclude', 'tokenFiles', 'componentDirs', 'rawControls', 'allowPx', 'notUi', 'registry']
    .map(k => [k, C[k]])));
  return C;
}

function collect(ROOT, C) {
  const rel = p => path.relative(ROOT, p).split(path.sep).join('/');
  const excluded = r => C.exclude.some(e => r === e || r.startsWith(e + '/') || r.split('/').includes(e));
  const files = [];
  const walk = d => { for (const e of fs.readdirSync(d, { withFileTypes: true })) {
    const p = path.join(d, e.name), r = rel(p);
    if (excluded(r)) continue;
    if (e.isDirectory()) walk(p); else if (C.ext.includes(path.extname(e.name).toLowerCase())) files.push({ rel: r, text: fs.readFileSync(p, 'utf8') });
  } };
  for (const r of C.roots) { const p = path.join(ROOT, r); if (fs.existsSync(p)) fs.statSync(p).isDirectory() ? walk(p) : files.push({ rel: rel(p), text: fs.readFileSync(p, 'utf8') }); }
  if (C.registry && fs.existsSync(path.join(ROOT, C.registry))) C.__registryText = fs.readFileSync(path.join(ROOT, C.registry), 'utf8');
  return files;
}

// v2 keys keep the numbers of literal rules: a changed value in an old debt line is a new violation
const keyV = (x, v) => sha(`${x.rule}|${x.file}|${v >= 2 && x.rule.startsWith('literal-') ? x.text.trim() : norm(x.text)}`);

function writeBaseline(ROOT, C, issues, allows, files, { force, init }) {
  const blP = path.join(ROOT, C.baseline);
  const base = fs.existsSync(blP) ? JSON.parse(fs.readFileSync(blP, 'utf8')) : null;
  if (!base && !init && !force && spawnSync('git', ['cat-file', '-e', `HEAD:${C.baseline}`], { cwd: ROOT }).status === 0) {
    console.error(`Refused: ${C.baseline} is in the last saved version but deleted here. Restore it (git checkout -- ${C.baseline}).`);
    return 1;
  }
  if (base && !force) {
    if (base.cfg && base.cfg !== C.__print) {
      console.error('Refused: ui-check.json changed after the baseline was written. What the gate checks is the owner\'s decision: ' +
        'propose ui-check.json and the baseline together (agent-skills-kit: docs/harness/proposed/, menu item 6).');
      return 1;
    }
    const v = base.v || 1, old = issues.filter(x => (NEW_IN[x.rule] || 1) <= v);
    if (old.length > base.keys.length) { console.error(`Refused: debt grew ${base.keys.length} -> ${old.length}. The debt may only shrink.`); return 1; }
    if (allows.length > (base.allows ?? 0)) { console.error(`Refused: exceptions ${base.allows ?? 0} -> ${allows.length}. A new exception only by the owner.`); return 1; }
  }
  const keys = issues.map(x => keyV(x, BASELINE_V));
  fs.writeFileSync(blP, JSON.stringify({ v: BASELINE_V, written: new Date().toISOString(), cfg: C.__print, files: files.length, allows: allows.length, keys }, null, 1) + '\n');
  console.log(`Baseline: debt ${keys.length}, exceptions ${allows.length}, files ${files.length} -> ${C.baseline}`);
  return 0;
}

function init(ROOT) {
  const cfgPath = path.join(ROOT, 'ui-check.json');
  if (fs.existsSync(cfgPath)) { console.log('ui-check.json already exists: nothing to set up.'); return 0; }
  const ui = listUi(ROOT);
  if (!ui.length) { console.log('No interface files: nothing to set up.'); return 0; }
  const top = r => r.includes('/') ? r.split('/')[0] : r;
  const notUi = [...new Set(ui.map(top).filter(t => ui.some(r => r.includes('/') && top(r) === t) && INIT_NOT_UI.test(t)))]
    .map(t => ({ path: t, why: 'mockups, documents and tests are not product code' }));
  const product = ui.filter(r => !notUi.some(n => under(r, n.path)));
  const roots = [...new Set(product.map(top))].sort();
  const dirs = [...new Set(product.map(r => path.posix.dirname(r)))];
  const componentDirs = [...new Set(dirs.map(d => { const m = /^(.*?(?:^|\/)(?:kit|ui|components))(\/|$)/.exec(d); return m ? m[1] : null; }).filter(Boolean))].sort();
  const tokenFiles = product.filter(r => /\.(css|scss|sass|less)$/.test(r) && /token|theme|variables|vars/i.test(path.posix.basename(r))).sort();
  const cfg = { roots, notUi, tokenFiles, componentDirs, registry: componentDirs.length ? 'docs/UI_COMPONENTS.md' : '', smoke: [] };
  fs.writeFileSync(cfgPath, JSON.stringify(cfg, null, 1) + '\n');
  console.log(`ui-check.json written: roots ${roots.join(', ')}` + (notUi.length ? `; not product code: ${notUi.map(n => n.path).join(', ')}` : '') +
    `; tokens: ${tokenFiles.join(', ') || 'none found'}; kit: ${componentDirs.join(', ') || 'none yet'}.`);
  const C = loadConfig(ROOT), files = collect(ROOT, C), { issues, allows } = scan(files, C);
  const byRule = {}; for (const x of issues) byRule[x.rule] = (byRule[x.rule] || 0) + 1;
  console.log('Debt by rule: ' + (Object.entries(byRule).map(([r, n]) => `${r} ${n}`).join(', ') || 'none'));
  const code = writeBaseline(ROOT, C, issues, allows, files, { init: true });
  console.log('Next: the project check command must run `node scripts/ui_check.mjs`; list the pages and the kit showcase in "smoke". ' +
    'With agent-skills-kit these files are protected: propose them (docs/harness/proposed/), the owner saves them in the menu.');
  return code;
}

// ---------- run ----------
function main() {
  if (process.argv.includes('--self-test')) return selfTest();
  const ROOT = process.cwd();
  if (process.argv.includes('--init')) process.exit(init(ROOT));
  const C = loadConfig(ROOT);
  const ui = listUi(ROOT);
  if (!C) {
    if (!ui.length) { console.log('No interface files: nothing to check.'); process.exit(0); }
    console.log(`Interface files exist (${ui.length}, e.g. ${ui.slice(0, 3).join(', ')}), but the interface gate is not set up.\n` +
      `Run: node scripts/ui_check.mjs --init   (writes ui-check.json and today's debt as the baseline)\n${HINT}`);
    process.exit(1);
  }
  const bad = [];
  for (const n of C.notUi) if (!n || !n.path || !n.why || String(n.why).trim().length < 10) bad.push(`notUi entry without a reason of >= 10 characters: ${JSON.stringify(n)}`);
  const files = collect(ROOT, C);
  // roots: [] is honest only while every interface file is in notUi (the coverage check below)
  if (!files.length && C.roots.length) bad.push(`roots ${JSON.stringify(C.roots)} match no files: the gate checks nothing. Fix roots.`);
  const seen = new Set(files.map(f => f.rel));
  const outside = ui.filter(r => !seen.has(r) && !C.notUi.some(n => n && n.path && under(r, n.path)));
  if (outside.length) bad.push(`${outside.length} interface files are not checked (outside roots or excluded): ${outside.slice(0, 10).join(', ')}` +
    `${outside.length > 10 ? ' ...' : ''}. Add their folder to roots, or to notUi with a reason (owner's decision).`);

  const { issues, allows } = scan(files, C);
  if (process.argv.includes('--write-baseline')) {
    if (bad.length) { for (const b of bad) console.error('✗ ' + b); process.exit(1); }
    process.exit(writeBaseline(ROOT, C, issues, allows, files, { force: process.argv.includes('--force') }));
  }

  // baseline: old debt does not block, new violations do; debt and exceptions may only shrink
  const blP = path.join(ROOT, C.baseline);
  const base = fs.existsSync(blP) ? JSON.parse(fs.readFileSync(blP, 'utf8')) : null;
  const v = base?.v || 1;
  if (base?.cfg && base.cfg !== C.__print) bad.push('ui-check.json changed after the baseline was written. What the gate checks is the owner\'s decision: ' +
    'restore it, or propose ui-check.json and the baseline together.');
  if (base?.files && files.length < base.files * 0.8) bad.push(`checked files fell from ${base.files} to ${files.length}: settings or files went missing.`);
  if (!base && spawnSync('git', ['cat-file', '-e', `HEAD:${C.baseline}`], { cwd: ROOT }).status === 0) bad.push(`${C.baseline} is deleted: restore it.`);
  const pool = new Map(); for (const k of base?.keys || []) pool.set(k, (pool.get(k) || 0) + 1);
  const fresh = [], unknown = [];
  for (const x of issues) {
    const k = keyV(x, v);
    if (pool.get(k)) pool.set(k, pool.get(k) - 1);
    else if (base && (NEW_IN[x.rule] || 1) > v) unknown.push(x);   // a rule newer than the baseline: shown, adopted by --write-baseline
    else fresh.push(x);
  }
  const paid = [...pool.values()].reduce((a, b) => a + b, 0);
  const allowGrew = base && allows.length > (base.allows ?? 0);

  const show = (xs, title) => {
    const byRule = {}; for (const x of xs) (byRule[x.rule] ||= []).push(x);
    for (const [rule, ys] of Object.entries(byRule)) {
      console.log(`\n■ ${rule} - ${ys.length} places ${title}:`);
      for (const x of ys.slice(0, 40)) console.log(`  ${x.file}:${x.line}  ${x.msg}${x.rule === 'duplicate-block' ? '' : '\n      ' + x.text}`);
      if (ys.length > 40) console.log(`  ... ${ys.length - 40} more`);
    }
  };
  show(fresh, '(fix the whole class, at the source)');
  show(unknown, '(new rule, not in the old baseline: record it with --write-baseline, then pay it off)');
  if (allows.length) { console.log(`\nExceptions (each one goes into the owner's report):`); for (const a of allows) console.log(`  ${a.file}:${a.line} ${a.rule} — ${a.why}`); }
  if (allowGrew) console.log(`\n✗ More exceptions (${base.allows ?? 0} -> ${allows.length}). Remove them; a new exception is the owner's decision.`);
  for (const b of bad) console.log('\n✗ ' + b);
  console.log(`\nFiles: ${files.length}. New violations: ${fresh.length}. Debt: ${(base?.keys.length || 0) - paid}` +
    (paid ? ` (${paid} paid - record it: --write-baseline)` : '') + `. Exceptions: ${allows.length}.`);
  const red = fresh.length || allowGrew || bad.length;
  if (red) console.log(HINT);
  process.exit(red ? 1 : 0);
}
main();
