// GUI9a larger jobs in a real browser: open the six-copy batch fixture through
// the shipped file drop, generate it in the WASM Worker (a plan the previous
// build refused above 100,000 motions), scrub it, and read the display's own
// accounting for the boundary it just crossed.
export async function gui9Scenario({control,edit,state,waitFor,send,evaluate,sleep,record,screenshot,readFileSync,pressKey,click,path,out}) {
  // Dropping a portable job is the documented browser route for an existing
  // document (GUI4/GUI7 use the same helper shape).
  const batch = readFileSync('fixtures/gui9/flower-box-batch.job.json', 'utf8');
  await evaluate(`(()=>{const t=new DataTransfer();t.items.add(new File([${JSON.stringify(batch)}],'flower-box-batch.job.json',{type:'application/json'}));document.getElementById('cam').dispatchEvent(new DragEvent('drop',{dataTransfer:t,bubbles:true,cancelable:true}));})()`);
  const opened = await waitFor(
    s => s.job?.operations?.length === 6 && !s.active,
    'larger batch job',
    240,
  );
  if (!opened.job.operations.every(operation => operation.kind === 'flat_vcarve'))
    throw new Error(`batch operations: ${JSON.stringify(opened.job.operations.map(operation => operation.kind))}`);
  if (opened.job.stock.xy.width_mm !== 620 || opened.job.stock.xy.length_mm !== 210)
    throw new Error(`batch stock rectangle: ${JSON.stringify(opened.job.stock)}`);
  record('larger fixture opened', {
    operations: opened.job.operations.length,
    stock: opened.job.stock.xy,
    previousMotionLimit: 100000,
  });
  await screenshot('gui9-batch-opened.png');

  // The batch needs the reviewed machine configuration only for checked output;
  // generation itself does not depend on it, but the review tour does.
  await control('Machine');
  await control('Example machine');
  await control('Apply flower machine profile');
  await waitFor(s => !s.active && s.job.machine, 'applied machine profile', 240);

  await control('Cutting');
  await control('Generate');
  // While that generation runs, the viewer keeps working: the camera responds
  // and a document edit lands. The running result must not replace the newer
  // edit, so it is discarded and the job is generated again below.
  await sleep(250);
  const busy = await state();
  if (busy.active !== true)
    throw new Error('the batch generation was not running when it was sampled');
  await control('Isometric');
  await edit('Roughing feed', '1850');
  const edited = await waitFor(
    s => s.revision > busy.revision,
    'document edit during the generation',
    120,
  );
  const settled = await waitFor(s => !s.active, 'generation settled', 1200);
  if (edited.workspace.view.isometric !== true)
    throw new Error('the camera change made during the generation was lost');
  if (settled.current)
    throw new Error('a stale generation result replaced the newer edit');
  record('edited while generating', {
    busyDuringEdit: true,
    revisionBefore: busy.revision,
    revisionAfter: settled.revision,
    staleResultKeptOut: settled.current === false,
    cameraKept: settled.workspace.view.isometric,
    status: settled.status,
  });
  await screenshot('gui9c-edit-under-load.png');

  await control('Generate');
  const generated = await waitFor(
    s => s.current && !s.active && s.motions > 0,
    'larger batch generation',
    1200,
  );
  if (!generated.exportReady)
    throw new Error(`batch generation is not export-ready: ${generated.status}`);
  if (/100,000/.test(generated.status ?? ''))
    throw new Error(`the display still refuses the larger job: ${generated.status}`);
  if (generated.motions < 100000)
    throw new Error(`the batch fixture stopped being a larger job: ${generated.motions} motions`);
  record('larger generation admitted', {
    motions: generated.motions,
    revision: generated.revision,
  });
  await screenshot('gui9-batch-generated.png');

  // The playback bar only exists while simulating. It must lay out a bounded
  // stage row: six carvings give twelve stages, so eight rows plus scroll
  // controls — never twelve rows.
  await control('Simulate');
  const timeline = await waitFor(s => s.timelineRows === 8, 'bounded stage row', 60);
  await control('Start');
  await waitFor(s => !s.active && s.stockPrefix === 0, 'stock restored to the start', 300);
  // The final stage is outside the first window, so it is reached through the
  // window control rather than by laying out every stage.
  await control('Later stages');
  await waitFor(s => s.controls['After v-bit (6 of 6)'], 'later stages window', 60);
  await control('After v-bit (6 of 6)');
  const seeked = await waitFor(
    s => !s.active && s.stockPrefix === s.motions,
    'final stock prefix',
    600,
  );
  if (!seeked.stockTransferBytes)
    throw new Error('the display did not report the stock transfer it performed');
  record('windowed timeline and forward scrub', {
    rows: timeline.timelineRows,
    stages: timeline.controls['After v-bit (6 of 6)'] ? 12 : 'windowed',
    stockPrefix: seeked.stockPrefix,
    transferBytes: seeked.stockTransferBytes,
    replayedMotions: seeked.stockReplayed,
  });
  await screenshot('gui9-batch-final-stock.png');

  // --- GUI9b: another display resolution, same execution, same playhead -----
  const before = await state();
  await control('Display resolution');
  await control('Display resolution Fine');
  const fine = await waitFor(
    s => !s.active && s.display?.preset === 'fine',
    'finer display raster',
    240,
  );
  if (!(fine.display.cellMm < before.display.cellMm))
    throw new Error(
      `the finer preset did not resolve smaller cells: ${before.display.cellMm} -> ${fine.display.cellMm}`,
    );
  if (fine.display.key === before.display.key)
    throw new Error('the rebuild kept the old simulation key');
  if (fine.stockPrefix !== before.stockPrefix)
    throw new Error(
      `the rebuild moved the playhead: ${before.stockPrefix} -> ${fine.stockPrefix}`,
    );
  if (!(fine.display.retainedBytes > before.display.retainedBytes))
    throw new Error('the finer raster did not report its larger checkpoint budget');
  record('display rebuilt at another resolution', {
    from: before.display.preset,
    to: fine.display.preset,
    cellMmBefore: before.display.cellMm,
    cellMmAfter: fine.display.cellMm,
    referenceCellMm: fine.display.referenceCellMm,
    checkpoints: fine.display.checkpoints,
    retainedBytes: fine.display.retainedBytes,
    simulationKey: fine.display.key,
    stockPrefix: fine.stockPrefix,
  });
  await screenshot('gui9b-fine-resolution.png');

  // The section is the display's own cross-section at the same playhead. Drive
  // the XY readout the inspector exposes, then read both axes.
  const setInspection = async (label, text) => {
    await control(label);
    await pressKey('a', 'KeyA', 2);
    await send('Input.insertText', {text});
    await sleep(200);
  };
  await setInspection('Inspect X', '310');
  await setInspection('Inspect Y', '105');
  const section = await waitFor(
    s => (s.display?.sectionSamples ?? 0) > 0,
    'raster section at the inspected point',
    60,
  );
  if (!section.controls['Section plot']) throw new Error('the section plot is not laid out');
  await control('Section Y');
  const across = await waitFor(
    s => (s.display?.sectionSamples ?? 0) > 0,
    'section along Y',
    60,
  );
  if (across.display.sectionSamples === section.display.sectionSamples)
    throw new Error('both section axes reported the same sample count');
  record('raster section at the inspected point', {
    alongX: section.display.sectionSamples,
    alongY: across.display.sectionSamples,
    cellMm: across.display.cellMm,
  });
  await screenshot('gui9b-section.png');

  // --- GUI9c: bounded uploads, a renderer drill, and editing under load -----
  // The bounded upload finishes over several frames; the probe reports how many
  // frames still had work outstanding.
  const loaded = await waitFor(
    s => s.renderer && s.renderer.pendingTiles === 0 && s.renderer.pagesDeferred === 0,
    'bounded upload finished',
    240,
  );
  record('bounded display upload', {
    framesLoading: loaded.renderer.framesLoading,
    stockFramesLoading: loaded.renderer.stockFramesLoading,
    residentPages: loaded.renderer.residentPages,
    residentBytes: loaded.renderer.residentBytes,
  });

  // A renderer failure and its recovery must leave the document, the retained
  // result and the view exactly as they were.
  const beforeDrill = await state();
  await control('Renderer diagnostics');
  await control('Inject renderer failure');
  const failed = await waitFor(s => s.renderer?.unavailable === true, 'renderer failure', 60);
  if (failed.stockPrefix !== beforeDrill.stockPrefix)
    throw new Error('the injected failure moved the playhead');
  if (failed.display.key !== beforeDrill.display.key)
    throw new Error('the injected failure changed the simulation key');
  if (failed.revision !== beforeDrill.revision)
    throw new Error('the injected failure changed the document revision');
  await screenshot('gui9c-failure.png');

  await control('Rebuild renderer resources');
  const recovered = await waitFor(
    s =>
      s.renderer?.unavailable === false &&
      s.renderer.recoveries > failed.renderer.recoveries,
    'renderer resources rebuilt',
    240,
  );
  const restored = await waitFor(
    s => s.renderer.pendingTiles === 0 && s.renderer.pagesDeferred === 0,
    'recovered display re-uploaded',
    240,
  );
  for (const [field, expected] of [
    ['stockPrefix', failed.stockPrefix],
    ['revision', failed.revision],
  ]) {
    if (restored[field] !== expected)
      throw new Error(`recovery changed ${field}: ${expected} -> ${restored[field]}`);
  }
  if (restored.display.key !== failed.display.key)
    throw new Error('recovery changed the simulation key');
  if (
    JSON.stringify(restored.workspace.view) !== JSON.stringify(failed.workspace.view)
  )
    throw new Error('recovery changed the saved view');
  record('renderer failure and recovery', {
    recoveries: restored.renderer.recoveries,
    playhead: restored.stockPrefix,
    preset: restored.display.preset,
    simulationKey: restored.display.key,
    revision: restored.revision,
    residentPages: restored.renderer.residentPages,
  });
  await screenshot('gui9c-recovered.png');

  // --- GUI9c: sustained scrubbing, and an idle display that copies nothing ---
  // Alternate between the start and the first stage end without waiting for
  // each seek: the display coalesces the requests and the UI keeps drawing.
  // The stage window has moved during the tour, so pick whichever stage jump
  // is laid out right now instead of assuming an index.
  const laidOut = (await state()).controls;
  const stageLabel = Object.keys(laidOut).find(key => key.startsWith('After endmill'));
  if (!stageLabel) throw new Error('no stage jump button is laid out');
  const stage = laidOut[stageLabel];
  const start = (await state()).controls['Start'];
  if (!start) throw new Error('the Start button is not laid out');
  const press = async rect => {
    await click((rect[0] + rect[2]) / 2, (rect[1] + rect[3]) / 2);
    await sleep(40);
  };
  const framesBefore = (await state()).renderer.framesSeen;
  for (let round = 0; round < 20; round += 1) {
    await press(stage);
    await press(start);
  }
  const scrubbed = await waitFor(s => !s.active, 'scrub burst settled', 300);
  if (scrubbed.renderer.framesSeen <= framesBefore)
    throw new Error('the scrub burst did not draw any frames');

  // Camera movement at a settled scene: enough frames to fill the sampling
  // window, and no scene data may be copied (§2.5 camera movement and idle).
  const idleBefore = await state();
  const size = await evaluate('({w:innerWidth,h:innerHeight})');
  const cx = Math.round(size.w * 0.55);
  const cy = Math.round(size.h * 0.5);
  for (let step = 0; step < 150; step += 1) {
    await send('Input.dispatchMouseEvent', {
      type: 'mouseWheel',
      x: cx,
      y: cy,
      deltaX: 0,
      deltaY: step % 2 === 0 ? 120 : -120,
    });
    await sleep(15);
  }
  await sleep(400);
  const panned = await state();
  if (panned.renderer.pageUploads !== idleBefore.renderer.pageUploads)
    throw new Error('camera movement copied motion pages');
  if (panned.renderer.uploadBytes !== idleBefore.renderer.uploadBytes)
    throw new Error('camera movement copied scene bytes');
  if (panned.stockPrefix !== idleBefore.stockPrefix)
    throw new Error('camera movement moved the playhead');
  // The plan's M camera target: p95 frame interval at or below 33 ms.
  if (panned.renderer.frameMs.p95 > 33)
    throw new Error(
      `camera frame p95 ${panned.renderer.frameMs.p95} ms exceeds the 33 ms budget`,
    );
  // §2.5 idle rule: at least thirty seconds with no animation, task or input.
  await sleep(30000);
  const idleAfter = await state();
  if (idleAfter.renderer.pageUploads !== panned.renderer.pageUploads)
    throw new Error('an idle display copied motion pages again');
  if (idleAfter.stockPrefix !== panned.stockPrefix)
    throw new Error('an idle display moved the playhead');
  if (idleAfter.renderer.uploadBytes !== panned.renderer.uploadBytes)
    throw new Error('an idle display copied scene bytes');
  record('sustained scrub and camera-only frames', {
    rounds: 20,
    playheadAfterBurst: scrubbed.stockPrefix,
    framesSampled: scrubbed.renderer.frameMs.samples,
    frameMsP50: scrubbed.renderer.frameMs.p50,
    frameMsP95: scrubbed.renderer.frameMs.p95,
    frameMsMax: scrubbed.renderer.frameMs.max,
    cameraFramesSampled: panned.renderer.frameMs.samples,
    cameraFrameMsP50: panned.renderer.frameMs.p50,
    cameraFrameMsP95: panned.renderer.frameMs.p95,
    cameraFrameMsMax: panned.renderer.frameMs.max,
    cameraPageUploads: panned.renderer.pageUploads - idleBefore.renderer.pageUploads,
    cameraUploadBytes: panned.renderer.uploadBytes - idleBefore.renderer.uploadBytes,
    idleSeconds: 30,
    residentBytes: idleAfter.renderer.residentBytes,
    build: idleAfter.renderer.build,
  });

  // Display-memory categories against the browser budget (plan section 2.5).
  // The page cannot see WASM linear memory or total GPU memory; that is
  // recorded as an unknown rather than as zero.
  const memory = idleAfter.renderer.memory;
  if (memory.withinBudget !== true)
    throw new Error(
      `declared display data ${memory.declaredDisplayBytes} B exceeds the ${memory.displayBudgetBytes} B budget`,
    );
  const jsHeap = await evaluate(`(() => {
    const m = performance.memory;
    return m ? {usedBytes: m.usedJSHeapSize, totalBytes: m.totalJSHeapSize, limitBytes: m.jsHeapSizeLimit} : null;
  })()`);
  record('display memory categories', {
    sceneBytes: memory.sceneBytes,
    checkpointBytes: memory.checkpointBytes,
    gpuPageBytes: memory.gpuPageBytes,
    stockTileBytes: memory.stockTileBytes,
    declaredDisplayBytes: memory.declaredDisplayBytes,
    displayBudgetBytes: memory.displayBudgetBytes,
    withinBudget: memory.withinBudget,
    jsHeap: jsHeap ?? 'not exposed by this browser',
    unknown: 'WASM linear memory and total GPU memory are not exposed to the page',
  });
  await screenshot('gui9c-sustained-scrub.png');

  // A display rebuild is short by design (18–109 ms natively); the edit during
  // a *heavy* task is covered by the generation step above. Switch back so the
  // final regression tour starts from the operation editor.
  await control('Cutting');
  await screenshot('gui9c-edit-under-load.png');

  // The small reference must still work in the same session.
  await control('File');
  await control('Flower fixture');
  await waitFor(s => !s.active && s.job?.operations?.length === 1, 'small reference opened', 240);
  await control('Cutting');
  await control('Generate');
  // Cancel that generation mid-flight: the worker stops, the draft and the
  // window survive, and the same session generates again afterwards.
  await sleep(250);
  const cancelling = await state();
  if (cancelling.active !== true)
    throw new Error('the reference generation was not running when sampled');
  await control('Cancel');
  const cancelled = await waitFor(s => !s.active, 'cancelled generation', 240);
  if (cancelled.revision !== cancelling.revision)
    throw new Error('cancelling changed the document revision');
  if (!cancelled.job || cancelled.job.operations?.length !== 1)
    throw new Error('cancelling lost the document');
  if (!/cancel/i.test(cancelled.status ?? ''))
    throw new Error(`cancelling was not reported: ${cancelled.status}`);
  record('cancelled a running generation', {
    revisionBefore: cancelling.revision,
    revisionAfter: cancelled.revision,
    documentKept: Boolean(cancelled.job),
    status: cancelled.status,
  });
  await screenshot('gui9c-cancelled.png');
  await control('Generate');
  const small = await waitFor(
    s => s.current && !s.active && s.motions === 22883,
    'small reference generation',
    900,
  );
  record('small reference unchanged', {motions: small.motions});

  await screenshot('gui9-small-reference.png');
  if (!out) throw new Error('missing evidence directory');
}
