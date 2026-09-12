// Real-browser smoke test for the browser build. Native tests cannot catch a
// wasm-only panic: a single trapped frame leaves the page as a static image and
// every native test still passes. This drives a real Chrome over CDP, loads the
// application, edits the canonical flower job and exercises the retained loop.
//
// Prerequisites: a built `pkg/`, `node web/serve.mjs` on 127.0.0.1:5182, and
// Chrome (or Edge with --browser=edge). It needs a browser with WebGPU.
import {spawn} from 'node:child_process';
import {createHash} from 'node:crypto';
import {mkdtempSync, mkdirSync, rmSync, writeFileSync, readFileSync, existsSync} from 'node:fs';
import {tmpdir} from 'node:os';
import path from 'node:path';

const edge = process.argv.includes('--browser=edge');
const chrome = edge
  ? 'C:/Program Files (x86)/Microsoft/Edge/Application/msedge.exe'
  : 'C:/Program Files/Google/Chrome/Application/chrome.exe';
const base = process.argv.find(argument => argument.startsWith('--url='))?.slice('--url='.length)
  ?? 'http://127.0.0.1:5182/web/index.html';
const port = Number(process.argv.find(argument=>argument.startsWith('--port='))?.slice(7)??9335);
const profile = mkdtempSync(path.join(tmpdir(), 'gui2-smoke-'));
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
  const text = await evaluate('globalThis.CAM_GUI?.probe_state?.() ?? ""');
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
    await send('Input.dispatchKeyEvent', {type, key, code, windowsVirtualKeyCode: key==='Escape'?27:key.toUpperCase().charCodeAt(0), nativeVirtualKeyCode: key==='Escape'?27:key.toUpperCase().charCodeAt(0), modifiers});
  }
};

const control = async label => {
  for(let attempt=0;attempt<12;attempt++) {
    const current=await state(); const rect=current.controls?.[label];
    if(!rect)throw new Error(`Missing control ${label}`);
    const nav=label.startsWith('Artwork ') || ['Setup','Machine','Artwork','Cutting','Inspect result','Endmill tool','V-bit tool','+ Import artwork'].includes(label);
    const clip=current.controls?.[nav?'Navigator viewport':'Inspector viewport'];
    const bottom=clip?.[3]??await evaluate('innerHeight-65');
    const top=clip?.[1]??150;
    if(rect[1]>=top && rect[3]<=bottom || !nav && rect[0]<(clip?.[0]??850) || ['Filter fields','File','Generate','Prepare','Simulate','Export…','Prepare checked output','Save job','Undo','Redo','Cancel','Restore draft','Retry previous save'].includes(label)) {
      await click((rect[0]+rect[2])/2,(rect[1]+rect[3])/2); await sleep(120);return;
    }
    const x=nav?100:1100,y=(top+bottom)/2;
    await send('Input.dispatchMouseEvent',{type:'mouseMoved',x,y});
    await send('Input.dispatchMouseEvent',{type:'mouseWheel',x,y,deltaX:0,deltaY:rect[1]<top?-200:200});await sleep(200);
  }
  throw new Error(`Could not scroll to ${label}`);
};
const edit = async(label,text)=>{
  await control('Filter fields');await pressKey('a','KeyA',2);await send('Input.insertText',{text:label});await sleep(160);
  await control(label);await pressKey('a','KeyA',2);
  await send('Input.insertText',{text});await sleep(100);
};
const out=path.resolve('artifacts/gui/browser-smoke',new Date().toISOString().replaceAll(':','-'));mkdirSync(out,{recursive:true});
const screenshot=async name=>{const shot=await send('Page.captureScreenshot',{format:'png'});const bytes=Buffer.from(shot.data,'base64');writeFileSync(path.join(out,name),bytes);return bytes;};
const checks=[];
const record=(label,value)=>{checks.push({label,value});console.log(label);};
const chooseFile=async(label,filename)=>{
  await send('Page.setInterceptFileChooserDialog',{enabled:true});
  let listener;
  const chosen=new Promise((resolve,reject)=>{
    const timer=setTimeout(()=>{socket.removeEventListener('message',listener);reject(new Error('No file chooser for '+label));},5000);
    listener=event=>{const message=JSON.parse(event.data);if(message.method==='Page.fileChooserOpened'){clearTimeout(timer);socket.removeEventListener('message',listener);resolve(message.params.backendNodeId);}};
    socket.addEventListener('message',listener);
  });
  await control(label);
  const backendNodeId=await chosen;
  await send('DOM.setFileInputFiles',{backendNodeId,files:(Array.isArray(filename)?filename:[filename]).map(name=>path.resolve(name))});
  await send('Page.setInterceptFileChooserDialog',{enabled:false});
};
try {
  await waitFor(s=>s.gui2,'GUI2 first frame');
  await send('Browser.setDownloadBehavior',{behavior:'allow',downloadPath:out});
  if(process.argv.includes('--gui4')) {
    const {gui4Scenario}=await import('./gui4-scenario.mjs');
    await gui4Scenario({control,edit,state,waitFor,send,evaluate,sleep,record,screenshot,readFileSync,click,pressKey,path,out,chooseFile});
  } else if(process.argv.includes('--gui3')) {
    const {gui3Scenario}=await import('./gui3-scenario.mjs');
    await gui3Scenario({control,edit,state,waitFor,send,evaluate,sleep,record,screenshot,readFileSync,click,pressKey,path,out,chooseFile});
  } else if(process.argv.includes('--authoring')) {
    const {authoringScenario}=await import('./authoring-scenario.mjs');
    await authoringScenario({control,edit,state,waitFor,send,evaluate,sleep,record,screenshot,readFileSync,writeFileSync,path,out});
  } else {
  await control('File');await control('Flower fixture');await waitFor(s=>s.job?.name==='flower_box.svg','canonical flower');
  await control('Machine');await control('Example machine');await control('Apply flower machine profile');await waitFor(s=>s.job?.machine,'applied machine');
  await control('Cutting');await edit('Roughing feed','1900');await waitFor(s=>s.job?.feed===1900,'real feed edit');
  await edit('Maximum depth','1.1');await waitFor(s=>s.job?.depth===1.1,'real depth edit');
  await control('Generate');const generated=await waitFor(s=>s.current&&!s.active&&s.motions>20000,'generated edited execution',120);
  record('real edits generate',generated);
  await control('Simulate');await control('After endmill');const rough=await waitFor(s=>!s.active&&s.stockPrefix>0&&s.stockPrefix<s.motions,'roughing stock');record('roughing stock',rough.stockPrefix);
  await control('After V-bit');const final=await waitFor(s=>!s.active&&s.stockPrefix===s.motions,'finishing stock');record('finishing stock',final.stockPrefix);
  const rect=(await state()).controls['Stock motion'];await click(rect[0]+35,(rect[1]+rect[3])/2);
  const scrub=await waitFor(s=>!s.active&&s.stockPrefix>0&&s.stockPrefix<final.stockPrefix&&s.stockPrefix!==rough.stockPrefix,'backward arbitrary scrub');record('backward scrub',scrub.stockPrefix);
  await control('Start');await waitFor(s=>!s.active&&s.stockPrefix===0,'pristine stock');
  await control('Play');const played=await waitFor(s=>s.stockPrefix>rough.stockPrefix,'play through both stages',120);record('playback crosses into V-bit',played.stockPrefix);
  const live=await state();if(live.controls.Pause)await control('Pause');await waitFor(s=>!s.active,'playback request finishes');
  await control('Prepare checked output');const prepared=await waitFor(s=>s.prepared&&!s.active,'checked retained output',120);record('checked output',prepared.preparedSha256);
  await evaluate('globalThis.showSaveFilePicker=async()=>{throw new Error("GUI2 injected denied destination")}');
  await control('Save checked bytes');await waitFor(s=>s.status.includes('denied destination'),'failed destination');
  await evaluate('globalThis.showSaveFilePicker=undefined');await control('Retry previous save');await waitFor(s=>s.status.includes('Download requested'),'exact-byte retry download');
  for(let i=0;i<100&&!existsSync(path.join(out,'sequence.ngc'));i++)await sleep(100);
  const gcode=readFileSync(path.join(out,'sequence.ngc'));if(createHash('sha256').update(gcode).digest('hex')!==prepared.preparedSha256)throw new Error('Downloaded program differs from checked bytes');record('downloaded exact checked bytes after failed save',gcode.length);
  await control('Save job');await waitFor(s=>s.status.includes('Download requested'),'job download');
  for(let i=0;i<100&&!existsSync(path.join(out,'carving.gui2.job.json'));i++)await sleep(100);
  const saved=readFileSync(path.join(out,'carving.gui2.job.json'),'utf8');const job=JSON.parse(saved);
  if(job.schema_version!==5||!job.machine_configuration||job.operations[0].settings.settings.endmill.cutting_feed_mm_min!==1900)throw new Error('Invalid saved job');
  const screenshot=await send('Page.captureScreenshot',{format:'png'});writeFileSync(path.join(out,'workspace.png'),Buffer.from(screenshot.data,'base64'));
  await control('Cutting');await edit('Maximum depth','-');await waitFor(s=>s.pending&&!s.current,'pending text invalidates output');
  await sleep(1600);await send('Page.reload');await waitFor(s=>s.controls?.['Restore draft'],'recoverable draft');
  await control('Restore draft');await waitFor(s=>s.pending&&s.job?.rawDepth==='-','raw draft restored');record('restart recovers partial input without artifact trust',await state());
  await evaluate(`(()=>{const transfer=new DataTransfer();transfer.items.add(new File([${JSON.stringify(saved)}],'saved.job.json',{type:'application/json'}));document.getElementById('cam').dispatchEvent(new DragEvent('drop',{dataTransfer:transfer,bubbles:true,cancelable:true}));})()`);
  await waitFor(s=>s.job?.depth===1.1&&s.job?.feed===1900&&!s.pending,'saved job reopens through drop');record('saved job reopened',await state());
  await control('Generate');await waitFor(s=>s.active,'live generation');await control('Cancel');await waitFor(s=>!s.active&&!s.current,'worker cancelled');record('real Worker termination',await state());
  await control('Generate');await waitFor(s=>s.current&&!s.active,'fresh worker regenerates',120);
  await control('Prepare checked output');await waitFor(s=>s.prepared&&!s.active,'fresh worker prepares retained plan',120);
  }
  if(problems.length)throw new Error('Browser errors: '+problems.join('\n'));
  writeFileSync(path.join(out,'evidence.json'),JSON.stringify({url:base,checks,consoleErrors:problems,note:'Real Chromium/WebGPU UI and WASM Worker. Denied save is injected; the fallback download is written and its actual bytes checked. Browser terminate call does not measure stopped CPU latency.'},null,2));
  console.log((process.argv.includes('--gui4')?'GUI4':process.argv.includes('--gui3')?'GUI3':'GUI2')+' browser workflow passed');
} catch(error) {const screenshot=await send('Page.captureScreenshot',{format:'png'});writeFileSync(path.join(out,'failure.png'),Buffer.from(screenshot.data,'base64'));console.error(error);console.error(await state());console.error(problems);process.exitCode=1;}
finally {
  await Promise.race([send('Browser.close'),sleep(1500)]);
  socket.close();browser.kill();await sleep(500);
  rmSync(profile,{recursive:true,force:true});
}
