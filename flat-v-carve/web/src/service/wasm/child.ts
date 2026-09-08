// Disposable compute child: loads the engine once, answers exactly one
// computation per message, and is terminated by the parent to cancel.
import init, * as core from '../../wasm/gen/cam_wasm';

export type ChildKind = 'plan' | 'verify' | 'export';

const ready = init();

self.addEventListener('message', async event => {
  const { id, kind, input } = event.data as { id: string; kind: ChildKind; input: unknown };
  try {
    await ready;
    if (kind !== 'plan' && kind !== 'verify' && kind !== 'export') {
      throw new Error(`unknown computation kind ${String(kind)}`);
    }
    const reply = kind === 'plan'
      ? core.plan(JSON.stringify(input))
      : kind === 'verify'
        ? core.verify(JSON.stringify(input))
        : core.export_linuxcnc(JSON.stringify(input));
    (self as unknown as Worker).postMessage({ id, reply: JSON.parse(reply) });
  } catch (error) {
    (self as unknown as Worker).postMessage({
      id,
      reply: { error: { code: 'COMPUTE_FAILURE', severity: 'error', stage: 'planning', message: String(error) } },
    });
  }
});
