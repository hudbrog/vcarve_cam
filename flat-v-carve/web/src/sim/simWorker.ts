// Simulator worker shell (U8 §5.4): receives the transferred compact store,
// creates the seek session, and answers seeks with dirty-tile deltas. All
// state machine logic lives in session.ts and store.ts, which vitest covers
// without a Worker.
import { normalizeTool } from './engine';
import { SimulationSession } from './session';
import type { InitMessage, SeekMessage } from './protocol';

let session: SimulationSession | null = null;

self.onmessage = (event: MessageEvent) => {
  const data = event.data;
  try {
    if (data?.type === 'init') {
      const message = data as InitMessage;
      session = new SimulationSession(
        message.stock,
        message.tools.map(normalizeTool),
        message.resolution,
        message.store,
      );
      postMessage({ type: 'ready' });
      return;
    }
    if (data?.type === 'seek') {
      if (session === null) throw new Error('simulator worker received a seek before init');
      const message = data as SeekMessage;
      const result = session.seek(message.index, message.fraction);
      postMessage({
        type: 'seekDone',
        applied: result.applied,
        fraction: result.fraction,
        tiles: result.tiles,
        stats: result.stats,
      });
      return;
    }
    throw new Error(`simulator worker received an unknown message type '${String(data?.type)}'`);
  } catch (error) {
    postMessage({ type: 'error', message: error instanceof Error ? error.message : String(error) });
  }
};
