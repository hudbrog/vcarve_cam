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
    const keyCode=({Escape:27,Backspace:8,Enter:13,Tab:9})[key]??key.toUpperCase().charCodeAt(0);
    await send('Input.dispatchKeyEvent', {type, key, code, windowsVirtualKeyCode:keyCode, nativeVirtualKeyCode:keyCode, modifiers});
  }
};

const control = async label => {
  for(let attempt=0;attempt<12;attempt++) {
    const current=await state(); const rect=current.controls?.[label];
    const libraryMenus={
      'Import library':'Library actions','Export library':'Library actions','Import machine configuration':'Library actions',
      'Load library':'Library actions','Compare stored revision':'Library actions','Reload stored library':'Library actions','Overwrite reviewed revision':'Library actions',
      'Duplicate library tool':'Library item actions','Duplicate machine configuration':'Library item actions','Add geometry to job':'Library item actions',
      'New endmill':'New tool','New V-bit':'New tool','Capture job geometry':'New tool',
      'New machine profile':'New machine','New machine ID':'New machine','Capture applied machine':'New machine'
    };
    const dropdown=['Library rotation','Library plunge','Library ramp','Copied plunge','Copied ramp','Work offset','Length compensation','Coolant','Path control','M6 return'].find(prefix=>label.startsWith(prefix+' '));
    if(!rect){
      if(current.resources?.open&&libraryMenus[label]){await control(libraryMenus[label]);continue;}
      if((current.resources?.open||current.resources?.jobsOpen)&&dropdown){await control(dropdown);continue;}
      if(current.resources?.open&&['Path control','Coolant','Configuration blend tolerance','Configuration naive CAM tolerance'].includes(label)){await control('Motion & coolant');continue;}
      if(current.resources?.open&&['Reapply reviewed profile','Reset assignment overrides'].includes(label)){await control('Applied job values');continue;}
      throw new Error(`Missing control ${label}`);
    }
    const nav=label.startsWith('Artwork ') || ['Setup','Machine','Job settings','Artwork','Cutting','Inspect result','Endmill tool','V-bit tool','+ Import artwork','Tool library','Job tools'].includes(label);
    const resource=!nav&&current.resources?.open,jobTools=!nav&&current.resources?.jobsOpen;
    const list=resource&&label!=='Library tool name'&&(label.startsWith('Library tool ')||label.startsWith('Library machine '));
    const clip=current.controls?.[nav?'Navigator viewport':list?'Resource list viewport':resource?'Resource viewport':jobTools?'Job tools viewport':'Inspector viewport'];
    const bottom=(clip?.[3]??await evaluate('innerHeight-65'))+1;
    const top=(clip?.[1]??150)-1;
    if(rect[1]>=top && rect[3]<=bottom || !nav && !resource && !jobTools && rect[0]<(clip?.[0]??850) || libraryMenus[label] || dropdown && (resource||jobTools) || ['Library actions','Library search','New tool','New machine','Use machine','Use tool','Use tool & profile','Apply reviewed machine','Tools & profiles','Machines','Close library','Load library','Save library','Compare stored revision','Reload stored library','Overwrite reviewed revision','Import library','Export library','Import machine configuration','Close job tools','Roughing assignment','Finishing assignment','Filter fields','File','Generate','Prepare','Simulate','Export…','Prepare checked output','Save job','Undo','Redo','Cancel','Restore draft','Retry previous save'].includes(label)) {
      await click((rect[0]+rect[2])/2,(rect[1]+rect[3])/2); await sleep(120);return;
    }
    const x=nav?100:clip?(clip[0]+clip[2])/2:1100,y=(top+bottom)/2;
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
  if(process.argv.includes('--trace-io'))await evaluate(`(()=>{globalThis.GUI_IO_TRACE=[];const original=globalThis.CAM_GUI.receive_event;globalThis.CAM_GUI.receive_event=text=>{try{const event=JSON.parse(text);if(event.Io)globalThis.GUI_IO_TRACE.push(event.Io);}catch{}return original(text);};})()`);
  await send('Browser.setDownloadBehavior',{behavior:'allow',downloadPath:out});
  if(process.argv.includes('--gui5')) {
    const {gui5Scenario}=await import('./gui5-scenario.mjs');
    await gui5Scenario({control,edit,state,waitFor,send,evaluate,sleep,record,screenshot,readFileSync,click,pressKey,path,out,chooseFile});
  } else if(process.argv.includes('--gui4')) {
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
  const exportTab=(await state()).workspace.inspector;
  await control('Prepare checked output');
  if(!(await state()).controls['Export dialog'])throw new Error('Export did not immediately open a dialog');
  await screenshot('export-progress.png');
  const prepared=await waitFor(s=>s.prepared&&!s.active,'checked retained output',120);record('checked output',prepared.preparedSha256);
  if(prepared.workspace.inspector!==exportTab)throw new Error('Export changed the inspector');
  await screenshot('export-ready.png');
  await send('Emulation.setDeviceMetricsOverride',{width:900,height:700,deviceScaleFactor:1,mobile:false});await sleep(700);
  const exportControls=(await state()).controls;
  if(exportControls['Export dialog'][3]>700||exportControls['Save as…'][3]>700)throw new Error('Export actions do not fit a compact window');
  await screenshot('export-compact.png');await send('Emulation.clearDeviceMetricsOverride');await sleep(700);
  await evaluate('globalThis.showSaveFilePicker=async()=>{throw new Error("GUI2 injected denied destination")}');
  await control('Save as…');await waitFor(s=>s.status.includes('denied destination'),'failed destination');
  await screenshot('export-save-error.png');
  await evaluate('globalThis.showSaveFilePicker=undefined');await control('Save as…');await waitFor(s=>s.status.includes('Download requested'),'exact-byte retry download');
  for(let i=0;i<100&&!existsSync(path.join(out,'sequence.ngc'));i++)await sleep(100);
  const gcode=readFileSync(path.join(out,'sequence.ngc'));if(createHash('sha256').update(gcode).digest('hex')!==prepared.preparedSha256)throw new Error('Downloaded program differs from checked bytes');record('downloaded exact checked bytes after failed save',gcode.length);
  await control('Save job');await waitFor(s=>s.status.includes('Download requested'),'job download');
  for(let i=0;i<100&&!existsSync(path.join(out,'carving.gui2.job.json'));i++)await sleep(100);
  const saved=readFileSync(path.join(out,'carving.gui2.job.json'),'utf8');const job=JSON.parse(saved);
  if(job.schema_version!==5||!job.machine_configuration||job.operations[0].settings.settings.endmill.cutting_feed_mm_min!==1900)throw new Error('Invalid saved job');
  await screenshot('workspace.png');
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
  console.log((process.argv.includes('--gui5')?'GUI5':process.argv.includes('--gui4')?'GUI4':process.argv.includes('--gui3')?'GUI3':'GUI2')+' browser workflow passed');
} catch(error) {const screenshot=await send('Page.captureScreenshot',{format:'png'});writeFileSync(path.join(out,'failure.png'),Buffer.from(screenshot.data,'base64'));writeFileSync(path.join(out,'failure-state.json'),JSON.stringify(await state(),null,2));if(process.argv.includes('--trace-io'))writeFileSync(path.join(out,'io-trace.json'),JSON.stringify(await evaluate('globalThis.GUI_IO_TRACE'),null,2));console.error(error);console.error(await state());console.error(problems);process.exitCode=1;}
finally {
  await Promise.race([send('Browser.close'),sleep(1500)]);
  socket.close();browser.kill();await sleep(500);
  rmSync(profile,{recursive:true,force:true});
}
