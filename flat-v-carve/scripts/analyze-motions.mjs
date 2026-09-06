// Measures serialized motion records; does not authenticate or verify the plan.
// Feed-only time excludes rapid travel, acceleration, tool changes and dwell.
import fs from 'node:fs';
import { pathToFileURL } from 'node:url';

export function statistics(motions) {
  const byKind = {};
  for (const m of motions) {
    const row = byKind[m.kind] ??= { count: 0, distance_mm: 0, xy_distance_mm: 0, feed_minutes: 0 };
    const distance = Math.hypot(m.end.x - m.start.x, m.end.y - m.start.y, m.end.z - m.start.z);
    row.count++;
    row.distance_mm += distance;
    row.xy_distance_mm += Math.hypot(m.end.x - m.start.x, m.end.y - m.start.y);
    if (m.feed_mm_min) row.feed_minutes += distance / m.feed_mm_min;
  }
  return {
    motions: motions.length,
    total_distance_mm: Object.values(byKind).reduce((sum, row) => sum + row.distance_mm, 0),
    total_xy_distance_mm: Object.values(byKind).reduce((sum, row) => sum + row.xy_distance_mm, 0),
    feed_minutes: Object.values(byKind).reduce((sum, row) => sum + row.feed_minutes, 0),
    by_kind: byKind,
  };
}

if (process.argv[1] && import.meta.url === pathToFileURL(process.argv[1]).href) {
  if (process.argv.length < 3) throw new Error('usage: node scripts/analyze-motions.mjs <plan.json> [...]');
  for (const file of process.argv.slice(2)) {
    const plan = JSON.parse(fs.readFileSync(file, 'utf8'));
    console.log(JSON.stringify({
      file,
      engine_version: plan.engine_version,
      plan_bytes: fs.statSync(file).size,
      endmill: statistics(plan.endmill?.motions ?? plan.motions),
      ...(plan.vbit_motions ? { vbit: statistics(plan.vbit_motions) } : {}),
    }, null, 2));
  }
}
