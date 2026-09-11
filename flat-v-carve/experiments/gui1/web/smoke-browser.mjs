// Real-browser smoke test for the browser build. Native tests cannot catch a
// wasm-only panic: a single trapped frame leaves the page as a static image and
// every native test still passes. This drives a real Chrome over CDP, loads the
// application, loads both builtin references and fails on any exception.
//
// Prerequisites: a built `pkg/`, `node web/serve.mjs` on 127.0.0.1:5181, and
// Chrome (or Edge with --browser=edge). It is an opt-in platform check, like the
// golden layout tests, because it needs a browser with WebGPU.
import {spawn} from 'node:child_process';
import {mkdtempSync, mkdirSync, rmSync, writeFileSync} from 'node:fs';
import {tmpdir} from 'node:os';
import path from 'node:path';

const edge = process.argv.includes('--browser=edge');
const chrome = edge
  ? 'C:/Program Files (x86)/Microsoft/Edge/Application/msedge.exe'
  : 'C:/Program Files/Google/Chrome/Application/chrome.exe';
const base = process.argv.find(argument => argument.startsWith('--url='))?.slice('--url='.length)
  ?? 'http://127.0.0.1:5181/web/index.html';
const port = 9334;
const profile = mkdtempSync(path.join(tmpdir(), 'gui1-smoke-'));
const browser = spawn(chrome, [
  `--remote-debugging-port=${port}`,
  `--user-data-dir=${profile}`,
  '--headless=new',
  '--window-size=1280,800',
  '--no-first-run',
  '--no-default-browser-check',
  '--disable-extensions',
  '--enable-unsafe-webgpu',
  'about:blank',
], {stdio: 'ignore', windowsHide: true});

const sleep = milliseconds => new Promise(resolve => setTimeout(resolve, milliseconds));
const problems = [];
let target;
for (let attempt = 0; attempt < 60 && !target; attempt++) {
  await sleep(500);
  try {
    const list = await (await fetch(`http://127.0.0.1:${port}/json/list`)).json();
    target = list.find(entry => entry.type === 'page' && entry.webSocketDebuggerUrl);
  } catch {}
}
if (!target) {
  console.error('FAILED: no debuggable browser page; is Chrome installed and the server running?');
  browser.kill();
  rmSync(profile, {recursive: true, force: true});
  process.exit(2);
}

const socket = new WebSocket(target.webSocketDebuggerUrl);
let nextId = 1;
const send = (method, params = {}) => new Promise(resolve => {
  const id = nextId++;
  const onMessage = event => {
    const payload = JSON.parse(event.data);
    if (payload.id === id) {
      socket.removeEventListener('message', onMessage);
      resolve(payload.result);
    }
  };
  socket.addEventListener('message', onMessage);
  socket.send(JSON.stringify({id, method, params}));
});
await new Promise(resolve => socket.addEventListener('open', resolve));
socket.addEventListener('message', event => {
  const payload = JSON.parse(event.data);
  if (payload.method === 'Runtime.consoleAPICalled' && payload.params.type === 'error') {
    problems.push(`console.error: ${payload.params.args.map(a => a.value ?? a.description ?? '').join(' ')}`);
  }
  if (payload.method === 'Runtime.exceptionThrown') {
    problems.push(`exception: ${payload.params.exceptionDetails.exception?.description ?? payload.params.exceptionDetails.text}`);
  }
});
await send('Runtime.enable');
await send('Page.enable');
await send('Page.navigate', {url: base});

const evaluate = async expression => {
  const result = await send('Runtime.evaluate', {expression, returnByValue: true});
  return result?.result?.value;
};
const state = async () => {
  const text = await evaluate('globalThis.GUI1?.probe_state?.() ?? ""');
  try { return JSON.parse(text || '{}'); } catch { return {}; }
};
const waitFor = async (test, label, attempts = 60) => {
  for (let attempt = 0; attempt < attempts; attempt++) {
    await sleep(1000);
    const current = await state();
    if (test(current)) return current;
  }
  throw new Error(`timed out waiting for ${label}; last state ${JSON.stringify(await state())}`);
};
const click = async (x, y) => {
  for (const type of ['mousePressed', 'mouseReleased']) {
    await send('Input.dispatchMouseEvent', {type, x, y, button: 'left', clickCount: 1});
  }
};
const pressKey = async (key, code, modifiers = 0) => {
  for (const type of ['keyDown', 'keyUp']) {
    await send('Input.dispatchKeyEvent', {type, key, code, windowsVirtualKeyCode: key.toUpperCase().charCodeAt(0), nativeVirtualKeyCode: key.toUpperCase().charCodeAt(0), modifiers});
  }
};

try {
  const started = await waitFor(current => Object.keys(current).length > 0, 'the first published frame');
  console.log(`startup: dpi ${started.dpi} · status ${JSON.stringify(started.status)}`);
  const evidence = {
    browser: edge ? 'Edge' : 'Chrome',
    url: base,
    started,
    checks: [],
  };
  const record = (label, state) => evidence.checks.push({label, state});
  // Top command panel: "Small combined" then "Flower reference" at 1280 px wide.
  await click(266, 13);
  const small = await waitFor(current => current.motions === 37, 'the small reference');
  console.log(`small reference: ${small.motions} motions · ${small.pages} pages · status ${JSON.stringify(small.status)}`);
  record('small reference loaded', small);
  await click(379, 13);
  const flower = await waitFor(current => current.motions === 22883, 'the flower reference', 120);
  console.log(`flower reference: ${flower.motions} motions · ${flower.pages} pages · resident ${flower.residentPages} · upload ${flower.uploadBytes} B · tiles ${flower.tilesCopied} · heap peak ${(flower.heapPeakBytes / 1048576).toFixed(1)} MiB`);
  record('flower reference loaded', flower);

  // Keyboard focus, the Ctrl+F shortcut and the IME text agent. egui drops key
  // events until the canvas reports focus, which needs tabindex on the canvas.
  await click(600, 400);
  const focused = await waitFor(current => current.canvasFocused === true, 'canvas keyboard focus');
  console.log(`canvas keyboard focus: ${focused.canvasFocused}`);
  record('canvas takes keyboard focus', {canvasFocused: focused.canvasFocused});
  await pressKey('f', 'KeyF', 2); // 2 = Ctrl
  const control = await waitFor(current => current.focused === true, 'Ctrl+F to focus a control');
  console.log(`Ctrl+F focused an editable control: ${control.focused}`);
  record('Ctrl+F focuses a control', {focused: control.focused});
  const commit = await evaluate(`(() => {
    const agent = document.body.querySelector('input');
    if (!agent) return 'missing text agent';
    agent.dispatchEvent(new CompositionEvent('compositionstart', {bubbles: true, data: ''}));
    agent.dispatchEvent(new CompositionEvent('compositionupdate', {bubbles: true, data: '工具'}));
    agent.dispatchEvent(new CompositionEvent('compositionend', {bubbles: true, data: '工具'}));
    return 'dispatched';
  })()`);
  if (commit !== 'dispatched') throw new Error(`IME probe failed: ${commit}`);
  const ime = await waitFor(current => current.search === '工具', 'the committed IME text');
  console.log(`IME commit reached the focused field: ${JSON.stringify(ime.search)}`);
  record('IME commit reaches the focused field', {search: ime.search});

  const alive = await evaluate('typeof globalThis.GUI1?.probe_state === "function"');
  if (alive !== true) throw new Error('the application stopped responding after loading a scene');
  if (problems.length) throw new Error(`browser reported ${problems.length} error(s)`);
  const out = path.resolve('artifacts');
  mkdirSync(out, {recursive: true});
  writeFileSync(path.join(out, 'browser-smoke.json'), JSON.stringify({
    note: 'Real-browser smoke test: the wasm build starts, loads both builtin references, takes keyboard focus, runs Ctrl+F and delivers an IME commit. A real OS IME tour and real OS drag gesture remain manual.',
    ...evidence,
    consoleErrors: problems,
  }, null, 2) + '\n');
  console.log('browser smoke test passed');
  browser.kill();
  await sleep(500);
  rmSync(profile, {recursive: true, force: true});
} catch (error) {
  console.error(`FAILED: ${error?.message ?? error}`);
  console.error(problems.join('\n') || '(no console errors captured)');
  browser.kill();
  await sleep(500);
  rmSync(profile, {recursive: true, force: true});
  process.exit(1);
}
