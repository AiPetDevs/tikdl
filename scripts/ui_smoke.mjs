#!/usr/bin/env node
// ui_smoke.mjs — the screen really opens (skill ui-architect). Needs the playwright package.
//   node scripts/ui_smoke.mjs   pages from ui-check.json -> "smoke": ["mockup/App.html", "mockup/App.html#/kit", "http://127.0.0.1:5173/"]
// Fails on: a JS error or console.error, an empty page, a failed request or a 4xx/5xx answer. Screenshots: shots/ at
// two widths. "smokeReady": a selector to wait for instead of a fixed pause. Point it at a build that needs no sign-in.
import fs from 'node:fs';
import path from 'node:path';
import { pathToFileURL } from 'node:url';
import { createRequire } from 'node:module';
import process from 'node:process';   // explicit: project linters without Node globals pass this file
import console from 'node:console';

const ROOT = process.cwd();
const cfgPath = path.join(ROOT, 'ui-check.json');
const C = fs.existsSync(cfgPath) ? JSON.parse(fs.readFileSync(cfgPath, 'utf8')) : {};
const pages = C.smoke || [];
const widths = C.smokeWidths || [1440, 900];
if (!pages.length) { console.error('ui_smoke: ui-check.json has no "smoke" list - name the pages and the kit showcase'); process.exit(1); }
let chromium;
try { ({ chromium } = createRequire(path.join(ROOT, 'x.js'))('playwright')); }
catch { try { ({ chromium } = await import('playwright')); } catch { console.error('ui_smoke: no playwright package (npm i -D playwright)'); process.exit(1); } }

const toUrl = p => /^(https?|file):/.test(p) ? p : (() => { const [f, h] = p.split('#'); return pathToFileURL(path.join(ROOT, f)).href + (h ? '#' + h : ''); })();
const outDir = path.join(ROOT, 'shots'); fs.mkdirSync(outDir, { recursive: true });
const browser = await chromium.launch();
let failed = 0;
for (const p of pages) for (const w of widths) {
  const page = await browser.newPage({ viewport: { width: w, height: 900 } });
  const errs = [];
  page.on('pageerror', e => errs.push('JS: ' + e.message));
  page.on('console', m => { if (m.type() === 'error') errs.push('console: ' + m.text()); });
  page.on('requestfailed', r => errs.push('network: ' + r.url() + ' ' + (r.failure()?.errorText || '')));
  page.on('response', r => { if (r.status() >= 400) errs.push(`network: ${r.status()} ${r.url()}`); });
  try {
    await page.goto(toUrl(p), { waitUntil: 'load', timeout: 15000 });
    // "smokeReady": a selector the app sets when its data arrived (live feeds need more than a fixed pause)
    if (C.smokeReady) await page.waitForSelector(C.smokeReady, { timeout: 15000 }); else await page.waitForTimeout(400);
  }
  catch (e) { errs.push('open: ' + e.message.split('\n')[0]); }
  const visible = await page.evaluate(() => {
    const els = [...document.body?.querySelectorAll('*') || []];
    return els.filter(e => { const r = e.getBoundingClientRect(); return r.width > 0 && r.height > 0 && getComputedStyle(e).visibility !== 'hidden'; }).length;
  }).catch(e => { errs.push('count visible: ' + e.message.split('\n')[0]); return 0; });
  if (visible < 3) errs.push(`empty page: ${visible} visible elements`);
  const shot = path.join(outDir, `${p.replace(/[^\w.-]+/g, '_')}@${w}.png`);
  await page.screenshot({ path: shot, fullPage: true }).catch(e => errs.push('screenshot: ' + e.message.split('\n')[0]));
  console.log(`${errs.length ? 'FAIL' : 'PASS'}  ${p} @${w}px  (${visible} visible elements) -> ${path.relative(ROOT, shot)}`);
  for (const e of errs) console.log('      ' + e);
  failed += errs.length ? 1 : 0;
  await page.close();
}
await browser.close();
console.log(failed ? `\n${failed} checks failed. Look at the screenshots too.` : '\nAll pages open. Now look at the screenshots.');
process.exit(failed ? 1 : 0);
