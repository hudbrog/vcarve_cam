// Run after the native release build. Evidence stays separate from source fixtures.
import {spawnSync,spawn} from 'node:child_process';
import {readFileSync,writeFileSync,mkdirSync,statSync} from 'node:fs';
import {createHash} from 'node:crypto';
import {fileURLToPath} from 'node:url';
import path from 'node:path';
const root=fileURLToPath(new URL('..',import.meta.url));
const out=path.join(root,'artifacts');mkdirSync(out,{recursive:true});
const exe=path.join(root,'target/release/cam-gui1-desktop.exe');
const samples=[];
for(let i=0;i<6;i++){
 const run=spawnSync(exe,['--capture',out,'small',...(i===5?['export']:[])],{windowsHide:true,timeout:120_000});
 if(run.error||run.status!==0) throw new Error(`Small reference failed: ${run.error??run.status}`);
 const report=JSON.parse(readFileSync(path.join(out,'small-reference.json')));samples.push({export:i===5,elapsedMs:report.elapsedMs});
}
const flower=spawnSync(exe,['--capture',out,'flower'],{windowsHide:true,timeout:240_000});
if(flower.error||flower.status!==0) throw new Error(`Flower reference failed: ${flower.error??flower.status}`);
const files=['../../../../real_data/flower_box-svg.job-real.json','../../../../real_data/flower_box.svg','../../../../real_data/machine-profile.json','../../../fixtures/m4/contact-line.json','../../../web/src/sim/engine.ts','../../../web/src/sim/setup.ts'];
const repo=fileURLToPath(new URL('../../../../',import.meta.url));
const inputs=files.map(relative=>{const file=new URL(relative,import.meta.url),bytes=readFileSync(file);return {path:path.relative(repo,fileURLToPath(file)).replaceAll('\\','/'),bytes:bytes.length,sha256:createHash('sha256').update(bytes).digest('hex')};});
const request=path.join(out,'busy-request.json'),reply=path.join(out,'busy-reply.json');writeFileSync(request,JSON.stringify('Busy'));
const cancellation=[];
for(let i=0;i<5;i++){
 const child=spawn(exe,['--worker',request,reply],{windowsHide:true,stdio:'ignore'});
 await new Promise((resolve,reject)=>{child.once('spawn',resolve);child.once('error',reject);});
 await new Promise(r=>setTimeout(r,150));
 const begin=performance.now();
 const closed=new Promise(resolve=>child.once('exit',resolve));child.kill();await closed;
 cancellation.push(performance.now()-begin);
}
writeFileSync(path.join(out,'measurements.json'),JSON.stringify({nativeExecutableBytes:statSync(exe).size,inputs,samples,cancellationKillToExitMs:cancellation,
 note:'CPU process kill-to-exit probe; UI input-to-visible, actual simulator, GPU/JS/WASM peak memory and sustained M budget are not qualified.'},null,2));
console.log(JSON.stringify({samples,cancellation},null,2));
