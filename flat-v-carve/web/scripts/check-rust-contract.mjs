import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import { fileURLToPath } from 'node:url';
import * as schema from '../src/contracts/job.ts';

// A drift alarm, not a Rust schema generator or machining validator.
// Check the full serialized field sets, including opaque blocks the U1 UI preserves.
const checks = [
  ['job.rs', 'Job', schema.jobSchema],
  ['job.rs', 'SourceSnapshot', schema.sourceSchema],
  ['job.rs', 'ToolSettings', schema.toolSchema],
  ['job.rs', 'StockSettings', schema.stockSchema],
  ['job.rs', 'OperationSettings', schema.operationSchema],
  ['job.rs', 'PlanningTolerances', schema.tolerancesSchema],
  ['job.rs', 'MachineProfile', schema.machineProfileSchema],
  ['svg/mod.rs', 'ImportOptions', schema.importSchema],
  ['svg/mod.rs', 'Placement', schema.importSchema.shape.placement],
  ['model.rs', 'EndmillSpec', schema.endmillSpecSchema],
  ['model.rs', 'VBitSpec', schema.vbitSpecSchema],
  ['pocket/settings.rs', 'EndmillPlanningSettings', schema.endmillPlanningSchema],
  ['vcarve/settings.rs', 'VBitPlanningSettings', schema.vbitPlanningSchema],
];
for (const [file, name, validator] of checks) {
  const source = readFileSync(new URL(`../../crates/cam-core/src/${file}`, import.meta.url), 'utf8');
  const body = source.match(new RegExp(`pub struct ${name} \\{([\\s\\S]*?)\\n\\}`))?.[1];
  assert.ok(body, `Cannot locate Rust ${name}; review schema check`);
  const rustFields = [...body.matchAll(/^\s*pub (\w+):/gm)].map(match => match[1]).sort();
  assert.deepEqual(Object.keys(validator.shape).sort(), rustFields, `Rust ${name} field set changed`);
}
const jobSource = readFileSync(new URL('../../crates/cam-core/src/job.rs', import.meta.url), 'utf8');
assert.match(jobSource, /JOB_SCHEMA_VERSION: u32 = 3;/, 'Review frontend schema after Rust job schema changes');
console.log(`Checked ${checks.length} Rust struct field sets and job schema version (read-only).`);
console.log(`Schema: ${fileURLToPath(new URL('../src/contracts/job.ts', import.meta.url))}`);
