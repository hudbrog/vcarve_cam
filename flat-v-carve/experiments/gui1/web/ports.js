// Small experimental platform glue. No CAM or stock algorithms.
let active;
const emit = event => globalThis.GUI1.receive_event(JSON.stringify(event));
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
  cancelWorker();
  const begin = performance.now();
  const worker = new Worker(new URL('worker.js', document.baseURI), {type:'module'});
  active = {worker,id};
  worker.onmessage = ({data}) => {
    if (active?.id !== id) return;
    worker.terminate(); active = undefined;
    const result = data.protocol === 'gui1-spike-1' ? JSON.parse(data.reply) : {Err:'Worker version mismatch'};
    emit({Computed: {id, elapsed_ms:performance.now()-begin, result}});
  };
  worker.onerror = event => {
    event.preventDefault();
    if (active?.id !== id) return;
    worker.terminate(); active = undefined;
    emit({Computed:{id,elapsed_ms:performance.now()-begin,result:{Err:`Compute worker failed: ${event.message}`}}});
  };
  worker.postMessage({protocol:'gui1-spike-1',request});
}
export function openFile(recovery) {
  const picker = document.createElement('input'); picker.type='file'; picker.accept='.json';
  picker.onchange = async () => {
    const file=picker.files[0]; if (!file) return;
    try {
      if (file.size > 8_000_000) throw new Error('Input exceeds 8 MB spike limit');
      const text=await file.text();
      // Rust validates recovery shape/version as well; JSON is never interpreted as instructions.
      if (recovery) {
        emit({Recovered:{Ok:JSON.parse(globalThis.GUI1.validate_recovery(text))}});
      } else emit({Opened:{Ok:text}});
    } catch (e) { emit({Saved:{Err:String(e)}}); }
  };
  picker.oncancel=()=>emit({Saved:{Err:'Open cancelled; current draft retained'}});
  picker.click();
}
export function saveFile(name, bytes, deny) {
  if (deny) { emit({Saved:{Err:'Injected quota/denied write; bytes and draft retained for retry'}}); return; }
  const blob = new Blob([bytes.slice()], {type:'application/octet-stream'});
  const url=URL.createObjectURL(blob); const anchor=document.createElement('a');
  anchor.href=url;anchor.download=name;anchor.click();
  setTimeout(()=>URL.revokeObjectURL(url),30_000);
  emit({Saved:{Ok:`Download requested (${bytes.length} bytes); browser cannot confirm disk write`}});
}
