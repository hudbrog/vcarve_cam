import init, {solve_sketch_request} from '../pkg/cam_gui.js';
const ready = init();
self.onmessage = async ({data}) => {
  try { await ready; self.postMessage(solve_sketch_request(data)); }
  catch (error) {
    const {id, revision} = JSON.parse(data);
    self.postMessage(JSON.stringify({id, revision, result:{Err:`Solver failed: ${error}`}}));
  }
};
