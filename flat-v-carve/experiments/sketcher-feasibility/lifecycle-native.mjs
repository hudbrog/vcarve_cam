import {spawn} from 'node:child_process';
import {readFile,writeFile} from 'node:fs/promises';
import {performance} from 'node:perf_hooks';
const root=new URL('../../artifacts/sketcher-feasibility/',import.meta.url);
const fixture=JSON.parse(await readFile(new URL('./fixtures.json',import.meta.url))).find(f=>f.id==='scale-100');
fixture.samples=10000;
const input=new URL('cancellation.json',root),output=new URL('cancelled-output.json',root);
await writeFile(input,JSON.stringify([fixture]));
const {fileURLToPath}=await import('node:url');
const results=[];
for(const [backend,exe] of [['arael',new URL('./target/release/sketch-probe.exe',import.meta.url)],['planegcs',new URL('native-plane/plane-probe.exe',root)]]) {
  const start=performance.now();
  const child=spawn(fileURLToPath(exe),[fileURLToPath(input),fileURLToPath(output)],{windowsHide:true,stdio:'ignore'});
  let killed=false;
  const timer=setTimeout(()=>{killed=child.kill();},75);
  await new Promise((resolve,reject)=>{child.on('error',reject);child.on('exit',resolve);});clearTimeout(timer);
  if(!killed)throw Error(`${backend}: cancellation fixture ended before kill`);
  results.push({backend,process_killed:killed,elapsed_ms:performance.now()-start});
}
await writeFile(new URL('native-lifecycle.json',root),JSON.stringify({scope:'helper-process termination; not cooperative in-process cancellation',results},null,2));
console.log(results);
