import init, {compute, protocol} from '../pkg/cam_gui1.js';
const ready=init();
self.onmessage=async ({data})=>{
  await ready;
  if(data.protocol!==protocol()) throw new Error('Worker protocol mismatch');
  if(JSON.parse(data.request)==='Crash') throw new Error('Injected worker crash');
  const reply=compute(data.request);
  self.postMessage({protocol:protocol(),reply});
};
