// Platform glue. No CAM or stock algorithms.
let active;
const emit = event => globalThis.CAM_GUI.receive_event(JSON.stringify(event));
export function cancelWorker() {
  if (active) {
    const {worker, id} = active;
    const start = performance.now();
    worker.terminate(); active = undefined;
    // API return latency only: browsers do not expose actual stopped CPU confirmation.
    emit({Cancelled: {id, stop_ms: performance.now()-start}});
  }
}
export function startWorker(id, request) {
  const begin = performance.now();
  const worker = active?.worker ?? new Worker(new URL('worker.js', document.baseURI), {type:'module'});
  active = {worker,id};
  worker.onmessage = ({data}) => {
    if (active?.id !== id) return;
    if (data.protocol !== 'cam-gui-retained-1' || data.gui2Protocol !== 'gui2-retained-1') {
      cancelWorker();
      emit({Computed:{id,elapsed_ms:performance.now()-begin,result:{Err:'Worker version mismatch; reload matching assets'}}});return;
    }
    const payload = data.payload ? new Uint8Array(data.payload) : new Uint8Array(0);
    globalThis.CAM_GUI.receive_payload(payload);
    globalThis.CAM_GUI.receive_event(JSON.stringify({ComputedBinary:{id,elapsed_ms:performance.now()-begin,meta:JSON.parse(data.meta)}}));
  };
  worker.onerror = event => {
    event.preventDefault();if(active?.id!==id)return;
    worker.terminate();active=undefined;
    emit({Computed:{id,elapsed_ms:performance.now()-begin,result:{Err:'Compute worker failed: '+event.message}}});
  };
  worker.postMessage({protocol:'cam-gui-retained-1',request});
}
export function openFile(id,recovery) {
  const complete=result=>emit({Io:{id,result}});
  const picker = document.createElement('input'); picker.type='file'; picker.accept='.json';
  picker.onchange = async () => {
    const file=picker.files[0]; if (!file) {complete({Err:'Open cancelled'});return;}
    try {
      if (file.size > 8_000_000) throw new Error('Input exceeds 8 MB limit');
      const text=await file.text();
      // Rust validates recovery shape/version as well; JSON is never interpreted as instructions.
      if (recovery) {
        complete({Ok:{Draft:JSON.parse(globalThis.CAM_GUI.validate_recovery(text))}});
      } else complete({Ok:{Job:text}});
    } catch (e) { complete({Err:String(e)}); }
  };
  picker.oncancel=()=>complete({Err:'Open cancelled; current draft retained'});
  try{picker.click();}catch(error){complete({Err:String(error)});}
}
export async function saveFile(id,name,bytes,deny) {
  const complete=result=>emit({Io:{id,result}});
  if(deny){complete({Err:'Injected denied write; exact bytes retained for retry'});return;}
  // Copy before the first await: this view belongs to Rust WASM memory.
  const retained=bytes.slice();
  try {
    if(typeof globalThis.showSaveFilePicker==='function') {
      const handle=await globalThis.showSaveFilePicker({suggestedName:name});
      const stream=await handle.createWritable();
      try {await stream.write(retained);await stream.close();}catch(error){try{await stream.abort();}catch{}throw error;}
      const saved=await handle.getFile();
      if(saved.size!==retained.length)throw new Error('Saved-byte length mismatch');
      const actual=new Uint8Array(await saved.arrayBuffer());
      if(actual.length!==retained.length||actual.some((byte,i)=>byte!==retained[i]))throw new Error('Saved-byte readback mismatch');
      const digest=new Uint8Array(await crypto.subtle.digest('SHA-256',actual));
      complete({Ok:{Saved:`Saved ${actual.length} exact bytes; SHA256 ${Array.from(digest,b=>b.toString(16).padStart(2,'0')).join('')}`}});
    }else{
      const url=URL.createObjectURL(new Blob([retained],{type:'application/octet-stream'}));const anchor=document.createElement('a');
      anchor.href=url;anchor.download=name;anchor.click();setTimeout(()=>URL.revokeObjectURL(url),30_000);
      complete({Ok:{Saved:`Download requested (${retained.length} bytes); browser cannot confirm disk write`}});
    }
  }catch(error){complete({Err:String(error)});}
}
const recoveryStore=()=>import(new URL('recovery-store.js',document.baseURI).href);
export async function loadRecovery() {
  try {
    const text=await (await recoveryStore()).loadRecord();
    emit({RecoveryLoaded:{Ok:text===null?null:JSON.parse(globalThis.CAM_GUI.validate_session(text))}});
  }catch(error){emit({RecoveryLoaded:{Err:String(error)}});}
}
export async function saveRecovery(edit,expected,snapshot) {
  try {
    const value=JSON.parse(snapshot);const previous=JSON.parse(expected);
    globalThis.CAM_GUI.validate_session(JSON.stringify({revision:(previous??0)+1,snapshot:value}));
    const revision=await (await recoveryStore()).writeRecord(previous,value);
    emit({RecoverySaved:{edit,result:{Ok:revision}}});
  }catch(error){emit({RecoverySaved:{edit,result:{Err:String(error)}}});}
}
