// GUI10 machine-time playback in a real browser: the tester's facing job plays
// as a machine clock instead of a sequence of states, and a job whose holder
// cannot clear its own carve reports the warning, with the raster it rests on,
// and seeks to it.
export async function gui10Scenario({control,state,waitFor,send,evaluate,sleep,record,screenshot,readFileSync,click,out}) {
  const simulation = snapshot => {
    const value = snapshot?.display?.simulation;
    if (!value) throw new Error('the probe publishes no simulation state');
    return value;
  };
  const warnings = snapshot => {
    const value = snapshot?.display?.warnings;
    if (!value) throw new Error('the probe publishes no machine warnings');
    return value;
  };
  const drop = async (json, name) => {
    await evaluate(`(()=>{const t=new DataTransfer();t.items.add(new File([${JSON.stringify(json)}],${JSON.stringify(name)},{type:'application/json'}));document.getElementById('cam').dispatchEvent(new DragEvent('drop',{dataTransfer:t,bubbles:true,cancelable:true}));})()`);
  };

  // --- the tester's facing job: a machine clock, not a step counter ---------
  // `real_data/facing_job.json` is the job the report came from: 47 motions and
  // roughly two and a half minutes of modeled motion.
  const facing = readFileSync('../real_data/facing_job.json', 'utf8');
  await drop(facing, 'facing_job.json');
  const opened = await waitFor(
    s => s.job?.operations?.length === 1 && !s.active,
    'the reported facing job',
    240,
  );
  await control('Machine');
  await control('Example machine');
  await control('Apply flower machine profile');
  await waitFor(s => !s.active && s.job.machine, 'applied machine profile', 240);
  await control('Cutting');
  await control('Generate');
  const generated = await waitFor(
    s => s.current && !s.active && s.motions > 0,
    'facing generation',
    600,
  );
  await control('Simulate');
  const timed = await waitFor(
    s => !s.active && simulation(s).timed,
    'a timed playback clock',
    240,
  );
  const clock = simulation(timed);
  if (!(clock.totalSeconds > 60 && clock.totalSeconds < 400))
    throw new Error(`the facing job's modeled time is ${clock.totalSeconds} s`);
  if (!clock.rapidRateAssumed || clock.rapidRateMmMin !== 5000)
    throw new Error(`this machine states no rapid rate: ${JSON.stringify(clock)}`);
  if (clock.fit || clock.fastForward !== 1)
    throw new Error(`playback starts at real time: ${JSON.stringify(clock)}`);
  record('facing job carries a machine clock', {
    motions: generated.motions,
    totalSeconds: clock.totalSeconds,
    rapidRateMmMin: clock.rapidRateMmMin,
    rapidRateAssumed: clock.rapidRateAssumed,
    previousBehaviour: '20 s of steps however long the program is',
  });
  await screenshot('gui10-facing-timed.png');

  // --- playing at 1x advances continuously, inside a single motion ----------
  await control('Start');
  await waitFor(s => !s.active && s.stockPrefix === 0, 'stock restored to the start', 300);
  await control('Play');
  const samples = [];
  let elapsed = 0;
  for (let index = 0; index < 8; index++) {
    await sleep(350);
    elapsed += 0.35;
    samples.push(simulation(await state()));
  }
  if (await state().then(s => s.controls.Pause)) await control('Pause');
  await waitFor(s => !s.active, 'playback paused');
  const mid = samples.filter(sample => sample.fraction > 0);
  if (!mid.length)
    throw new Error(`playback never stood inside a motion: ${JSON.stringify(samples)}`);
  for (let index = 1; index < samples.length; index++) {
    if (!(samples[index].elapsedSeconds >= samples[index - 1].elapsedSeconds))
      throw new Error(
        `the clock went backwards: ${samples[index - 1].elapsedSeconds} -> ${samples[index].elapsedSeconds}`,
      );
  }
  const advanced = samples[samples.length - 1].elapsedSeconds;
  if (advanced < elapsed * 0.25)
    throw new Error(`1x advanced ${advanced} s over ${elapsed} s of wall clock`);
  record('1x is continuous machine time', {
    samples: samples.map(sample => ({
      prefix: sample.prefix,
      fraction: Number(sample.fraction.toFixed(3)),
      elapsedSeconds: Number(sample.elapsedSeconds.toFixed(2)),
      feedMmMin: sample.feedMmMin,
    })),
    insideAMotion: mid.length,
    advancedSeconds: Number(advanced.toFixed(2)),
    wallClockSeconds: elapsed,
  });

  // The walls drawn for the material that is left have to move with the floor,
  // not with the motion index: a facing pass removes material for seconds inside
  // one motion, and wall geometry keyed on the prefix stands still until the
  // move ends. Sample the wall revision next to the position: two samples inside
  // the same motion must not share a wall revision.
  const walls = snapshot => {
    const value = snapshot?.display?.walls;
    if (!value) throw new Error('the probe publishes no wall geometry');
    return value;
  };
  const wallSamples = [];
  const transferBytes = [];
  await control('Start');
  await waitFor(s => !s.active && s.stockPrefix === 0, 'stock restored again', 300);
  await control('Play');
  for (let index = 0; index < 6; index++) {
    await sleep(300);
    const sample = await state();
    wallSamples.push({
      prefix: simulation(sample).prefix,
      fraction: simulation(sample).fraction,
      walls: walls(sample).revision,
      instances: walls(sample).instances,
    });
    transferBytes.push(sample.stockTransferBytes);
  }
  if (await state().then(s => s.controls.Pause)) await control('Pause');
  await waitFor(s => !s.active, 'playback paused again');
  let insideOneMotion = 0;
  let stale = 0;
  for (let index = 1; index < wallSamples.length; index++) {
    const previous = wallSamples[index - 1];
    const sample = wallSamples[index];
    if (sample.prefix !== previous.prefix) continue;
    insideOneMotion++;
    if (sample.walls === previous.walls) stale++;
  }
  if (!insideOneMotion)
    throw new Error(`no two wall samples shared a motion: ${JSON.stringify(wallSamples)}`);
  if (stale)
    throw new Error(
      `${stale} wall update(s) stood still inside a motion: ${JSON.stringify(wallSamples)}`,
    );
  record('walls move with the floor, inside a motion', {
    samples: wallSamples.map(sample => ({
      prefix: sample.prefix,
      fraction: Number(sample.fraction.toFixed(3)),
      walls: sample.walls,
      instances: sample.instances,
    })),
    insideOneMotion,
  });

  // The display owns the clock between seeks: playing forward must not ask the
  // compute process for anything, so the last stock transfer stays the one the
  // seek caused.
  const transfers = new Set(transferBytes);
  if (transfers.size !== 1)
    throw new Error(
      `playback asked the compute process for stock: ${JSON.stringify(transferBytes)}`,
    );
  record('playback stays in the display process', {
    transferBytes: transferBytes[0],
    samples: transferBytes.length,
  });

  // --- scrubbing to a time lands inside a move, forwards and backwards ------
  const scrub = async (share, label) => {
    const rect = (await state()).controls['Program time'];
    if (!rect) throw new Error('the transport has no Program time slider');
    await click(rect[0] + (rect[2] - rect[0]) * share, (rect[1] + rect[3]) / 2);
    const settled = await waitFor(s => !s.active, `${label} scrub`, 240);
    // The widget rect includes the label, so the click lands somewhere on the
    // track rather than at exactly this share of the program. What the check is
    // about is the behaviour: a scrub to a time lands *inside* a move, and a
    // click further along the track lands later in the program.
    return simulation(settled);
  };
  const forward = await scrub(0.7, 'forward');
  const backward = await scrub(0.2, 'backward');
  if (!(backward.elapsedSeconds < forward.elapsedSeconds))
    throw new Error(
      `the backward scrub did not move back: ${forward.elapsedSeconds} -> ${backward.elapsedSeconds}`,
    );
  if (!(backward.fraction > 0))
    throw new Error(`a scrub to a time did not land inside a move: ${JSON.stringify(backward)}`);
  if (!(backward.prefix < forward.prefix))
    throw new Error(
      `the two scrubs landed on the same motion: ${backward.prefix} and ${forward.prefix}`,
    );
  record('scrubbing to a time lands inside a move', {
    forward: {
      elapsedSeconds: Number(forward.elapsedSeconds.toFixed(2)),
      prefix: forward.prefix,
      fraction: Number(forward.fraction.toFixed(3)),
      feedMmMin: forward.feedMmMin,
    },
    backward: {
      elapsedSeconds: Number(backward.elapsedSeconds.toFixed(2)),
      prefix: backward.prefix,
      fraction: Number(backward.fraction.toFixed(3)),
      feedMmMin: backward.feedMmMin,
    },
  });

  // --- Fit maps the whole program into one window at the same ratios --------
  await control('Playback Fit');
  const fitted = await waitFor(
    s => !s.active && simulation(s).fit,
    'the whole program fitted',
    120,
  );
  const fit = simulation(fitted);
  const expected = fit.totalSeconds / 20;
  if (Math.abs(fit.fastForward - expected) > expected * 0.05)
    throw new Error(`Fit runs at ${fit.fastForward}x, not ${expected}x`);
  record('Fit keeps the ratios of the program in one window', {
    totalSeconds: fit.totalSeconds,
    fastForward: Number(fit.fastForward.toFixed(3)),
    windowSeconds: 20,
  });

  // --- a job with no stated assembly is not guessed at ----------------------
  const none = simulation(await state());
  if (none.assembly?.shaftDiameterMm !== null || none.assembly?.stickoutMm !== null)
    throw new Error(`a tool with no assembly was invented: ${JSON.stringify(none.assembly)}`);
  if (warnings(await state()).count !== 0)
    throw new Error('a tool with no assembly produced machine warnings');
  record('an unstated assembly is not invented', {assembly: none.assembly});

  // --- a holder that cannot clear the carve is reported and seekable ---------
  const tight = readFileSync('fixtures/gui10/tight-holder.job.json', 'utf8');
  await drop(tight, 'tight-holder.job.json');
  await waitFor(s => s.job?.name && !s.active, 'the tight-holder fixture', 240);
  await control('Cutting');
  await control('Generate');
  await waitFor(s => s.current && !s.active && s.motions > 0, 'tight generation', 600);
  await control('Simulate');
  const checked = await waitFor(
    s => !s.active && warnings(s).count > 0,
    'a machine warning',
    300,
  );
  const list = warnings(checked);
  const reported = list.entries.filter(entry => entry.kind === 'assemblyBelowSurface');
  if (!reported.length)
    throw new Error(`the holder warning is missing: ${JSON.stringify(list)}`);
  if (!(list.cellMm > 0))
    throw new Error(`the warning does not name its raster: ${JSON.stringify(list)}`);
  // One row per problem: it names the first motion that shows it and the worst
  // depth the program reaches, which for this fixture is 0.7 mm of nut inside
  // the material beside the carve.
  const deepest = reported[0];
  if (!(deepest.maxDepthMm > 0.4 && deepest.maxDepthMm <= 1))
    throw new Error(`the holder reaches ${deepest.maxDepthMm} mm inside the material`);
  const assembly = reported[0];
  const model = simulation(checked);
  if (model.assembly?.shaftDiameterMm !== 2.5 || model.assembly?.stickoutMm !== 0.8)
    throw new Error(`the scene does not model the stated assembly: ${JSON.stringify(model.assembly)}`);
  if (model.holderSegments !== 2)
    throw new Error(`the ER20 body is ${model.holderSegments} segments`);
  record('the holder that cannot clear the carve is reported', {
    cellMm: list.cellMm,
    count: list.count,
    first: reported[0],
    deepest,
    assembly: model.assembly,
    holderSegments: model.holderSegments,
  });
  await screenshot('gui10-holder-warning.png');

  // The warning seeks to the motion it names through the same path as any
  // other jump.
  await control('Show warning');
  const shown = await waitFor(
    s => !s.active && simulation(s).prefix === assembly.motion,
    'the warning seek',
    300,
  );
  record('a warning seeks to its motion', {
    motion: assembly.motion,
    prefix: simulation(shown).prefix,
    seconds: Number(simulation(shown).elapsedSeconds.toFixed(2)),
  });

  // The warnings belong to the execution, not to the playhead: seeking away and
  // back must not lose them.
  await control('Start');
  const away = await waitFor(s => !s.active && s.stockPrefix === 0, 'away from the warning', 300);
  const stillThere = warnings(away);
  if (
    stillThere.count !== list.count ||
    JSON.stringify(stillThere.entries) !== JSON.stringify(list.entries)
  )
    throw new Error(
      `seeking dropped or changed the warnings: ${JSON.stringify(stillThere)} vs ${JSON.stringify(list)}`,
    );
  record('warnings belong to the execution, not the playhead', {
    count: stillThere.count,
    atStart: stillThere.entries[0],
  });

  // Every problem is listed — one row per problem, not one per motion — and the
  // raster every claim rests on travels with the list. egui paints to a canvas,
  // so the row text itself is not readable from the DOM; the panel line is the
  // same numbers the probe publishes.
  const listed = warnings(await state());
  if (listed.count !== listed.entries.length)
    throw new Error(
      `the panel hides problems: ${listed.count} reported, ${listed.entries.length} listed`,
    );
  if (listed.count !== 1)
    throw new Error(`one problem, one row: ${JSON.stringify(listed)}`);
  record('one row per problem, with its raster', {
    count: listed.count,
    cellMm: listed.cellMm,
    entry: listed.entries[0],
  });
}
