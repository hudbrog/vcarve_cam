import {runPlane,loadPlane} from './plane.mjs';
let arael,plane,araelWasm;
onmessage=async ({data})=>{
  const send=message=>postMessage({...message,request:data.request,document_revision:data.document_revision,sketch_revision:data.sketch_revision});
  try {
    if(data.backend==='arael') {
      arael??=await import('./pkg/sketcher_feasibility.js');
      araelWasm=await arael.default();
    } else {plane??=await loadPlane(new URL('/plane/planegcs.js',location.href).href);}
    send({kind:'ready'});
    if(data.geometry){send({kind:'geometry',result:JSON.parse(arael.geometry_probe())});}
    for(const fixture of data.fixtures) {
      send({kind:'solving',id:fixture.id});
      const result=data.backend==='arael'?JSON.parse(arael.run(JSON.stringify([fixture])))[0]:runPlane(plane,fixture);
      if(data.backend==='arael')result.heap_bytes=araelWasm.memory.buffer.byteLength;
      if(data.delay_ms)await new Promise(resolve=>setTimeout(resolve,data.delay_ms));
      send({kind:'result',result});
    }
    send({kind:'done'});
  }catch(e){send({kind:'error',error:String(e),stack:e.stack});}
};
