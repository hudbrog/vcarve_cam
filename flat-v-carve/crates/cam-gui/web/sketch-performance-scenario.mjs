import {performanceFixtures} from '../../../scripts/sketch-qualification-fixtures.mjs';
import {writeFileSync} from 'node:fs';
export async function sketchPerformanceScenario({send,state,record,out,path}) {
  const fixtures=performanceFixtures();
  const result=await send('Runtime.evaluate',{awaitPromise:true,returnByValue:true,expression:`(async()=>{
    const fixtures=${JSON.stringify(fixtures)},results=[];
    let beats=0,last=performance.now(),maxGapMs=0;
    const heartbeat=setInterval(()=>{const now=performance.now();maxGapMs=Math.max(maxGapMs,now-last);last=now;beats++;},16);
    const start=()=>new Worker(new URL('sketch-benchmark-worker.js',document.baseURI),{type:'module'});
    const request=(worker,model,id,raw)=>new Promise(resolve=>{
      const t=performance.now(),text=raw??JSON.stringify({id,revision:id,model,targets:[]});
      const timeout=setTimeout(()=>{worker.terminate();resolve({timeout:true,elapsedMs:performance.now()-t});},10000);
      worker.onmessage=({data})=>{clearTimeout(timeout);resolve({...data,elapsedMs:performance.now()-t,requestBytes:new TextEncoder().encode(text).length,replyBytes:data.reply?.length,reply:data.reply?JSON.parse(data.reply):null});};
      worker.onerror=event=>{event.preventDefault();clearTimeout(timeout);resolve({error:event.message,elapsedMs:performance.now()-t});};
      worker.postMessage(text);
    });
    try {
      for(const f of fixtures){
        const worker=start(),cold=await request(worker,f.model,1);
        const row={name:f.name,entities:f.model.entities.length,constraints:f.model.constraints.length,coldMs:cold.elapsedMs,accepted:cold.reply?.result?.Ok?.accepted??false,requestBytes:cold.requestBytes,replyBytes:cold.replyBytes,warmMs:[],computeMs:[],linearMemoryBytes:cold.linearMemoryBytes};
        if(row.accepted){
          let model=cold.reply.result.Ok.candidate;
          for(let i=1;i<=21;i++){
            model.constraints.find(c=>c.id==='r0-width').definition.value_mm=24+i*.02;
            const r=await request(worker,model,i+1);
            if(!r.reply?.result?.Ok?.accepted){row.failure=r;break;}
            model=r.reply.result.Ok.candidate;row.warmMs.push(r.elapsedMs);row.computeMs.push(r.computeMs);row.linearMemoryBytes=r.linearMemoryBytes;
          }
          if(row.warmMs.length){const ordered=[...row.warmMs].sort((a,b)=>a-b);row.p50Ms=ordered[Math.ceil(ordered.length*.5)-1];row.p95Ms=ordered[Math.ceil(ordered.length*.95)-1];}
        }else row.failure=cold;
        worker.terminate();results.push(row);
      }
      const worker=start(),oversize=await request(worker,null,999,'x'.repeat(2500001));
      const retry=await request(worker,fixtures[0].model,1000);worker.terminate();
      // A posted request is disposable even during WASM startup.
      const cancelled=start();cancelled.postMessage(JSON.stringify({id:1001,revision:1001,model:fixtures[2].model,targets:[]}));
      const stop=performance.now();cancelled.terminate();const terminateCallMs=performance.now()-stop;
      return {fixtures:results,oversize:{error:oversize.error,elapsedMs:oversize.elapsedMs},retry:{accepted:retry.reply?.result?.Ok?.accepted??false,elapsedMs:retry.elapsedMs},terminateCallMs,heartbeat:{beats,maxGapMs},userAgent:navigator.userAgent,hardwareConcurrency:navigator.hardwareConcurrency,notes:['Wall time includes postMessage and JSON; compute time includes production solve entry point and serialization.','Linear memory is WASM reserved memory, not total browser/Worker process RSS.','Worker cold time uses the browser HTTP/WASM cache; it is not an uncached network-load measurement.','Termination timing measures the API call, not stopped CPU latency.']};
    }finally{clearInterval(heartbeat);}
  })()`});
  if(result.exceptionDetails)throw Error(JSON.stringify(result.exceptionDetails));
  const evidence=result.result.value;
  writeFileSync(path.join(out,'sketch-performance.json'),JSON.stringify(evidence,null,2)+'\n');
  for(const row of evidence.fixtures)record(`${row.name}: cold ${row.coldMs.toFixed(1)} ms, warm p95 ${row.p95Ms?.toFixed(1)} ms`,row);
  if(evidence.fixtures.some(f=>f.failure||!f.accepted)||!evidence.retry.accepted||!evidence.oversize.error)throw Error('Sketch performance or bounded failure qualification failed');
  if(!(await state()).gui2)throw Error('Main app stopped responding during solver work');
  record('browser limits, retry and main-thread responsiveness',evidence.heartbeat);
}
