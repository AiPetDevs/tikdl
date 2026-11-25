#!/usr/bin/env node
// passport.mjs — app passport: an index of the code plus hand-written cards, checked against each other.
// Node >= 18, no dependencies. Run from the project root.
//
//   node scripts/passport.mjs init             draft passport/ for this project (never overwrites)
//   node scripts/passport.mjs build            index the code → passport/generated/*, passport/view.html; then check
//   node scripts/passport.mjs check            exit 1 if cards and code disagree
//   node scripts/passport.mjs where <q>        symbol, file, component or feature: owner, lines, callers, callees, features
//   node scripts/passport.mjs impact <symbol>  everything that depends on it, components, features, covering tests
//
// Languages: Rust, JavaScript/TypeScript (js mjs cjs jsx ts tsx), Python. Other files are tracked for coverage only.
// The index is built with a small tokenizer and regular expressions, not a compiler: calls are matched by name and marked
// approx. Good enough for a map; use the language server for proof before a big change.
// Adapters link code across a boundary the parser cannot follow: "tauri" (commands and events between Rust and JS).
import fs from 'node:fs';
import path from 'node:path';
import crypto from 'node:crypto';
import { fileURLToPath } from 'node:url';

const ROOT = process.cwd();
const HERE = path.dirname(fileURLToPath(import.meta.url));
const P = (...a) => path.join(ROOT, ...a);
const rel = p => path.relative(ROOT, p).split(path.sep).join('/');
const readJSON = p => JSON.parse(fs.readFileSync(p, 'utf8'));
const exists = p => fs.existsSync(p);
const SKIP_DIRS = ['node_modules', '.git', 'target', 'dist', 'build', 'out', '.next', '.venv', 'venv', 'env', '__pycache__',
  '.mypy_cache', '.pytest_cache', '.tox', 'vendor', 'coverage', 'shots', 'passport', '.idea', '.vscode', 'gen'];
const CODE_EXT = ['rs', 'py', 'js', 'mjs', 'cjs', 'jsx', 'ts', 'tsx', 'vue', 'svelte', 'go', 'swift', 'kt', 'java', 'c', 'cc', 'cpp', 'h', 'hpp', 'cs', 'rb', 'php', 'sh', 'html', 'css', 'scss'];

// ---------------------------------------------------------------- files
function walk(dir, skip, out = []) {
  if (!exists(dir)) return out;
  for (const e of fs.readdirSync(dir, { withFileTypes: true })) {
    const p = path.join(dir, e.name);
    if (e.isDirectory()) { if (!skip.includes(e.name) && !skip.includes(rel(p))) walk(p, skip, out); }
    else if (e.isFile()) out.push(rel(p));
  }
  return out;
}
function globRe(g) {
  let s = '';
  for (let i = 0; i < g.length; i++) {
    const c = g[i];
    if (c === '*' && g[i + 1] === '*') { s += '.*'; i++; if (g[i + 1] === '/') i++; }
    else if (c === '*') s += '[^/]*';
    else if (c === '{') { const j = g.indexOf('}', i); s += '(' + g.slice(i + 1, j).split(',').map(x => x.replace(/[.]/g, '\\.')).join('|') + ')'; i = j; }
    else s += c.replace(/[.+^$()|[\]\\?]/g, '\\$&');
  }
  return new RegExp('^' + s + '$');
}
const matchAny = (file, globs) => globs.some(g => globRe(g).test(file));
const lineOf = (src, off) => src.slice(0, off).split('\n').length;
function blockEnd(s, open) { let d = 0; for (let j = open; j < s.length; j++) { if (s[j] === '{') d++; else if (s[j] === '}' && --d === 0) return j; } return s.length; }
const blank = s => s.replace(/[^\n]/g, ' ');
const isTestFile = f => /(\.|_)(test|spec)\.[jt]sx?$|(^|\/)(test_[^/]*|[^/]*_test)\.py$|(^|\/)(__tests__|tests?)\//.test(f);

// ---------------------------------------------------------------- Rust
function stripRust(src) {
  let out = '', i = 0;
  while (i < src.length) {
    const c = src[i], d = src[i + 1];
    if (c === '/' && d === '/') { const j = src.indexOf('\n', i); const k = j < 0 ? src.length : j; out += blank(src.slice(i, k)); i = k; continue; }
    if (c === '/' && d === '*') { let depth = 1, j = i + 2; while (j < src.length && depth) { if (src.startsWith('/*', j)) { depth++; j += 2; } else if (src.startsWith('*/', j)) { depth--; j += 2; } else j++; } out += blank(src.slice(i, j)); i = j; continue; }
    const raw = /^(b?r)(#*)"/.exec(src.slice(i, i + 12));
    if (raw && !/\w/.test(src[i - 1] || '')) { const end = '"' + raw[2]; const j = src.indexOf(end, i + raw[0].length); const k = j < 0 ? src.length : j + end.length; out += '"' + blank(src.slice(i + 1, k - 1)) + '"'; i = k; continue; }
    if (c === '"') { let j = i + 1; while (j < src.length && src[j] !== '"') j += src[j] === '\\' ? 2 : 1; out += '"' + blank(src.slice(i + 1, j)) + '"'; i = j + 1; continue; }
    if (c === "'") { const m = /^'(\\.[^']*|[^\\'])'/.exec(src.slice(i, i + 12)); if (m) { out += "'" + blank(m[0].slice(1, -1)) + "'"; i += m[0].length; continue; } }
    out += c; i++;
  }
  return out;
}
function indexRust(file, src, idx) {
  const s = stripRust(src);
  const base = path.basename(file, '.rs');
  const mod = base === 'mod' ? path.basename(path.dirname(file)) : base;
  const tests = [];
  for (const m of s.matchAll(/#\[cfg\(test\)\]\s*mod\s+\w+\s*\{/g)) tests.push([m.index, blockEnd(s, m.index + m[0].length - 1)]);
  const inTest = off => tests.some(([a, b]) => off >= a && off <= b);
  const impls = [];
  for (const m of s.matchAll(/(^|\n)\s*impl(?:<[^>{]*>)?\s+(?:[\w:]+(?:<[^>{]*>)?\s+for\s+)?(\w+)[^{;]*\{/g)) {
    const open = m.index + m[0].length - 1; impls.push({ name: m[2], a: open, b: blockEnd(s, open), trait: /\sfor\s/.test(m[0]) });
  }
  const implAt = off => impls.find(x => off > x.a && off < x.b);
  for (const m of s.matchAll(/(?:^|\n)([ \t]*)((?:#\[[^\]]*\]\s*)*)(?:pub(?:\([^)]*\))?\s+)?(?:const\s+)?(?:async\s+)?(?:unsafe\s+)?(?:extern\s+"[^"]*"\s+)?fn\s+(\w+)/g)) {
    const off = m.index + m[0].lastIndexOf('fn ');
    const im = implAt(off);
    if (im?.trait) continue; // trait impls (Display::fmt …) are plumbing, not the project's API
    const open = s.indexOf('{', off), semi = s.indexOf(';', off);
    const end = open >= 0 && (semi < 0 || open < semi) ? blockEnd(s, open) : semi;
    const attrs = m[2] + (src.slice(Math.max(0, off - 200), off).match(/#\[[^\]]*\]\s*$/)?.[0] || '');
    const qual = im ? `${im.name}::${m[3]}` : m[3];
    idx.add({ file, name: m[3], id: `${file}#${qual}`, qual, kind: 'fn', lang: 'rust', mod, line: lineOf(src, off), end: lineOf(src, end),
      test: inTest(off) || /#\[(tokio::)?test/.test(attrs), command: /#\[tauri::command\]/.test(attrs), body: s.slice(open, end), owner: im?.name });
  }
  for (const m of s.matchAll(/(?:^|\n)\s*(?:pub(?:\([^)]*\))?\s+)?(struct|enum|const|static|type|trait)\s+(\w+)/g)) {
    const off = m.index + m[0].lastIndexOf(m[2]);
    if (implAt(off)) continue;
    idx.add({ file, name: m[2], id: `${file}#${m[2]}`, qual: m[2], kind: m[1], lang: 'rust', mod, line: lineOf(src, off), end: lineOf(src, off), test: inTest(off) });
  }
  const h = /generate_handler!\[([^\]]*)\]/.exec(s);
  if (h) { idx.tauri.registered.push(...h[1].split(',').map(x => x.trim().split('::').pop()).filter(Boolean)); idx.tauri.registeredAt = { file, line: lineOf(src, h.index) }; }
  for (const m of src.matchAll(/\.emit(?:_to|_all)?\(\s*(?:[^,()]+,\s*)?"([\w:/-]+)"/g)) idx.tauri.emits.push({ event: m[1], file, line: lineOf(src, m.index) });
}
const STOP = new Set(['new', 'read', 'write', 'get', 'set', 'push', 'insert', 'remove', 'list', 'len', 'clone', 'stream', 'next',
  'send', 'delete', 'export', 'load', 'store', 'parse', 'join', 'iter', 'map', 'filter', 'contains', 'lock', 'take', 'from', 'into',
  'default', 'build', 'spawn', 'run', 'show', 'hide', 'emit', 'state', 'path', 'name', 'files', 'is_file', 'is_dir', 'exists', 'add',
  'start', 'stop', 'open', 'close', 'update', 'init', 'with', 'call', 'apply', 'value', 'values', 'keys', 'items', 'append', 'extend']);
function rustCalls(idx) {
  const fns = idx.list.filter(x => x.lang === 'rust' && x.kind === 'fn');
  const mods = new Set(fns.map(f => f.mod));
  const methods = new Map();
  for (const f of fns) if (f.owner) (methods.get(f.name) || methods.set(f.name, []).get(f.name)).push(f);
  for (const f of fns) {
    if (!f.body) continue;
    const add = (t, how) => { if (t && t !== f) idx.edges.push({ from: f.id, to: t.id, kind: 'call', how, approx: true }); };
    const seen = new Set();
    for (const m of f.body.matchAll(/(\.|\b(\w+)::|\b)(\w+)\s*(?:::<[^>]*>)?\s*\(/g)) {
      const [, pre, q, name] = m; const key = pre + name; if (seen.has(key)) continue; seen.add(key);
      if (pre === '.') { const c = methods.get(name); if (c?.length === 1 && !STOP.has(name)) add(c[0], 'method'); continue; }
      if (q) {
        if (q === 'Self' && f.owner) add(fns.find(x => x.owner === f.owner && x.name === name), 'self');
        else if (q === 'super' || q === 'crate') add(fns.find(x => !x.owner && x.name === name && !x.test && (q === 'crate' || x.file !== f.file)), 'module');
        else if (mods.has(q)) add(fns.find(x => x.mod === q && !x.owner && x.name === name && !x.test), 'module');
        else add(fns.find(x => x.owner === q && x.name === name), 'type');
        continue;
      }
      add(fns.find(x => x.file === f.file && !x.owner && x.name === name && !x.test), 'local');
    }
  }
}

// ---------------------------------------------------------------- JavaScript / TypeScript
function stripJS(src) {
  let out = '', i = 0, prev = '';
  const n = src.length;
  while (i < n) {
    const c = src[i], d = src[i + 1];
    if (c === '/' && d === '/') { const j = src.indexOf('\n', i); const k = j < 0 ? n : j; out += blank(src.slice(i, k)); i = k; continue; }
    if (c === '/' && d === '*') { const j = src.indexOf('*/', i + 2); const k = j < 0 ? n : j + 2; out += blank(src.slice(i, k)); i = k; continue; }
    if (c === "'" || c === '"') { let j = i + 1; while (j < n && src[j] !== c && src[j] !== '\n') j += src[j] === '\\' ? 2 : 1; out += c + blank(src.slice(i + 1, j)) + c; i = j + 1; prev = c; continue; }
    if (c === '`') { let j = i + 1; while (j < n && src[j] !== '`') j += src[j] === '\\' ? 2 : 1; out += '`' + blank(src.slice(i + 1, j)) + '`'; i = j + 1; prev = '`'; continue; }
    if (c === '/' && (prev === '' || '(,=:[!&|?{};+-*%<>~^'.includes(prev) || /(?:return|typeof|case)\s*$/.test(out.slice(-12)))) {
      let j = i + 1, cls = false;
      while (j < n && src[j] !== '\n' && (cls || src[j] !== '/')) { if (src[j] === '\\') j++; else if (src[j] === '[') cls = true; else if (src[j] === ']') cls = false; j++; }
      j++; while (/[a-z]/.test(src[j] || '')) j++;
      out += blank(src.slice(i, j)); i = j; prev = 'r'; continue;
    }
    out += c; if (!/\s/.test(c)) prev = c; i++;
  }
  return out;
}
const JS_EXT = ['.js', '.mjs', '.cjs', '.jsx', '.ts', '.tsx'];
function resolveJS(from, spec, fileSet) {
  if (!spec.startsWith('.')) return null;
  const b = rel(path.resolve(path.dirname(P(from)), spec));
  const tries = [b, ...JS_EXT.map(e => b + e), ...JS_EXT.map(e => `${b}/index${e}`), b.replace(/\.js$/, '.ts'), b.replace(/\.js$/, '.tsx')];
  return tries.find(t => fileSet.has(t)) || null;
}
function indexJS(file, src, idx) {
  const s = stripJS(src);
  const add = (name, off, kind, end, extra = {}) => idx.add({ file, name, id: `${file}#${name}`, qual: name, kind, lang: 'js', line: lineOf(src, off), end: lineOf(src, end), ...extra });
  const endOf = (off, stop = ';') => {
    const eol = s.indexOf('\n', off), line = s.slice(off, eol < 0 ? s.length : eol);
    const arrow = line.indexOf('=>'), brace = line.indexOf('{', arrow >= 0 ? arrow : 0);
    if (brace >= 0 && (arrow >= 0 ? /^\s*$/.test(line.slice(arrow + 2, brace)) : true)) return blockEnd(s, off + brace);
    const j = s.indexOf(stop, off); return j < 0 ? s.length : j;
  };
  const testFile = isTestFile(file);
  for (const m of s.matchAll(/(?:^|\n)(export\s+)?(?:default\s+)?(?:async\s+)?function\s*\*?\s*(\w+)/g)) {
    const off = m.index + m[0].lastIndexOf(m[2]);
    let p = s.indexOf('(', off), depth = 0;
    for (; p < s.length; p++) { if (s[p] === '(') depth++; else if (s[p] === ')' && --depth === 0) break; }
    add(m[2], off, 'fn', blockEnd(s, s.indexOf('{', p)), { exported: !!m[1], test: testFile });
  }
  for (const m of s.matchAll(/(?:^|\n)(export\s+)?(?:default\s+)?class\s+(\w+)/g)) {
    const off = m.index + m[0].lastIndexOf(m[2]); const open = s.indexOf('{', off); const end = blockEnd(s, open);
    add(m[2], off, 'class', end, { exported: !!m[1], test: testFile });
    const body = s.slice(open + 1, end);
    for (const k of body.matchAll(/(?:^|\n)\s+(?:static\s+)?(?:async\s+)?(?:get\s+|set\s+)?(\w+)\s*\([^)]*\)\s*\{/g)) {
      if (['if', 'for', 'while', 'switch', 'catch', 'return'].includes(k[1])) continue;
      const ko = open + 1 + k.index + k[0].indexOf(k[1]); add(`${m[2]}.${k[1]}`, ko, 'fn', blockEnd(s, s.indexOf('{', ko)), { owner: m[2], test: testFile });
    }
  }
  for (const m of s.matchAll(/(?:^|\n)(export\s+)?(?:const|let|var)\s+(\w+)\s*(?::[^=\n]+)?=\s*(\{|\(|async\b|function\b|\w+\s*=>|\[)/g)) {
    const off = m.index + m[0].indexOf(m[2], m[0].search(/(const|let|var)/));
    if (m[3] === '{') {
      const open = s.indexOf('{', off); const end = blockEnd(s, open); const body = s.slice(open + 1, end);
      add(m[2], off, 'object', end, { exported: !!m[1], test: testFile });
      for (const k of body.matchAll(/(?:^|\n)\s{2}(?:async\s+)?(\w+)\s*(:|\()/g)) { const ko = open + 1 + k.index + k[0].indexOf(k[1]); add(`${m[2]}.${k[1]}`, ko, 'fn', Math.min(endOf(ko, '\n'), end), { exported: !!m[1], test: testFile }); }
    } else if (m[3] !== '[') add(m[2], off, 'fn', endOf(off), { exported: !!m[1], test: testFile });
  }
  // imports: alias → file (namespace, default, `const x = await import()`), name → file (named)
  const imp = { ns: {}, named: {} };
  const reg = (spec, line, kind, names) => {
    const target = resolveJS(file, spec, idx.fileSet); if (!target) return;
    idx.imports.push({ from: file, to: target, line });
    if (kind === 'ns') imp.ns[names] = target;
    if (kind === 'named') for (const x of names.split(',')) { const [a, b] = x.trim().split(/\s+as\s+/); if (a) imp.named[(b || a).trim()] = { file: target, name: a.trim() }; }
  };
  for (const m of src.matchAll(/(?:^|\n)\s*import\s+([^'";]*?)\s*from\s*['"]([^'"]+)['"]/g)) {
    const spec = m[1], line = lineOf(src, m.index);
    const ns = /\*\s+as\s+(\w+)/.exec(spec); const named = /\{([^}]*)\}/.exec(spec); const def = /^\s*(\w+)\s*(,|$)/.exec(spec);
    if (ns) reg(m[2], line, 'ns', ns[1]); if (named) reg(m[2], line, 'named', named[1]); if (def) reg(m[2], line, 'ns', def[1]);
    if (!ns && !named && !def) reg(m[2], line, 'side');
  }
  for (const m of src.matchAll(/(?:^|\n)\s*import\s*['"]([^'"]+)['"]/g)) reg(m[1], lineOf(src, m.index), 'side');
  for (const m of src.matchAll(/(?:(?:const|let|var)\s+(?:\{([^}]*)\}|(\w+))\s*=\s*)?(?:await\s+)?import\(\s*['"]([^'"]+)['"]\s*\)/g)) {
    const line = lineOf(src, m.index);
    if (m[1]) reg(m[3], line, 'named', m[1].replace(/(\w+)\s*:\s*(\w+)/g, '$1 as $2')); else if (m[2]) reg(m[3], line, 'ns', m[2]); else reg(m[3], line, 'side');
  }
  for (const m of src.matchAll(/(?:const|let|var)\s+(?:\{([^}]*)\}|(\w+))\s*=\s*require\(\s*['"]([^'"]+)['"]\s*\)/g)) {
    const line = lineOf(src, m.index); if (m[1]) reg(m[3], line, 'named', m[1].replace(/(\w+)\s*:\s*(\w+)/g, '$1 as $2')); else reg(m[3], line, 'ns', m[2]);
  }
  idx.jsMeta[file] = { s, imp };
  for (const m of src.matchAll(/\binvoke(?:<[^>]*>)?\(\s*['"](\w+)['"]/g)) idx.tauri.invokes.push({ command: m[1], file, line: lineOf(src, m.index) });
  const listenRe = new RegExp(idx.cfg.adapters?.tauri?.listen || "\\blisten(?:<[^>]*>)?\\(\\s*['\"]([\\w:/-]+)['\"]", 'g');
  for (const m of src.matchAll(listenRe)) idx.tauri.listens.push({ event: m[1], file, line: lineOf(src, m.index) });
}
function jsCalls(idx) {
  const js = idx.list.filter(x => x.lang === 'js' && x.kind === 'fn');
  const byFile = {};
  for (const f of js) (byFile[f.file] ||= []).push(f);
  const sym = (file, name) => js.find(x => x.file === file && x.name === name);
  for (const [file, { s, imp }] of Object.entries(idx.jsMeta)) {
    const own = (byFile[file] || []).slice().sort((a, b) => a.line - b.line);
    const ownerAt = n => { let o = null; for (const f of own) if (f.line <= n && f.end >= n) o = f; return o; };
    s.split('\n').forEach((ln, i) => {
      const from = ownerAt(i + 1); if (!from) return;
      for (const m of ln.matchAll(/(?<![.\w$])(\w+)(?:\.(\w+))?(?:\.(\w+))?\s*(?:\(|\?\.\()/g)) {
        const [, a, b, c] = m; let t = null;
        if (imp.ns[a] && b) t = sym(imp.ns[a], c ? `${b}.${c}` : b) || sym(imp.ns[a], b);
        else if (imp.named[a]) { const r = imp.named[a]; t = b ? sym(r.file, `${r.name}.${b}`) : sym(r.file, r.name); }
        else if (!b) t = own.find(x => x.name === a);
        else if (a === 'this' && from.owner) t = sym(file, `${from.owner}.${b}`);
        else t = sym(file, `${a}.${b}`);
        if (t && t !== from) idx.edges.push({ from: from.id, to: t.id, kind: 'call', how: 'js', approx: true });
      }
    });
  }
}

// ---------------------------------------------------------------- Python
function stripPy(src) {
  let out = '', i = 0; const n = src.length;
  while (i < n) {
    const c = src[i];
    if (c === '#') { const j = src.indexOf('\n', i); const k = j < 0 ? n : j; out += blank(src.slice(i, k)); i = k; continue; }
    const q = /^([rbfuRBFU]{0,2})('''|"""|'|")/.exec(src.slice(i, i + 5));
    if (q && (q[1] === '' || !/\w/.test(src[i - 1] || ''))) {
      const quote = q[2]; let j = i + q[0].length;
      while (j < n && !src.startsWith(quote, j) && (quote.length === 3 || src[j] !== '\n')) j += src[j] === '\\' ? 2 : 1;
      const k = Math.min(n, j + quote.length);
      out += q[0] + blank(src.slice(i + q[0].length, j)) + quote; i = k; continue;
    }
    out += c; i++;
  }
  return out;
}
function resolvePy(from, spec, fileSet, roots) {
  const dots = /^\.*/.exec(spec)[0].length; const parts = spec.slice(dots).split('.').filter(Boolean);
  const bases = dots ? [rel(path.resolve(path.dirname(P(from)), ...Array(dots - 1).fill('..')))] : ['', ...roots];
  for (const b of bases) {
    const p = [b, ...parts].filter(Boolean).join('/');
    for (const t of [`${p}.py`, `${p}/__init__.py`]) if (fileSet.has(t)) return t;
  }
  return null;
}
function indexPy(file, src, idx) {
  const s = stripPy(src); const lines = s.split('\n'); const raw = src.split('\n');
  const testFile = isTestFile(file);
  const indentOf = l => /^[ \t]*/.exec(l)[0].replace(/\t/g, '    ').length;
  const endLine = (i, ind) => { let j = i + 1; for (; j < lines.length; j++) { const l = lines[j]; if (l.trim() && indentOf(l) <= ind) break; } let e = j; while (e > i + 1 && !lines[e - 1].trim()) e--; return e; };
  const stack = []; // classes: {name, indent}
  lines.forEach((l, i) => {
    if (!l.trim()) return;
    const ind = indentOf(l);
    while (stack.length && ind <= stack[stack.length - 1].indent) stack.pop();
    let m = /^\s*(?:async\s+)?def\s+(\w+)/.exec(l);
    if (m) {
      const cls = stack.length && ind > stack[stack.length - 1].indent ? stack[stack.length - 1].name : null;
      const nested = !cls && ind > 0;
      if (!nested) {
        const end = endLine(i, ind), qual = cls ? `${cls}.${m[1]}` : m[1];
        let k = i - 1; while (k >= 0 && /^\s*@/.test(raw[k] || '') && !/^\s*@\w+\.(get|post|put|patch|delete|route|websocket)\(/.test(raw[k])) k--;
        const deco = (raw[k] || '').trim();
        idx.add({ file, name: m[1], id: `${file}#${qual}`, qual, kind: 'fn', lang: 'py', line: i + 1, end, owner: cls, test: testFile || /^test_/.test(m[1]),
          body: lines.slice(i + 1, end).join('\n'), route: /^@\w+\.(get|post|put|patch|delete|route|websocket)\(/.test(deco) ? deco : null });
      }
      return;
    }
    m = /^\s*class\s+(\w+)/.exec(l);
    if (m) { if (ind === 0 || stack.length) { idx.add({ file, name: m[1], id: `${file}#${m[1]}`, qual: m[1], kind: 'class', lang: 'py', line: i + 1, end: endLine(i, ind), test: testFile }); stack.push({ name: m[1], indent: ind }); } return; }
    m = /^([A-Za-z_]\w*)\s*(?::[^=]+)?=(?!=)/.exec(l);
    if (m && ind === 0 && /^[A-Z_][A-Z0-9_]*$/.test(m[1])) idx.add({ file, name: m[1], id: `${file}#${m[1]}`, qual: m[1], kind: 'const', lang: 'py', line: i + 1, end: i + 1, test: testFile });
  });
  const imp = { ns: {}, named: {} };
  const roots = idx.cfg.pythonRoots || ['src'];
  s.split('\n').forEach((l, i) => {
    let m = /^\s*import\s+([\w., ]+)/.exec(l);
    if (m) for (const part of m[1].split(',')) { const [mod, as] = part.trim().split(/\s+as\s+/); const t = resolvePy(file, mod, idx.fileSet, roots); if (t) { idx.imports.push({ from: file, to: t, line: i + 1 }); imp.ns[(as || mod.split('.').pop()).trim()] = t; } }
    m = /^\s*from\s+([\w.]+)\s+import\s+\(?([\w., \n]+)/.exec(l);
    if (m) {
      const t = resolvePy(file, m[1], idx.fileSet, roots);
      for (const part of m[2].split(',')) {
        const [name, as] = part.trim().split(/\s+as\s+/); if (!name || name === '(') continue;
        const sub = resolvePy(file, `${m[1]}${m[1].endsWith('.') ? '' : '.'}${name}`, idx.fileSet, roots);
        if (sub) { idx.imports.push({ from: file, to: sub, line: i + 1 }); imp.ns[(as || name).trim()] = sub; }
        else if (t) { idx.imports.push({ from: file, to: t, line: i + 1 }); imp.named[(as || name).trim()] = { file: t, name }; }
      }
    }
  });
  idx.pyMeta[file] = imp;
}
function pyCalls(idx) {
  const py = idx.list.filter(x => x.lang === 'py');
  const sym = (file, q) => py.find(x => x.file === file && x.qual === q);
  const methods = new Map();
  for (const f of py) if (f.kind === 'fn' && f.owner && !f.name.startsWith('__')) (methods.get(f.name) || methods.set(f.name, []).get(f.name)).push(f);
  for (const f of py.filter(x => x.kind === 'fn' && x.body)) {
    const imp = idx.pyMeta[f.file] || { ns: {}, named: {} };
    const add = t => { if (t && t !== f) idx.edges.push({ from: f.id, to: t.id, kind: 'call', how: 'py', approx: true }); };
    for (const m of f.body.matchAll(/(?<![.\w])(\w+)(?:\.(\w+))?\s*\(/g)) {
      const [, a, b] = m;
      if (a === 'self' || a === 'cls') { if (b && f.owner) add(sym(f.file, `${f.owner}.${b}`)); continue; }
      if (b) {
        if (imp.ns[a]) add(sym(imp.ns[a], b) || sym(imp.ns[a], a));
        else if (imp.named[a]) add(sym(imp.named[a].file, `${imp.named[a].name}.${b}`));
        else { const c = methods.get(b); if (c?.length === 1 && !STOP.has(b)) add(c[0]); } // obj.method(): only a unique, non-generic name
        continue;
      }
      if (imp.named[a]) add(sym(imp.named[a].file, imp.named[a].name) || sym(imp.named[a].file, `${imp.named[a].name}.__init__`));
      else add(sym(f.file, a) || sym(f.file, `${a}.__init__`));
    }
  }
}

// ---------------------------------------------------------------- passport model
function loadModel() {
  if (!exists(P('passport/passport.json'))) { console.error('нет passport/passport.json — сначала: node scripts/passport.mjs init'); process.exit(2); }
  const pass = readJSON(P('passport/passport.json'));
  const dir = d => exists(P('passport', d)) ? fs.readdirSync(P('passport', d)).filter(f => f.endsWith('.json') && !f.startsWith('_')) : [];
  const comps = dir('components').map(f => ({ ...readJSON(P('passport/components', f)), _file: `passport/components/${f}` }));
  const feats = dir('features').map(f => ({ ...readJSON(P('passport/features', f)), _file: `passport/features/${f}` }));
  const extra = {};
  for (const k of ['data', 'config']) if (exists(P(`passport/${k}.json`))) extra[k] = readJSON(P(`passport/${k}.json`));
  const ops = exists(P('passport/ops')) ? fs.readdirSync(P('passport/ops')).filter(f => f.endsWith('.md')) : [];
  return { pass, comps, feats, ops, ...extra };
}
function buildIndex(pass) {
  const idx = { cfg: pass, list: [], byId: new Map(), edges: [], imports: [], jsMeta: {}, pyMeta: {},
    tauri: { registered: [], emits: [], invokes: [], listens: [], registeredAt: null },
    add(x) { if (this.byId.has(x.id)) x.id += `@${x.line}`; this.list.push(x); this.byId.set(x.id, x); } };
  const files = walk(ROOT, [...SKIP_DIRS, ...(pass.ignore || [])]).filter(f => matchAny(f, pass.sources));
  idx.fileSet = new Set(files);
  for (const f of files) {
    const src = fs.readFileSync(P(f), 'utf8');
    if (f.endsWith('.rs')) indexRust(f, src, idx);
    else if (/\.(m|c)?[jt]sx?$/.test(f)) indexJS(f, src, idx);
    else if (f.endsWith('.py')) indexPy(f, src, idx);
  }
  rustCalls(idx); jsCalls(idx); pyCalls(idx);
  const enclosing = (file, line) => idx.list.filter(x => x.file === file && x.kind === 'fn' && x.line <= line && x.end >= line).sort((a, b) => b.line - a.line)[0];
  idx.enclosing = enclosing;
  // tauri: JS function that invokes → Rust command, so impact walks from Rust up to the screens
  for (const iv of idx.tauri.invokes) {
    const a = enclosing(iv.file, iv.line), b = idx.list.find(x => x.command && x.name === iv.command);
    iv.via = a?.id || null;
    if (a && b) idx.edges.push({ from: a.id, to: b.id, kind: 'invoke', how: 'tauri', approx: false });
  }
  const hash = crypto.createHash('sha256');
  for (const f of files.slice().sort()) hash.update(f).update(fs.readFileSync(P(f)));
  return { idx, files, sourceHash: hash.digest('hex').slice(0, 16) };
}
function ownership(comps, idx, files) {
  const fileOwner = {}, symOwner = {}, problems = [];
  for (const c of comps) {
    for (const g of c.code || []) {
      const hit = files.filter(f => globRe(g).test(f));
      if (!hit.length) problems.push(`${c.id}: code «${g}» не совпадает ни с одним файлом`);
      for (const f of hit) (fileOwner[f] ||= []).push(c.id);
    }
    for (const sid of c.symbols || []) {
      if (!idx.byId.has(sid)) { problems.push(`${c.id}: символ ${sid} не найден в коде`); continue; }
      (symOwner[sid] ||= []).push(c.id);
    }
  }
  const ownerOf = sym => {
    const x = typeof sym === 'string' ? idx.byId.get(sym) : sym;
    if (!x) return null;
    if (symOwner[x.id]) return symOwner[x.id][0];
    if (x.owner) { const s = symOwner[`${x.file}#${x.owner}`]; if (s) return s[0]; }
    return fileOwner[x.file]?.[0] || null;
  };
  return { fileOwner, symOwner, ownerOf, problems };
}

// ---------------------------------------------------------------- checks
function check(model, built) {
  const { comps, feats, pass } = model;
  const { idx, files } = built;
  const errors = [], warns = [];
  const ids = new Set(comps.map(c => c.id));
  const own = ownership(comps, idx, files);
  errors.push(...own.problems);

  // 1. format, unfilled drafts
  const need = { component: ['id', 'name', 'kind', 'layer', 'purpose', 'not'], feature: ['id', 'name', 'user', 'trigger', 'steps', 'verify'] };
  for (const c of comps) for (const k of need.component) if (!c[k] || (Array.isArray(c[k]) && !c[k].length)) errors.push(`${c._file}: нет поля ${k}`);
  for (const f of feats) for (const k of need.feature) if (!f[k] || (Array.isArray(f[k]) && !f[k].length)) errors.push(`${f._file}: нет поля ${k}`);
  for (const x of [...comps, ...feats]) { const t = JSON.stringify(x).match(/TODO/g); if (t) errors.push(`${x._file}: не заполнено (TODO: ${t.length})`); }
  if (JSON.stringify(pass).includes('TODO')) errors.push('passport/passport.json: не заполнено (TODO)');
  for (const c of comps) if (!pass.layers.includes(c.layer)) errors.push(`${c.id}: неизвестный слой ${c.layer} (слои — passport.json → layers)`);
  const dupe = comps.map(c => c.id).filter((x, i, a) => a.indexOf(x) !== i);
  if (dupe.length) errors.push(`повтор id компонента: ${dupe.join(', ')}`);
  if (!feats.length) warns.push('нет ни одной карточки функции (passport/features/*.json)');

  // 2. bindings
  const bind = (where, ref) => {
    if (!ref) return;
    if (ref.includes('#')) { if (!idx.byId.has(ref)) errors.push(`${where}: символа ${ref} нет в коде`); }
    else if (!exists(P(ref))) errors.push(`${where}: файла ${ref} нет`);
  };
  for (const c of comps) {
    for (const u of c.uses || []) if (!ids.has(u.to)) errors.push(`${c.id}: uses → неизвестный компонент ${u.to}`);
    for (const t of c.tests || []) bind(c.id, t);
    for (const fl of c.failures || []) for (const r of fl.refs || []) bind(c.id, r);
  }
  for (const f of feats) {
    f.steps.forEach((s, i) => {
      if (!ids.has(s.comp)) errors.push(`${f.id} шаг ${i + 1}: неизвестный компонент ${s.comp}`);
      for (const r of [].concat(s.at || [])) {
        bind(`${f.id} шаг ${i + 1}`, r);
        const o = idx.byId.get(r) ? own.ownerOf(r) : (own.fileOwner[r] || [])[0];
        if (o && o !== s.comp) errors.push(`${f.id} шаг ${i + 1}: ${r} принадлежит ${o}, а шаг отнесён к ${s.comp}`);
      }
    });
    for (const e of f.errors || []) for (const r of [].concat(e.at || [])) bind(`${f.id} ошибка`, r);
    for (const t of f.tests || []) bind(f.id, t);
  }
  for (const d of [...(model.data?.items || []), ...(model.config?.items || [])]) for (const r of [].concat(d.at || [])) bind(d.id, r);
  for (const d of pass.decisions || []) bind('decisions', d.ref);

  // 3. coverage
  for (const f of files) {
    const fo = own.fileOwner[f] || [];
    const syms = idx.list.filter(x => x.file === f && !x.test && !x.owner);
    const split = syms.some(x => own.symOwner[x.id]);
    if (fo.length > 1) errors.push(`файл ${f} заявлен несколькими компонентами: ${fo.join(', ')}`);
    if (split) {
      if (fo.length) errors.push(`файл ${f} делится по символам, но целиком заявлен ${fo.join(', ')}`);
      for (const x of syms) {
        const so = own.symOwner[x.id] || [];
        if (!so.length) errors.push(`символ ${x.id} (строка ${x.line}) не отнесён ни к одному компоненту`);
        if (so.length > 1) errors.push(`символ ${x.id} отнесён к нескольким: ${so.join(', ')}`);
      }
    } else if (!fo.length) errors.push(`файл ${f} не принадлежит ни одному компоненту`);
  }

  // 4. boundaries
  const declared = new Set();
  for (const c of comps) for (const u of c.uses || []) declared.add(`${c.id}>${u.to}`);
  const observed = new Map();
  const note = (a, b, why) => { if (a && b && a !== b) { const k = `${a}>${b}`; if (!observed.has(k)) observed.set(k, why); } };
  for (const e of idx.edges) { const a = idx.byId.get(e.from), b = idx.byId.get(e.to); if (a?.test) continue; note(own.ownerOf(a), own.ownerOf(b), `${e.from} → ${e.to}`); }
  for (const im of idx.imports) { if (isTestFile(im.from)) continue; const from = idx.list.find(x => x.file === im.from && x.line <= im.line && x.end >= im.line); if (from?.test) continue; note(from ? own.ownerOf(from) : (own.fileOwner[im.from] || [])[0], (own.fileOwner[im.to] || [])[0] || own.ownerOf(idx.list.find(x => x.file === im.to)), `import ${im.from} → ${im.to}`); }
  const T = idx.tauri;
  const cmdFn = name => idx.list.find(x => x.command && x.name === name);
  if (T.registeredAt) { const reg = idx.enclosing(T.registeredAt.file, T.registeredAt.line); for (const r of T.registered) note(own.ownerOf(reg), own.ownerOf(cmdFn(r)), `generate_handler! ${r}`); }
  for (const iv of T.invokes) if (!cmdFn(iv.command)) errors.push(`${iv.file}:${iv.line}: invoke('${iv.command}'), но #[tauri::command] с таким именем нет`);
  for (const em of T.emits) {
    const from = own.ownerOf(idx.enclosing(em.file, em.line));
    const to = new Set(T.listens.filter(l => l.event === em.event).map(l => pass.bridge ? (own.fileOwner[pass.bridge] || [])[0] : own.ownerOf(idx.enclosing(l.file, l.line)) || (own.fileOwner[l.file] || [])[0]));
    for (const t of to) note(from, t, `event ${em.event}`);
  }
  for (const [k, why] of observed) if (!declared.has(k)) errors.push(`граница: ${k.replace('>', ' → ')} есть в коде (${why}), но не объявлена в uses`);
  const codeComps = new Set(comps.filter(c => (c.code || []).length || (c.symbols || []).length).map(c => c.id));
  const quiet = new Set(pass.externalLayers || ['data', 'ext', 'delivery']);
  const layerOf = id => comps.find(c => c.id === id)?.layer;
  const runtimeOnly = new Set(comps.flatMap(c => (c.uses || []).filter(u => u.static === false).map(u => `${c.id}>${u.to}`)));
  for (const k of declared) { const [a, b] = k.split('>'); if (runtimeOnly.has(k)) continue; if (codeComps.has(a) && codeComps.has(b) && !quiet.has(layerOf(a)) && !quiet.has(layerOf(b)) && !observed.has(k)) warns.push(`связь ${a} → ${b} объявлена, но в коде не найдена (вызов мог не распознаться)`); }

  // 5. tauri: both sides agree
  const cmds = idx.list.filter(x => x.command).map(x => x.name);
  if (T.registeredAt) {
    for (const c of cmds) if (!T.registered.includes(c)) errors.push(`команда ${c} не зарегистрирована в generate_handler!`);
    for (const r of T.registered) if (!cmds.includes(r)) errors.push(`generate_handler! называет ${r}, но #[tauri::command] с таким именем нет`);
    for (const c of cmds) if (!T.invokes.some(i => i.command === c)) warns.push(`команда ${c} нигде не вызывается через invoke`);
  }
  if (T.emits.length || T.listens.length) {
    for (const e of new Set(T.emits.map(x => x.event))) if (!T.listens.some(l => l.event === e)) warns.push(`событие ${e} отправляется, но никто его не слушает (шаблон слушателя — passport.json → adapters.tauri.listen)`);
    for (const l of T.listens) if (!T.emits.some(e => e.event === l.event)) warns.push(`${l.file}:${l.line}: слушает ${l.event}, но Rust его не отправляет`);
  }

  // 6. freshness
  const stampP = P('passport/generated/stamp.json');
  if (!exists(stampP)) errors.push('нет passport/generated/stamp.json — запустите build');
  else if (readJSON(stampP).sourceHash !== built.sourceHash) errors.push('индекс устарел: код изменился после build — запустите node scripts/passport.mjs build');
  return { errors, warns, own };
}

// ---------------------------------------------------------------- outputs
const featuresOf = (feats, sym) => feats.filter(f => f.steps.some(s => [].concat(s.at || []).includes(sym))).map(f => f.id);
function exportIndex(model, built, own) {
  const { idx } = built; const T = idx.tauri;
  return {
    symbols: idx.list.map(({ body, ...x }) => ({ ...x, component: own.ownerOf(x), features: featuresOf(model.feats, x.id) })),
    edges: idx.edges.filter(e => !idx.byId.get(e.from)?.test),
    imports: idx.imports,
    commands: idx.list.filter(x => x.command).map(x => { const iv = T.invokes.find(i => i.command === x.name); return { name: x.name, at: x.id, line: x.line, via: iv?.via || null, component: own.ownerOf(x) }; }),
    events: [...new Set(T.emits.map(e => e.event))].map(ev => ({ event: ev, emitted: T.emits.filter(e => e.event === ev).map(e => `${e.file}:${e.line}`), listened: T.listens.filter(l => l.event === ev).map(l => `${l.file}:${l.line}`) })),
    routes: idx.list.filter(x => x.route).map(x => ({ route: x.route, at: x.id, component: own.ownerOf(x) })),
  };
}
const vid = id => id.replace(/[^\w]/g, '_');
function c4(model) {
  const q = s => `'${String(s).replace(/'/g, '’')}'`;
  let out = '// Generated by scripts/passport.mjs from passport/components/*.json — do not edit.\n// Preview: npx likec4 start passport/generated\n';
  out += `specification {\n${model.pass.layers.map(l => `  element ${vid(l)}`).join('\n')}\n}\n\nmodel {\n`;
  for (const c of model.comps) out += `  ${vid(c.id)} = ${vid(c.layer)} ${q(c.name)} {\n    description ${q(c.purpose)}\n  }\n`;
  for (const c of model.comps) for (const u of c.uses || []) out += `  ${vid(c.id)} -> ${vid(u.to)} ${q(u.how)}\n`;
  return out + '}\n\nviews {\n  view index {\n    include *\n  }\n}\n';
}
function mermaid(model) {
  let out = '# Карта компонентов (генерируется)\n\nСгенерировано `node scripts/passport.mjs build` из `passport/components/*.json`. Не править руками.\n\n```mermaid\nflowchart LR\n';
  for (const layer of model.pass.layers) {
    const cs = model.comps.filter(c => c.layer === layer); if (!cs.length) continue;
    out += `  subgraph ${vid(layer)}["${(model.pass.layerNames || {})[layer] || layer}"]\n`;
    for (const c of cs) out += `    ${vid(c.id)}["${c.name.replace(/"/g, "'")}"]\n`;
    out += '  end\n';
  }
  for (const c of model.comps) for (const u of c.uses || []) out += `  ${vid(c.id)} -->|${String(u.how).replace(/[|"]/g, ' ')}| ${vid(u.to)}\n`;
  return out + '```\n';
}
function gitHead() {
  try {
    const head = fs.readFileSync(P('.git/HEAD'), 'utf8').trim();
    if (!head.startsWith('ref:')) return head.slice(0, 7);
    const ref = head.slice(5); const f = P('.git', ref);
    if (exists(f)) return fs.readFileSync(f, 'utf8').trim().slice(0, 7);
    const packed = exists(P('.git/packed-refs')) ? fs.readFileSync(P('.git/packed-refs'), 'utf8') : '';
    return (packed.split('\n').find(l => l.endsWith(ref)) || '').slice(0, 7) || null;
  } catch { return null; }
}
function build() {
  const model = loadModel();
  const built = buildIndex(model.pass);
  fs.mkdirSync(P('passport/generated'), { recursive: true });
  const stamp = { sourceHash: built.sourceHash, files: built.files.length, symbols: built.idx.list.length, edges: built.idx.edges.length, builtAt: new Date().toISOString(), commit: gitHead() };
  fs.writeFileSync(P('passport/generated/stamp.json'), JSON.stringify(stamp, null, 1) + '\n');
  const res = check(model, built);
  const index = exportIndex(model, built, res.own);
  fs.writeFileSync(P('passport/generated/index.json'), JSON.stringify(index, null, 1) + '\n');
  fs.writeFileSync(P('passport/generated/model.c4'), c4(model));
  fs.writeFileSync(P('passport/generated/map.md'), mermaid(model));
  const tplPath = [P('scripts/passport_view.html'), path.join(HERE, 'passport_view.html')].find(exists);
  if (!tplPath) console.log('  нет scripts/passport_view.html — страница view.html не собрана');
  else {
    const data = { pass: model.pass, comps: model.comps.map(({ _file, ...c }) => ({ ...c, file: _file })), feats: model.feats.map(({ _file, ...f }) => ({ ...f, file: _file })),
      data: model.data, config: model.config, ops: model.ops, stamp, check: { errors: res.errors, warns: res.warns },
      symbols: Object.fromEntries(index.symbols.map(s => [s.id, { line: s.line, component: s.component }])), commands: index.commands, events: index.events, routes: index.routes,
      agents: ['AGENTS.md', 'CLAUDE.md'].map(f => exists(P(f)) ? fs.readFileSync(P(f), 'utf8') : '').find(Boolean) || '' };
    fs.writeFileSync(P('passport/view.html'), fs.readFileSync(tplPath, 'utf8').replace('/*PASSPORT_DATA*/null', JSON.stringify(data).replace(/</g, '\\u003c')));
  }
  report(res, stamp);
}
function report(res, stamp) {
  if (stamp) console.log(`Индекс: файлов ${stamp.files}, символов ${stamp.symbols}, вызовов ${stamp.edges}, код ${stamp.sourceHash}, коммит ${stamp.commit}`);
  for (const w of res.warns) console.log(`  предупреждение: ${w}`);
  for (const e of res.errors) console.log(`  ОШИБКА: ${e}`);
  console.log(res.errors.length ? `\npassport:check — ${res.errors.length} ошибок. Карточки и код расходятся.` : `\npassport:check — зелёный (предупреждений: ${res.warns.length}).`);
  process.exitCode = res.errors.length ? 1 : 0;
}

// ---------------------------------------------------------------- init: a draft passport for a new project
const OPS = {
  'debug.md': `# Плейбук: отладка по симптому

1. Запишите симптом словами пользователя и точный текст ошибки.
2. Найдите его в \`passport/ops/symptoms.md\` и в «Типовых сбоях» карточек (\`failures\`). Нашли — идите по «где смотреть».
3. Определите функцию (действие пользователя) и получите её путь: \`node scripts/passport.mjs where feature.<id>\`.
   Читайте только названные строки.
4. Сверьте путь с журналом и состоянием на диске (раздел «Данные» паспорта).
5. На подозрительном символе: \`where <символ>\` — кто вызывает и что вызывает он.
6. Воспроизведите сбой до правки (способ — поле «Как отделить» в карточке компонента).
7. Перед правкой: \`impact <символ>\` — затронутые компоненты, функции и тесты.
8. После правки: проверка из карточки (\`verify\`), тесты проекта, \`build\` и \`check\` паспорта — зелёные.
9. Новый симптом — строка в \`symptoms.md\` и сбой в \`failures\` карточки.
`,
  'symptoms.md': `# Симптом → где смотреть

| Симптом (что видит пользователь) | Вероятная причина | Где смотреть | Компонент |
|---|---|---|---|
`,
  'tune.md': `# Плейбук: настройка

1. Найдите параметр в \`passport/config.json\`: вид, значение по умолчанию, на что влияет, где в коде (\`at\`), кто читает.
2. Меняйте его там, где он задаётся (окружение, файл конфигурации, константа), а не в месте использования.
3. Проверьте функции, у которых параметр в поле \`config\`.
4. Новый параметр — сначала строка в \`config.json\`, потом код; затем \`build\` и \`check\`.
`,
  'isolate.md': `# Плейбук: разобрать на части

1. \`node scripts/passport.mjs where <компонент>\` — символы, входящие и исходящие связи, функции.
2. Запустите компонент способом из поля «Как отделить» (\`isolate\`) его карточки.
3. Каждую исходящую связь замените заглушкой с тем же контрактом.
4. Проверка из \`verify\` должна пройти без остального приложения.
5. Вынесли часть в отдельный модуль — обновите \`code\`/\`symbols\` и \`uses\`, затем \`build\` и \`check\`.
`,
  'add-feature.md': `# Плейбук: новая функция

1. Сначала карточка \`passport/features/<id>.json\`: что делает пользователь, запуск, вход, результат, ошибки, проверка.
2. Новая связь между компонентами — сначала в \`uses\`; новый компонент — новая карточка.
3. Код.
4. Заполните \`at\` шагов настоящими символами (\`файл#Символ\`, методы — \`файл#Тип::метод\` в Rust, \`файл#Класс.метод\` в JS/Python).
5. \`build\` и \`check\` — зелёные; тесты проекта — зелёные.
`,
  'release.md': `# Плейбук: сборка и выпуск

1. Все проверки проекта и \`node scripts/passport.mjs check\` — зелёные.
2. Команды сборки — \`passport/passport.json → run\`.
3. Версия — во всех местах, где она записана, включая \`passport/passport.json → version\`.
`,
};
function init() {
  if (exists(P('passport/passport.json'))) { console.error('passport/passport.json уже есть — init ничего не перезаписывает. Правьте карточки и запускайте build.'); process.exit(2); }
  const all = walk(ROOT, SKIP_DIRS).filter(f => CODE_EXT.includes(f.split('.').pop()) && !/\.min\.(js|css)$/.test(f) && !f.startsWith('scripts/passport'));
  const exts = [...new Set(all.map(f => f.split('.').pop()))].sort();
  const sources = exts.length ? [`**/*.{${exts.join(',')}}`] : ['**/*'];
  // one draft component per folder (two levels deep); files at the root each get their own
  const groups = {};
  for (const f of all) {
    const segs = f.split('/'); const dir = segs.slice(0, -1);
    const key = dir.length === 0 ? f : dir.slice(0, 2).join('/');
    (groups[key] ||= []).push(f);
  }
  const keys = Object.keys(groups);
  const hasSub = k => keys.some(o => o !== k && o.startsWith(k + '/'));
  const cards = keys.map(k => {
    const isFile = groups[k].length === 1 && groups[k][0] === k;
    const depth = k.split('/').length;
    const code = isFile ? [k] : depth >= 2 || !hasSub(k) ? [`${k}/**`] : [`${k}/*.{${[...new Set(groups[k].map(f => f.split('.').pop()))].join(',')}}`];
    const id = k.replace(/\.[^./]+$/, '').replace(/[^\w/]+/g, '_').replace(/\//g, '.').toLowerCase() || 'root';
    return { id, name: `TODO: имя для ${k}`, kind: 'TODO: модуль, экран, граница, данные…', layer: 'TODO', purpose: 'TODO: за что отвечает, одним-двумя предложениями',
      not: 'TODO: за что НЕ отвечает', code, symbols: [], uses: [], data: { reads: '—', writes: '—' }, observe: 'TODO: журнал, файлы, команды, по которым видно, что он делает',
      verify: 'TODO: команда или сценарий проверки', tests: [], invariants: [], failures: [], isolate: 'TODO: как запустить отдельно', _files: groups[k].length };
  });
  fs.mkdirSync(P('passport/components'), { recursive: true });
  fs.mkdirSync(P('passport/features'), { recursive: true });
  fs.mkdirSync(P('passport/ops'), { recursive: true });
  const pass = {
    app: path.basename(ROOT), version: 'TODO', summary: 'TODO: что делает приложение, двумя-тремя предложениями',
    stack: ['TODO'], language: 'Язык карточек: русский.',
    run: { 'TODO: что': 'TODO: команда' }, checks: { passport: 'node scripts/passport.mjs check' },
    sources, ignore: [],
    layers: ['ui', 'app', 'core', 'data', 'ext', 'delivery'],
    layerNames: { ui: 'Интерфейс', app: 'Приложение', core: 'Ядро', data: 'Данные', ext: 'Снаружи', delivery: 'Сборка и проверки' },
    externalLayers: ['data', 'ext', 'delivery'],
    invariants: [], decisions: [],
  };
  fs.writeFileSync(P('passport/passport.json'), JSON.stringify(pass, null, 2) + '\n');
  for (const c of cards) { const { _files, ...card } = c; fs.writeFileSync(P('passport/components', `${c.id}.json`), JSON.stringify(card, null, 1) + '\n'); }
  fs.writeFileSync(P('passport/features/_example.json'), JSON.stringify({
    id: 'feature.example', name: 'Название действия пользователя', user: 'Что делает пользователь и что получает — его словами.',
    trigger: 'Кнопка, команда, событие, расписание', input: 'Что подаётся на вход', output: 'Что получается',
    steps: [{ comp: 'id компонента', does: 'что происходит на этом шаге', at: ['путь/к/файлу.py#функция', 'путь/к/файлу.rs#Тип::метод'] }],
    reads: 'что читает', writes: 'что пишет', config: ['ИМЯ_НАСТРОЙКИ'],
    errors: [{ when: 'когда', shows: 'что видит пользователь', at: ['файл#символ'] }],
    tests: ['файл#тест'], verify: 'команда или сценарий проверки',
  }, null, 1) + '\n');
  fs.writeFileSync(P('passport/data.json'), JSON.stringify({ items: [] }, null, 1) + '\n');
  fs.writeFileSync(P('passport/config.json'), JSON.stringify({ items: [] }, null, 1) + '\n');
  for (const [f, t] of Object.entries(OPS)) fs.writeFileSync(P('passport/ops', f), t);
  console.log(`Черновик паспорта: ${cards.length} компонентов по папкам, файлов кода ${all.length}, расширения: ${exts.join(', ')}.`);
  for (const c of cards) console.log(`  ${c.id.padEnd(32)} ${c.code.join(', ')}  (${c._files} файлов)`);
  console.log(`
Дальше:
  1. Прочитайте код и заполните passport/passport.json и карточки (поля с TODO). Объединяйте и делите черновые компоненты по смыслу,
     а не по папкам; большой файл можно поделить по символам (поле symbols вместо code).
  2. node scripts/passport.mjs build — индекс; ошибки «граница: … не объявлена» подскажут, какие uses дописать.
  3. Карточки функций: passport/features/<id>.json (образец — _example.json, его build не читает).
  4. node scripts/passport.mjs check — зелёный. Откройте passport/view.html.`);
}

// ---------------------------------------------------------------- queries
function query(kind, q) {
  if (!q) { console.error(`нужен аргумент: node scripts/passport.mjs ${kind} <символ|файл|компонент|функция>`); process.exit(2); }
  const model = loadModel();
  const built = buildIndex(model.pass);
  const own = ownership(model.comps, built.idx, built.files);
  const { idx } = built;
  const comp = model.comps.find(c => c.id === q);
  const feat = model.feats.find(f => f.id === q);
  if (comp) {
    console.log(`${comp.id} — ${comp.name} [${comp.layer}]\n  ${comp.purpose}\n  не отвечает: ${comp.not}\n  карточка: ${comp._file}`);
    for (const x of idx.list.filter(x => own.ownerOf(x) === comp.id && !x.test)) console.log(`  ${x.file}:${x.line}  ${x.qual}${x.command ? '  (команда)' : ''}${x.route ? `  ${x.route}` : ''}`);
    for (const u of comp.uses || []) console.log(`  → ${u.to}: ${u.how} ${u.what || ''}`);
    for (const c of model.comps) for (const u of c.uses || []) if (u.to === comp.id) console.log(`  ← ${c.id}: ${u.how} ${u.what || ''}`);
    console.log(`  функции: ${model.feats.filter(f => f.steps.some(s => s.comp === comp.id)).map(f => f.id).join(', ') || '—'}`);
    return;
  }
  if (feat) {
    console.log(`${feat.id} — ${feat.name}\n  ${feat.user}\n  запуск: ${feat.trigger}\n  карточка: ${feat._file}`);
    feat.steps.forEach((s, i) => console.log(`  ${i + 1}. [${s.comp}] ${s.does}\n     ${[].concat(s.at || []).map(r => { const x = idx.byId.get(r); return x ? `${x.file}:${x.line} ${x.qual}` : r; }).join(', ')}`));
    return;
  }
  const hits = idx.list.filter(x => x.id === q || x.qual === q || x.name === q || x.file === q || `${x.file}:${x.line}` === q);
  if (!hits.length) { console.log(`ничего не найдено по «${q}». Примеры: имя функции, Тип::метод, Класс.метод, путь к файлу, id компонента, feature.<id>`); process.exitCode = 1; return; }
  const callersOf = id => idx.edges.filter(e => e.to === id).map(e => e.from);
  for (const x of hits.slice(0, 12)) {
    console.log(`${x.id}  (${x.file}:${x.line}${x.end && x.end !== x.line ? `–${x.end}` : ''}) ${x.kind}${x.command ? ', команда Tauri' : ''}${x.test ? ', тест' : ''}`);
    console.log(`  компонент: ${own.ownerOf(x) || '—'}; функции: ${featuresOf(model.feats, x.id).join(', ') || '—'}`);
    if (kind === 'where') {
      console.log(`  вызывают: ${callersOf(x.id).join(', ') || '—'}`);
      console.log(`  вызывает: ${idx.edges.filter(e => e.from === x.id).map(e => e.to).join(', ') || '—'}`);
      continue;
    }
    const seen = new Map([[x.id, 0]]), queue = [x.id];
    while (queue.length) { const cur = queue.shift(); for (const c of callersOf(cur)) if (!seen.has(c)) { seen.set(c, seen.get(cur) + 1); queue.push(c); } }
    seen.delete(x.id);
    const rows = [...seen].sort((a, b) => a[1] - b[1]);
    for (const [id, d] of rows) { const y = idx.byId.get(id); if (!y?.test) console.log(`  глубина ${d}: ${id}${y ? ` (${y.file}:${y.line}) [${own.ownerOf(y)}]` : ''}`); }
    const tests = rows.map(([id]) => idx.byId.get(id)).filter(y => y?.test);
    console.log(`  тесты, которые его проходят: ${tests.map(y => `${y.name} (${y.file}:${y.line})`).join(', ') || 'не найдены — проверять сценарием'}`);
    const comps = new Set(rows.filter(([id]) => !idx.byId.get(id)?.test).map(([id]) => own.ownerOf(id)).concat(own.ownerOf(x)));
    const feats = new Set([x.id, ...seen.keys()].flatMap(id => featuresOf(model.feats, id)));
    console.log(`  затронуты компоненты: ${[...comps].filter(Boolean).join(', ')}\n  затронуты функции: ${[...feats].join(', ') || '—'}`);
    console.log('  связи найдены по именам (approx): перед большой правкой перепроверьте поиском по коду');
  }
}

const [cmd, arg] = process.argv.slice(2);
if (cmd === 'init') init();
else if (cmd === 'build') build();
else if (cmd === 'check') { const model = loadModel(); report(check(model, buildIndex(model.pass))); }
else if (cmd === 'where' || cmd === 'impact') query(cmd, arg);
else { console.log('node scripts/passport.mjs init | build | check | where <q> | impact <symbol>'); process.exitCode = 2; }
