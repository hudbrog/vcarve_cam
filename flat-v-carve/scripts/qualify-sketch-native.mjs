// Real release executable, production solver mailbox, bounded supervisor.
// UI acceptance is a separate, mandatory normal-app workflow.
import {spawn,execFileSync} from 'node:child_process';
import {mkdirSync,writeFileSync,readFileSync,existsSync,renameSync,unlinkSync,rmdirSync} from 'node:fs';
import {cpus,platform,release,totalmem} from 'node:os';
import {createHash} from 'node:crypto';
import path from 'node:path';
import {performanceFixtures,resize,percentile} from './sketch-qualification-fixtures.mjs';

const exe=path.resolve(process.argv[2]??'artifacts/gui/native/cam-gui.exe');
const out=path.resolve(process.argv[3]??'artifacts/sketch-m5/native-performance.json');
mkdirSync(path.dirname(out),{recursive:true});
const sleep=ms=>new Promise(r=>setTimeout(r,ms));
const evidence={timestamp:new Date().toISOString(),exe,sha256:createHash('sha256').update(readFileSync(exe)).digest('hex'),host:{os:platform(),release:release(),cpu:cpus()[0].model,logicalCpus:cpus().length,ramBytes:totalmem()},notes:['Wall times include JSON serialization and native mailbox I/O.','Memory is the isolated solver child working set, not the GUI process.','100 entities have 200 explicit constraints: 175 driving, 25 reference, plus 100 implicit wire joins.'],fixtures:[]};
let sequence=0;
function start() {
  const folder=path.resolve('artifacts/sketch-m5',`mailbox-${process.pid}-${sequence++}`);mkdirSync(folder,{recursive:true});
  const child=spawn(exe,['--sketch-worker',folder],{stdio:'ignore',windowsHide:true});
  return {folder,child};
}
async function stop(w) {
  if(w.child.exitCode===null&&w.child.signalCode===null){w.child.kill();await new Promise(r=>w.child.once('exit',r));}
  for(const name of ['pending','request','result','response']){const file=path.join(w.folder,name);if(existsSync(file))unlinkSync(file);}
  if(existsSync(w.folder))rmdirSync(w.folder);
}
async function request(w,model,id,limitMs=10000,raw) {
  const start=performance.now(),text=raw??JSON.stringify({id,revision:id,model,targets:[]});
  writeFileSync(path.join(w.folder,'pending'),text);renameSync(path.join(w.folder,'pending'),path.join(w.folder,'request'));
  while(!existsSync(path.join(w.folder,'response'))) {
    if(w.child.exitCode!==null)return {elapsedMs:performance.now()-start,exitCode:w.child.exitCode};
    if(performance.now()-start>limitMs){await stop(w);return {elapsedMs:performance.now()-start,timeout:true};}
    await sleep(2);
  }
  const bytes=readFileSync(path.join(w.folder,'response'));unlinkSync(path.join(w.folder,'response'));
  return {elapsedMs:performance.now()-start,requestBytes:Buffer.byteLength(text),replyBytes:bytes.length,reply:JSON.parse(bytes)};
}
function memory(w) {
  const command=`$p=Get-Process -Id ${w.child.pid}; @{workingSetBytes=$p.WorkingSet64; peakWorkingSetBytes=$p.PeakWorkingSet64; cpuSeconds=$p.TotalProcessorTime.TotalSeconds} | ConvertTo-Json -Compress`;
  return JSON.parse(execFileSync('powershell.exe',['-NoProfile','-Command',command],{encoding:'utf8',windowsHide:true}));
}
for(const f of performanceFixtures()) {
  const w=start(),cold=await request(w,f.model,1);
  const row={name:f.name,entities:f.model.entities.length,constraints:f.model.constraints.length,coldMs:cold.elapsedMs,accepted:cold.reply?.result?.Ok?.accepted??false,requestBytes:cold.requestBytes,replyBytes:cold.replyBytes,warmMs:[]};
  if(!row.accepted){row.failure=cold.reply??cold;evidence.fixtures.push(row);if(!cold.timeout)await stop(w);continue;}
  let model=cold.reply.result.Ok.candidate;
  for(let i=1;i<=21;i++) {
    const r=await request(w,resize(model,i),i+1);
    if(!r.reply?.result?.Ok?.accepted){row.failure=r.reply??r;break;}
    model=r.reply.result.Ok.candidate;row.warmMs.push(r.elapsedMs);
  }
  if(w.child.exitCode===null&&w.child.signalCode===null)row.memory=memory(w);
  if(row.warmMs.length){row.p50Ms=percentile(row.warmMs,.5);row.p95Ms=percentile(row.warmMs,.95);}
  evidence.fixtures.push(row);console.log(`${row.name}: cold=${row.coldMs.toFixed(1)}ms p95=${row.p95Ms?.toFixed(1)}ms`);
  if(!row.failure?.timeout)await stop(w);
}
// Transport failure must be bounded, and a new real process must accept a retry.
const bad=start();const badResult=await request(bad,null,1,10000,'x'.repeat(2500001));
evidence.oversize={...badResult,reply:undefined};if(!badResult.timeout)await stop(bad);
const cancelled=start();
await request(cancelled,performanceFixtures()[0].model,98);
const large=JSON.stringify({id:99,revision:99,model:resize(performanceFixtures()[2].model,1),targets:[]});
writeFileSync(path.join(cancelled.folder,'pending'),large);renameSync(path.join(cancelled.folder,'pending'),path.join(cancelled.folder,'request'));
const consumptionStart=performance.now();
while(existsSync(path.join(cancelled.folder,'request'))&&performance.now()-consumptionStart<5000)await sleep(2);
evidence.cancelledWhileActive=!existsSync(path.join(cancelled.folder,'request'))&&!existsSync(path.join(cancelled.folder,'response'));
const beforeStop=performance.now();await stop(cancelled);evidence.cancelStopMs=performance.now()-beforeStop;
const retry=start();const retried=await request(retry,performanceFixtures()[0].model,100);
evidence.retry={accepted:retried.reply?.result?.Ok?.accepted??false,elapsedMs:retried.elapsedMs};await stop(retry);
writeFileSync(out,JSON.stringify(evidence,null,2)+'\n');
if(evidence.fixtures.some(f=>f.failure||!f.accepted)||!evidence.retry.accepted||badResult.exitCode!==2)process.exitCode=1;
console.log(out);
