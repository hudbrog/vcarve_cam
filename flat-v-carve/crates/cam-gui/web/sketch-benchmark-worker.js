// Qualification-only instrumentation around the production WASM entry point.
// Not included in the distributed browser artifact or used by the editor.
import init, {solve_sketch_request} from '../pkg/cam_gui.js';
const ready=init();
self.onmessage=async ({data})=>{
  const wasm=await ready,start=performance.now();
  try {
    const reply=solve_sketch_request(data);
    self.postMessage({reply,computeMs:performance.now()-start,linearMemoryBytes:wasm.memory.buffer.byteLength});
  } catch(error) {
    self.postMessage({error:String(error),computeMs:performance.now()-start,linearMemoryBytes:wasm.memory.buffer.byteLength});
  }
};
