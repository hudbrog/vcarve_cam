import { afterAll, beforeAll, describe, expect, it } from 'vitest';
import { spawn, spawnSync, type ChildProcess } from 'node:child_process';
import { readFileSync, mkdirSync, writeFileSync } from 'node:fs';
import { fileURLToPath } from 'node:url';
import { dirname } from 'node:path';
import { createHttpService } from '../src/service/http';
import type { CamService } from '../src/contracts/service';
import { terminal, type PlanTask, type TaskIdentity } from '../src/contracts/planning';
import { verificationIdentity, type VerificationIdentity } from '../src/contracts/verification';
import { exportIdentity, profileSchema, type ExportIdentity } from '../src/contracts/export';
import { createHash } from 'node:crypto';
import { computationDefaults, defaultVbitComputation, defaultTolerances } from '../src/state/computation';
import { materialize, newDraft } from '../src/state/draft';
import { missingPlanningSettings } from '../src/state/setupNeeds';
import { applyLibraryToolToDraft, resolveDraftLibraryTool } from '../src/state/library';

// Runs the actual Rust server and the same-engine CLI; no fixture HTTP responses.
const workspace = fileURLToPath(new URL('../../', import.meta.url));
const web = fileURLToPath(new URL('../', import.meta.url));
const suffix = process.platform === 'win32' ? '.exe' : '';
// Set CAM_TEST_EXE to exercise one copied portable executable with no UI directory.
const portable = process.env.CAM_TEST_EXE;
const executable = portable ?? `${workspace}target/release/cam-web${suffix}`;
const cli = portable ?? `${workspace}target/release/cam${suffix}`;
const output = `${web}test-results/live`;
const libraryDirectory = `${output}/library-${crypto.randomUUID()}`;
let child: ChildProcess;
let base: string;
let service: CamService;
function runCli(args: string[], expectedStatus = 0) {
  const result = spawnSync(cli, args, { cwd: workspace, encoding: 'utf8', maxBuffer: 16_000_000, windowsHide: true });
  if (result.error) throw result.error;
  expect(result.status, result.stderr).toBe(expectedStatus);
  return result.stdout;
}
beforeAll(async () => {
  mkdirSync(output, { recursive: true });
  const args = portable ? ['serve'] : ['--ui-dir', `${web}dist`];
  const env = portable ? { ...process.env, PATH: process.platform === 'win32' ? `${process.env.SystemRoot}\\System32;${process.env.SystemRoot}` : '/usr/bin:/bin' } : process.env;
  child = spawn(executable, [...args, '--port', '0', '--library-dir', libraryDirectory], { cwd: portable ? dirname(portable) : workspace, env, windowsHide: true });
  base = await new Promise<string>((resolve, reject) => {
    let stdout = ''; let stderr = '';
    const timer = setTimeout(() => reject(new Error(`Server did not start: ${stderr}`)), 15_000);
    child.on('error', error => { clearTimeout(timer); reject(error); });
    child.stderr?.on('data', chunk => { stderr += chunk; });
    child.once('exit', code => { clearTimeout(timer); reject(new Error(`Server exited ${code}: ${stderr}`)); });
    child.stdout?.on('data', chunk => {
      stdout += chunk;
      const url = stdout.match(/CAM_WEB_URL=(http:\/\/127\.0\.0\.1:\d+)/)?.[1];
      if (url) { clearTimeout(timer); resolve(url); }
    });
  });
  service = createHttpService((url, init) => fetch(new URL(String(url), base), init));
  await service.capabilities();
}, 20_000);
afterAll(() => { child?.kill(); });

describe('packaged browser assets', () => {
  it('serves the complete production bundle with the expected bytes and content types', async () => {
    const manifest = JSON.parse(readFileSync(`${web}dist/.bundle-manifest.json`, 'utf8')) as { assets: Record<string, string> };
    for (const [path, hash] of Object.entries(manifest.assets)) {
      const response = await fetch(new URL(path === 'index.html' ? '/' : `/${path}`, base));
      expect(response.status, path).toBe(200);
      expect(createHash('sha256').update(new Uint8Array(await response.arrayBuffer())).digest('hex'), path).toBe(hash);
      const mime = path.endsWith('.html') ? 'text/html' : path.endsWith('.js') ? 'text/javascript' : path.endsWith('.css') ? 'text/css' : null;
      if (mime) expect(response.headers.get('content-type')).toContain(mime);
    }
    expect((await fetch(`${base}/.bundle-manifest.json`)).status).toBe(404);
    expect((await fetch(`${base}/missing.js`)).status).toBe(404);
  });
});

describe('guided setup to combined plan',()=>{
  it('supplies omitted computation and tolerance defaults before submission and completes planning',async()=>{
    const opened=await service.openJob!(readFileSync(`${workspace}fixtures/m4/wide-floor.json`,'utf8'),30);
    opened.job.vbit_planning=null;
    const before=await service.validateDraft(opened.job,30);
    expect(before.valid).toBe(true);
    expect(missingPlanningSettings(opened.job,before,'combined').map(n=>n.path)).toEqual(['vbit_planning']);
    opened.job.tolerances={motion_tolerance_mm:null,verification_tolerance_mm:null};
    const draft=newDraft(opened.job);
    for(const path of Object.keys(computationDefaults).filter(path=>path.startsWith('endmill_planning.'))) draft.text[path]='';
    const configured=materialize(draft).job!;
    expect(configured.vbit_planning).toEqual(defaultVbitComputation);
    expect(configured.tolerances).toEqual(defaultTolerances);
    const checked=await service.validateDraft(configured,31);
    expect(checked.valid).toBe(true); expect(missingPlanningSettings(configured,checked,'combined')).toEqual([]);
    const caps=await service.capabilities();
    const id:TaskIdentity={taskId:crypto.randomUUID(),instanceId:caps.planning!.instanceId,engineVersion:caps.engineVersion,revision:31,documentFingerprint:checked.documentFingerprint!,stage:'combined'};
    await service.startPlan!(configured,id); const task=await finish(id);
    expect(task.state,JSON.stringify(task.diagnostic)).toBe('succeeded'); expect(task.summary?.status).toBe('complete');
  },60_000);
  it('removes a deliberately low work override without changing the cut or accuracy requirements',async()=>{
    const opened=await service.openJob!(readFileSync(`${workspace}fixtures/m4/wide-floor.json`,'utf8'),32);
    opened.job.endmill_planning!.max_layers=1;
    const caps=await service.capabilities();
    async function plan(job:typeof opened.job,revision:number) {
      const checked=await service.validateDraft(job,revision);
      expect(checked.valid).toBe(true);
      const id:TaskIdentity={taskId:crypto.randomUUID(),instanceId:caps.planning!.instanceId,engineVersion:caps.engineVersion,revision,documentFingerprint:checked.documentFingerprint!,stage:'combined'};
      await service.startPlan!(job,id); return finish(id);
    }
    const limited=await plan(opened.job,32);
    expect(limited.state).toBe('failed'); expect(limited.diagnostic?.code).toBe('PLANNING_RESOURCE_LIMIT');
    const draft=newDraft(opened.job); draft.text['endmill_planning.max_layers']='';
    const resolved=materialize(draft).job!;
    expect(resolved.tools).toEqual(opened.job.tools); expect(resolved.operation).toEqual(opened.job.operation); expect(resolved.tolerances).toEqual(opened.job.tolerances);
    const completed=await plan(resolved,33);
    expect(completed.state,JSON.stringify(completed.diagnostic)).toBe('succeeded'); expect(completed.summary?.status).toBe('complete');
  },60_000);
});

describe('persistent tool library and CLI parity', () => {
  it('captures, edits, reviews, and applies snapshots with transactional conflicts and imports', async () => {
    const caps = await service.capabilities();
    const connection = {instanceId:caps.planning!.instanceId,engineVersion:caps.engineVersion};
    expect(caps.toolLibrary?.schemaVersion).toBe(1);
    expect((await service.library!(connection)).data).toEqual({state:'missing',library:null});
    expect((await service.initializeLibrary!(connection)).data.library?.revision).toBe(0);
    await expect(service.initializeLibrary!(connection)).rejects.toThrow(/LIBRARY_EXISTS/);
    const opened = await service.openJob!(readFileSync(`${workspace}fixtures/m4/finite-tip.json`,'utf8'),11);
    opened.job.tools.reverse();
    const validation = await service.validateDraft(opened.job,12);
    const source = {job:opened.job,revision:12,documentFingerprint:validation.documentFingerprint!};
    const captured = await service.captureLibraryTool!(connection,{...source,expectedRevision:0,slot:'endmill',toolId:'test-mill',name:'Synthetic test mill',
      preset:{id:'test-preset',name:'Recorded test values',material:'Test material',machine:null}});
    expect(captured.data.library?.revision).toBe(1);
    const tool = captured.data.library!.tools[0];
    expect(tool.geometry).toEqual(source.job.tools[1].geometry);
    const exported = `${output}/library-${crypto.randomUUID()}.json`;
    runCli(['tool-library','export',libraryDirectory,'--output',exported]);
    expect(JSON.parse(readFileSync(exported,'utf8'))).toEqual(captured.data.library);
    const selection={...source,expectedRevision:1,slot:'endmill' as const,toolId:'test-mill',presetId:null};
    const candidate = await service.applyLibraryTool!(connection,selection);
    const draftSelection={connection,expectedRevision:1,draftRevision:12,slot:'endmill' as const,jobToolId:source.job.tools[1].id,toolId:tool.id,presetId:null};
    expect(resolveDraftLibraryTool(captured,draftSelection).settings).toEqual(candidate.data.job.tools[1]);
    for (const key of ['spindle_rpm','cutting_feed_mm_min','plunge_feed_mm_min','max_stepdown_mm','stepover_mm']) expect(candidate.data.job.tools[1]).toHaveProperty(key,null);
    expect(candidate.data.job.tools[0]).toEqual(source.job.tools[0]);
    expect(candidate.data.job.operation).toEqual(source.job.operation);
    const jobPath=`${output}/library-source.job.json`; writeFileSync(jobPath,JSON.stringify(source.job));
    const appliedPath=`${output}/library-applied-${crypto.randomUUID()}.job.json`;
    runCli(['tool-library','apply',libraryDirectory,'--expected-revision','1','--job',jobPath,'--slot','endmill','--tool','test-mill','--output',appliedPath]);
    expect(JSON.parse(readFileSync(appliedPath,'utf8'))).toEqual(candidate.data.job);
    const withPreset=await service.applyLibraryTool!(connection,{...selection,presetId:'test-preset'});
    expect(withPreset.data.job).toEqual(source.job);
    expect(resolveDraftLibraryTool(captured,{...draftSelection,presetId:'test-preset'}).settings).toEqual(withPreset.data.job.tools[1]);
    await expect(service.applyLibraryTool!(connection,{...selection,slot:'vbit'})).rejects.toThrow(/LIBRARY_TOOL_KIND/);
    await expect(service.applyLibraryTool!(connection,{...selection,documentFingerprint:'0'.repeat(64)})).rejects.toThrow(/STALE_DOCUMENT/);
    await expect(service.library!({...connection,instanceId:'0'.repeat(32)})).rejects.toThrow(/service changed/);
    // An independent CLI writer makes the open editor/review stale.
    const changeFile=`${output}/library-change.json`;
    writeFileSync(changeFile,JSON.stringify({kind:'duplicate_tool',tool_id:tool.id,new_id:'cli-copy',name:'CLI copy'}));
    runCli(['tool-library','change',libraryDirectory,'--expected-revision','1','--input',changeFile]);
    await expect(service.changeLibrary!(connection,1,{kind:'replace_tool',tool:{...tool,name:'Stale write'}})).rejects.toThrow(/LIBRARY_CONFLICT/);
    await expect(service.applyLibraryTool!(connection,selection)).rejects.toThrow(/LIBRARY_CONFLICT/);
    const refreshed=await service.library!(connection);
    expect(()=>resolveDraftLibraryTool(refreshed,draftSelection)).toThrow(/library changed/);
    const current=refreshed.data.library!;
    expect(current.revision).toBe(2); expect(current.tools[0].name).toBe(tool.name);
    const before=readFileSync(`${libraryDirectory}/library.json`);
    const imported={schema_version:1,revision:999,tools:[{...tool,id:'new-import'},tool]};
    await expect(service.importLibrary!(connection,2,JSON.stringify(imported))).rejects.toThrow(/LIBRARY_DUPLICATE/);
    await expect(service.importLibrary!(connection,2,'{"schema_version":1,"schema_version":1,"revision":0,"tools":[]}')).rejects.toThrow(/LIBRARY_JSON/);
    expect(readFileSync(`${libraryDirectory}/library.json`)).toEqual(before);
    const merged=await service.importLibrary!(connection,2,JSON.stringify({...imported,tools:[{...imported.tools[0],ramp_capable:undefined,plunge_capable:undefined,
      cutting_presets:[{id:'blank',name:'Blank imported preset'}]}]}));
    expect(merged.data.library?.revision).toBe(3);
    expect(merged.data.library?.tools.find(t=>t.id==='new-import')?.cutting_presets[0].spindle_rpm).toBeNull();
    const preset={...tool.cutting_presets[0],id:'partial',name:'Partial settings',spindle_rpm:null};
    await service.changeLibrary!(connection,3,{kind:'add_preset',tool_id:tool.id,preset});
    await service.changeLibrary!(connection,4,{kind:'replace_preset',tool_id:tool.id,preset:{...preset,material:null}});
    await service.changeLibrary!(connection,5,{kind:'duplicate_preset',tool_id:tool.id,preset_id:preset.id,new_id:'preset-copy',name:'Copy'});
    const partial=await service.applyLibraryTool!(connection,{...selection,expectedRevision:6,presetId:'partial'});
    expect(partial.data.job.tools[1].spindle_rpm).toBeNull();
    expect(partial.data.job.tools[1].cutting_feed_mm_min).toBe(source.job.tools[1].cutting_feed_mm_min);
    expect(resolveDraftLibraryTool(await service.library!(connection),{...draftSelection,expectedRevision:6,presetId:'partial'}).settings).toEqual(partial.data.job.tools[1]);
    await service.changeLibrary!(connection,6,{kind:'remove_preset',tool_id:tool.id,preset_id:'preset-copy'});
    await service.changeLibrary!(connection,7,{kind:'remove_tool',tool_id:tool.id});
    expect((await service.validateDraft(candidate.data.job,13)).valid).toBe(true);
    const reopened=createHttpService((url,init)=>fetch(new URL(String(url),base),init));
    await reopened.capabilities();
    expect((await reopened.library!(connection)).data.library?.revision).toBe(8);
  },30_000);
  it('loads a V-bit into an invalid draft with Rust-equivalent settings, leaving planning validation intact',async()=>{
    const caps=await service.capabilities(),connection={instanceId:caps.planning!.instanceId,engineVersion:caps.engineVersion};
    const snapshot=await service.library!(connection),expectedRevision=snapshot.data.library!.revision;
    const opened=await service.openJob!(readFileSync(`${workspace}fixtures/m4/finite-tip.json`,'utf8'),20);
    const source={job:opened.job,revision:20,documentFingerprint:(await service.validateDraft(opened.job,20)).documentFingerprint!};
    const saved=await service.captureLibraryTool!(connection,{...source,expectedRevision,slot:'vbit',toolId:'draft-vbit',name:'Draft V-bit',
      preset:{id:'cut',name:'Recorded cut',material:null,machine:null}});
    const libraryRevision=saved.data.library!.revision,index=source.job.tools.findIndex(t=>t.id===source.job.operation.vbit_id);
    for (const presetId of [null,'cut']) {
      const selection={connection,expectedRevision:libraryRevision,draftRevision:20,slot:'vbit' as const,jobToolId:source.job.tools[index].id,toolId:'draft-vbit',presetId};
      const settings=resolveDraftLibraryTool(await service.library!(connection),selection).settings;
      const rust=await service.applyLibraryTool!(connection,{...source,expectedRevision:libraryRevision,slot:'vbit',toolId:'draft-vbit',presetId});
      expect(settings).toEqual(rust.data.job.tools[index]);
      const draft=newDraft(source.job);
      draft.text={[`tools.${index}.geometry.dimensions.included_angle_deg`]:'1e','stock.thickness_mm':'-1'};
      expect(materialize(draft).job).toBeNull();
      const applied=applyLibraryToolToDraft(draft,'vbit',settings);
      expect(applied.text).toEqual({'stock.thickness_mm':'-1'});
      expect((await service.validateDraft(materialize(applied).job!,21)).valid).toBe(false);
      delete applied.text['stock.thickness_mm'];
      const checked=await service.validateDraft(materialize(applied).job!,22);
      expect(checked.valid).toBe(true);
      if (presetId === null) expect(checked.missingMachiningFields).toContain(`tools.${selection.jobToolId}.spindle_rpm`);
    }
  });
});

async function finishExport(id: ExportIdentity) {
  const deadline = Date.now() + 60_000;
  while (Date.now() < deadline) {
    const task = await service.exportTask!(id);
    if (['succeeded','failed','cancelled'].includes(task.state)) return task;
    await new Promise(resolve => setTimeout(resolve,20));
  }
  throw new Error('Export did not finish within the test deadline');
}
describe('checked LinuxCNC output', () => {
  it('reports an unknown job ID separately from valid T1/T2 machine numbers', async () => {
    const {job,id} = await identity('m4/narrow-channel','combined');
    await service.startPlan!(job,id); const plan = await finish(id);
    const caps = await service.capabilities();
    const profile = profileSchema.parse(JSON.parse(readFileSync(`${workspace}fixtures/m6/macro-stock-bottom.json`,'utf8')));
    profile.tools[0].tool_id = '1';
    profile.tools[1].tool_id = '2';
    const accepted = exportIdentity(plan,profile,'combined',caps.verification!.defaultOptions,crypto.randomUUID());
    await service.startExport!(accepted);
    const task = await finishExport(accepted);
    expect(task.state).toBe('failed');
    expect(task.diagnostic?.code).toBe('POST_TOOL_MAPPING');
    expect(task.diagnostic?.message).toContain('select endmill ID "endmill" or V-bit ID "vbit"');
    expect(task.diagnostic?.message).not.toContain('compensation');
    await expect(service.exportResult!(accepted)).rejects.toThrow(/PLAN_RESULT_UNAVAILABLE/);
  },60_000);
  it.each([
    ['combined','narrow-channel','macro-stock-bottom','combined',6,1_000_000,'passed'],
    ['per-tool','wide-floor','macro-stock-bottom','per_tool',6,1_000_000,'passed'],
    ['tool-table','finite-tip','tool-table-synthetic','combined',6,1_000_000,'passed'],
    ['coarse','narrow-channel','macro-stock-bottom','combined',0,1_000_000,'passed'],
    ['limited','narrow-channel','macro-stock-bottom','combined',6,1,'inconclusive'],
  ] as const)('matches CLI reports and every program byte for %s', async (name,fixture,profileName,layout,precision,cells,status) => {
    const {job,id} = await identity(`m4/${fixture}`,'combined');
    await service.startPlan!(job,id); const plan = await finish(id);
    const caps = await service.capabilities();
    const profile = profileSchema.parse(JSON.parse(readFileSync(`${workspace}fixtures/m6/${profileName}.json`,'utf8')));
    profile.decimal_places = precision;
    const accepted = exportIdentity(plan,profile,layout,{...caps.verification!.defaultOptions,max_cells:cells},crypto.randomUUID());
    await expect(service.startExport!({...accepted,revision:accepted.revision + 1})).rejects.toThrow(/EXPORT_PLAN_IDENTITY/);
    expect((await service.startExport!(accepted)).state).toBe('queued');
    const changed = structuredClone(accepted); changed.export.profile.coolant = 'mist';
    await expect(service.startExport!(changed)).rejects.toThrow(/TASK_KEY_REUSED/);
    const task = await finishExport(accepted);
    expect(task.state,JSON.stringify(task.diagnostic)).toBe('succeeded');
    expect(task.summary?.status).toBe(status);
    const result = await service.exportResult!(accepted);
    const token = (await (await fetch(`${base}/api/v1/session`)).json()).sessionToken;
    const path = `${output}/export-${name}.plan.json`;
    writeFileSync(path,await (await fetch(`${base}/api/v1/tasks/${id.taskId}/artifact`,{headers:{'X-Cam-Session':token}})).text());
    const profilePath = `${output}/export-${name}.profile.json`; writeFileSync(profilePath,JSON.stringify(profile));
    const directory = `${output}/export-${name}-${crypto.randomUUID()}`;
    runCli(['export',path,'--profile',profilePath,'--output',directory,'--layout',layout.replace('_','-'),'--max-cells',String(cells)],status === 'passed' ? 0 : 1);
    expect(result.report).toEqual(JSON.parse(readFileSync(`${directory}/export-report.json`,'utf8')));
    expect(JSON.parse(result.reportJson)).toEqual(result.report);
    expect(result.programs.length).toBe(status === 'passed' ? result.report.programs.length : 0);
    for (const program of result.programs) {
      expect(Buffer.from(program.gcode)).toEqual(readFileSync(`${directory}/${program.filename}`));
      expect(createHash('sha256').update(program.gcode).digest('hex')).toBe(result.report.programs.find(p => p.filename === program.filename)!.sha256);
    }
    if (layout === 'per_tool') {
      expect(result.programs.map(p => p.filename)).toEqual(['endmill.ngc','vbit.ngc']);
      expect(result.report.programs[1].prerequisites.join(' ')).toMatch(/endmill.ngc/);
    }
    await expect(service.planResult!(accepted)).rejects.toThrow(/TASK_KIND/);
    await expect(service.verificationResult!({...accepted,verification:verificationIdentity(plan,accepted.export.options,'unused').verification})).rejects.toThrow(/TASK_KIND/);
    expect((await service.startExport!(accepted)).taskId).toBe(accepted.taskId);
    expect((await service.cancelExport!(accepted)).state).toBe('succeeded');
    // Capture precision adaptation separately from the historical failure fixture.
    if (name === 'coarse') writeFileSync(`${output}/m6-coarse.json`,JSON.stringify({result,plan},null,2));
    if (name === 'combined') writeFileSync(`${output}/m6-passed.json`,JSON.stringify({result,plan},null,2));
  },120_000);
  it('rejects incompatible/unreviewed profiles and cancels active export', async () => {
    const {job,id} = await identity('m4/island','combined');
    await service.startPlan!(job,id); const plan = await finish(id);
    const caps = await service.capabilities();
    const profile = profileSchema.parse(JSON.parse(readFileSync(`${workspace}fixtures/m6/macro-stock-bottom.json`,'utf8')));
    for (const patch of [{m6:{...profile.m6,reviewed:false}},{clearance_z_mm:6}]) {
      const accepted = exportIdentity(plan,{...profile,...patch},'combined',caps.verification!.defaultOptions,crypto.randomUUID());
      await service.startExport!(accepted);
      const task = await finishExport(accepted);
      expect(task.state).toBe('failed'); expect(task.diagnostic?.code).toMatch(/POST_M6_CONTRACT|POST_CLEARANCE/);
      await expect(service.exportResult!(accepted)).rejects.toThrow(/PLAN_RESULT_UNAVAILABLE/);
    }
    const accepted = exportIdentity(plan,profile,'combined',caps.verification!.defaultOptions,crypto.randomUUID());
    await service.startExport!(accepted); let task = await service.exportTask!(accepted);
    while (task.state === 'queued') task = await service.exportTask!(accepted);
    expect(task.state).toBe('running');
    expect((await service.validateDraft(job,id.revision)).valid).toBe(true);
    expect((await service.cancelExport!(accepted)).state).toBe('cancelling');
    expect((await finishExport(accepted)).state).toBe('cancelled');
    await expect(service.exportResult!(accepted)).rejects.toThrow(/PLAN_RESULT_UNAVAILABLE/);
  },60_000);
});

describe('live Rust/UI contract and CLI parity', () => {
  it('serves the offline production build with the API on the same origin', async () => {
    const response = await fetch(base);
    expect(response.status).toBe(200);
    expect(response.headers.get('content-security-policy')).toContain("connect-src 'self'");
    const html = await response.text();
    const script = html.match(/src="([^"]+\.js)"/)?.[1];
    expect(script).toBeTruthy();
    const asset = await fetch(new URL(script!, base));
    expect(asset.headers.get('content-type')).toContain('javascript');
    expect(asset.status).toBe(200);
    expect((await fetch(new URL('/Cargo.toml', base))).status).toBe(404);
  });
  it('imports Inkscape with exactly the CLI job, normalized rings, holes, and missing settings', async () => {
    const source = `${workspace}fixtures/m2/inkscape-export.svg`;
    const jsonPath = `${output}/cli-import.job.json`;
    runCli(['import', source, '--output', jsonPath]);
    const cliJob = JSON.parse(readFileSync(jsonPath, 'utf8'));
    const imported = await service.importArtwork!('inkscape-export.svg', readFileSync(source, 'utf8'), cliJob.import, 23);
    expect(imported.job).toEqual(cliJob);
    const inspection = JSON.parse(runCli(['validate-job', jsonPath])).inspection;
    expect(imported.display.engineVersion).toBe(inspection.engine_version);
    expect(imported.display.components).toEqual(inspection.geometry.sources.map((source: { id: string; source_id: string; label: string | null; geometry: { grid: { ticks_per_mm: number }; rings: { is_hole: boolean; points: { x: number; y: number }[] }[] } }) => ({
      id: source.id, label: source.label || source.source_id,
      rings: source.geometry.rings.map(ring => ({ hole: ring.is_hole, points: ring.points.map(point => ({ x: point.x / source.geometry.grid.ticks_per_mm, y: point.y / source.geometry.grid.ticks_per_mm })) })),
    })));
    expect(imported.missingMachiningFields).toEqual(inspection.missing_machining_fields);
    expect((await service.validateDraft(imported.job, 23)).documentFingerprint).toBe(imported.documentFingerprint);
  }, 15_000);
  it('opens configured jobs, migrates old schemas, and rejects future/invalid jobs', async () => {
    const original = readFileSync(`${workspace}fixtures/m4/finite-tip.json`, 'utf8');
    const opened = await service.openJob!(original, 4);
    expect(opened.job).toEqual(JSON.parse(original));
    expect(opened.missingMachiningFields).toEqual([]);
    const old = { ...opened.job, schema_version: 1 };
    expect((await service.openJob!(JSON.stringify(old), 5)).job).toEqual(opened.job);
    await expect(service.openJob!(JSON.stringify({ ...old, schema_version: 99 }), 6)).rejects.toThrow(/JOB_SCHEMA_VERSION/);
    opened.job.stock.thickness_mm = -1;
    const invalid = await service.validateDraft(opened.job, 7);
    expect(invalid.valid).toBe(false);
    expect(invalid.diagnostics[0].code).toBe('JOB_PARAMETER');
    await expect(service.openJob!(JSON.stringify(opened.job), 7)).rejects.toThrow(/JOB_PARAMETER/);
  });
  it('normalizes changed artwork placement and roundtrips the browser adapter snapshot through Rust', async () => {
    const options = { geometry_tolerance_mm: 0.001, ticks_per_mm: null, placement: { origin_mm: { x: 2, y: 3 }, scale: 1.4, rotation_deg: 27 } };
    const svg = '<svg xmlns="http://www.w3.org/2000/svg" width="30mm" height="20mm" viewBox="0 0 30 20"><rect id="plate" x="5" y="5" width="15" height="10"/></svg>';
    const imported = await service.importArtwork!('integration-plate.svg', svg, options, 8);
    expect(await service.displayFor(imported.job)).toEqual(imported.display);
    writeFileSync(`${output}/adapter.job.json`, JSON.stringify(imported.job));
    const inspection = JSON.parse(runCli(['validate-job', `${output}/adapter.job.json`])).inspection;
    expect(inspection.geometry.sources.map((source: { id: string }) => source.id)).toEqual(imported.display.components.map(component => component.id));
    const checked = await service.validateDraft(imported.job, 8);
    expect(checked.valid).toBe(true);
    expect(checked.missingMachiningFields).toEqual(inspection.missing_machining_fields);
    imported.job.name = 'changed';
    expect((await service.validateDraft(imported.job, 9)).documentFingerprint).not.toBe(checked.documentFingerprint);
  });
});

async function identity(fixture: string, stage: TaskIdentity['stage'] = 'endmill') {
  const opened = await service.openJob!(readFileSync(`${workspace}fixtures/${fixture}.json`, 'utf8'), 14);
  const caps = await service.capabilities();
  const id: TaskIdentity = { taskId: crypto.randomUUID(), instanceId: caps.planning!.instanceId,
    engineVersion: caps.engineVersion, revision: 14, documentFingerprint: opened.documentFingerprint, stage };
  return { job: opened.job, id };
}
async function finish(id: TaskIdentity, timeoutMs = 30_000) {
  const deadline = Date.now() + timeoutMs;
  let previous: PlanTask | null = null;
  while (Date.now() < deadline) {
    const task = await service.planTask!(id);
    if (previous) expect(task.sequence).toBeGreaterThanOrEqual(previous.sequence);
    if (terminal(task)) return task;
    previous = task;
    await new Promise(resolve => setTimeout(resolve, 20));
  }
  throw new Error('Planning did not finish within the test deadline');
}
describe('real background planning', () => {
  it.skipIf(process.env.CAM_TEST_LARGE_PLAN !== '1')('plans and reopens two flower copies beyond 128 MB', async () => {
    const original = JSON.parse(readFileSync(`${workspace}../real_data/flower_box-svg.job (2).json`, 'utf8'));
    const paths = (original.source.svg as string).match(/<path\b[\s\S]*?\/>/g)!;
    expect(paths.length).toBe(1);
    const svg = `<svg xmlns="http://www.w3.org/2000/svg" width="400mm" height="100mm" viewBox="0 0 400 100">${paths[0]}<g transform="translate(200 0)">${paths[0].replace(/\bid="[^"]*"/g, 'id="flower-copy"')}</g></svg>`;
    const imported = await service.importArtwork!('two-flowers.svg', svg, original.import, 50);
    const job = {...original, name:'Two flower copies: transport regression', source:imported.job.source,
      selected_region_ids:imported.job.selected_region_ids};
    writeFileSync(`${output}/two-flowers.job.json`, JSON.stringify(job));
    const checked = await service.validateDraft(job, 50);
    expect(checked.valid).toBe(true);
    const caps = await service.capabilities();
    const id:TaskIdentity = {taskId:crypto.randomUUID(), instanceId:caps.planning!.instanceId,
      engineVersion:caps.engineVersion, revision:50, documentFingerprint:checked.documentFingerprint!, stage:'combined'};
    const started = Date.now();
    await service.startPlan!(job, id);
    const task = await finish(id, 300_000);
    expect(task.state, JSON.stringify(task.diagnostic)).toBe('succeeded');
    expect(task.summary!.status).toBe('complete');
    const preview = await service.planResult!(id);
    expect(preview.motions.length).toBe(task.summary!.motionCount);
    expect(task.summary!.omittedMotionCount).toBe(0);
    const previewBytes = Buffer.byteLength(JSON.stringify(preview));
    const session = await (await fetch(`${base}/api/v1/session`)).json();
    const response = await fetch(`${base}/api/v1/tasks/${id.taskId}/artifact`, {headers:{'X-Cam-Session':session.sessionToken}});
    expect(response.status).toBe(200);
    const artifactBytes = Number(response.headers.get('content-length'));
    expect(artifactBytes).toBeGreaterThan(128_000_000);
    // Consume the download incrementally, just as the server produces it.
    const digest = createHash('sha256');
    let downloaded = 0;
    for await (const chunk of response.body!) { digest.update(chunk); downloaded += chunk.length; }
    expect(downloaded).toBe(artifactBytes);
    const planningMs = Date.now() - started;
    const measurement = {planningMs, previewBytes, artifactBytes, artifactSha256:digest.digest('hex'), motionCount:task.summary!.motionCount};
    console.info('Large plan generated and downloaded; reopening for verification:', measurement);
    const verification = verificationIdentity(task, {...caps.verification!.defaultOptions, max_cells:1}, crypto.randomUUID());
    await service.startVerification!(verification);
    const verified = await finishVerification(verification, 300_000);
    expect(verified.state, JSON.stringify(verified.diagnostic)).toBe('succeeded');
    expect((await service.verificationResult!(verification)).report.status).toBe('inconclusive');
    writeFileSync(`${output}/two-flowers-summary.json`, JSON.stringify({...measurement, verificationStatus:verified.summary?.status}, null, 2));
  }, 700_000);
  it.skipIf(process.env.CAM_TEST_REAL_DATA !== '1')('loads the complete unchanged real flower plan through bounded motion pages', async () => {
    const jobPath = `${workspace}../real_data/flower_box-svg.job (2).json`;
    const original = readFileSync(jobPath, 'utf8');
    const opened = await service.openJob!(original, 40);
    expect(opened.job).toEqual(JSON.parse(original));
    const svg = readFileSync(`${workspace}../real_data/flower_box.svg`, 'utf8');
    expect(opened.job.source.svg.trim()).toBe(svg.trim());
    const imported = await service.importArtwork!('flower_box.svg', svg, opened.job.import, 41);
    expect(imported.display.components).toEqual(opened.display.components);
    const caps = await service.capabilities();
    expect(caps.planning!.artifactBytes).toBeNull();
    const id: TaskIdentity = {taskId:crypto.randomUUID(), instanceId:caps.planning!.instanceId,
      engineVersion:caps.engineVersion, revision:40, documentFingerprint:opened.documentFingerprint, stage:'combined'};
    const started = Date.now();
    await service.startPlan!(opened.job, id);
    const task = await finish(id, 300_000);
    expect(task.state, JSON.stringify(task.diagnostic)).toBe('succeeded');
    expect(task.summary?.status).toBe('complete');
    expect(task.summary!.motionCount).toBeGreaterThan(20_000);
    const progress: number[] = [];
    const previewStarted = Date.now();
    const result = await service.planResult!(id, undefined, loaded => progress.push(loaded));
    const previewMs = Date.now() - previewStarted;
    expect(result.motions.length).toBe(task.summary!.motionCount);
    expect(task.summary!.omittedMotionCount).toBe(0);
    expect(progress.length).toBe(Math.ceil(task.summary!.motionCount / 20_000));
    expect(progress.at(-1)).toBe(task.summary!.motionCount);
    const previewBytes = Buffer.byteLength(JSON.stringify(result));
    const session = await (await fetch(`${base}/api/v1/session`)).json();
    const headers = { 'X-Cam-Session': session.sessionToken };
    const firstPage = await fetch(`${base}/api/v1/tasks/${id.taskId}/result`, { headers });
    expect(Buffer.byteLength(await firstPage.text())).toBeLessThan(16_000_000);
    for (const offset of [1, task.summary!.motionCount + 20_000]) {
      expect((await fetch(`${base}/api/v1/tasks/${id.taskId}/motions/${offset}`, { headers })).status).toBe(404);
    }
    expect((await fetch(`${base}/api/v1/tasks/${id.taskId}/motions/20000`)).status).toBe(401);
    const response = await fetch(`${base}/api/v1/tasks/${id.taskId}/artifact`, {headers:{'X-Cam-Session':session.sessionToken}});
    expect(response.status).toBe(200);
    const artifact = Buffer.from(await response.arrayBuffer());
    const recorded = JSON.parse(artifact.toString());
    const motions = [...recorded.endmill.motions, ...recorded.vbit_motions];
    // Compare every coordinate and field, in bounded batches so assertions do
    // not need an additional full-plan JSON string.
    for (let offset = 0; offset < motions.length; offset += 20_000) {
      expect(result.motions.slice(offset, offset + 20_000)).toEqual(motions.slice(offset, offset + 20_000));
    }
    expect(Number(response.headers.get('content-length'))).toBe(artifact.length);
    expect(artifact.length).toBeGreaterThan(100_000_000);
    const path = `${output}/real-flower.plan.json`;
    writeFileSync(path, artifact);
    const planningMs = Date.now() - started;
    const cliPath = `${output}/real-flower-cli.plan.json`;
    runCli(['plan', jobPath, '--stage', 'combined', '--output', cliPath]);
    const hash = (bytes: Buffer) => createHash('sha256').update(bytes).digest('hex');
    // CLI adds one final newline; all plan data must otherwise match exactly.
    expect(hash(Buffer.concat([artifact, Buffer.from('\n')]))).toBe(hash(readFileSync(cliPath)));
    // Reopen the disk-backed plan for independent verification. A deliberately
    // tiny verifier budget tests transport/reconstruction, not machining approval.
    const verification = verificationIdentity(task, {...caps.verification!.defaultOptions, max_cells:1}, crypto.randomUUID());
    await service.startVerification!(verification);
    const verified = await finishVerification(verification, 300_000);
    expect(verified.state, JSON.stringify(verified.diagnostic)).toBe('succeeded');
    expect((await service.verificationResult!(verification)).report.status).toBe('inconclusive');
    writeFileSync(`${output}/real-flower-summary.json`, JSON.stringify({planningMs, previewBytes, previewMs, motionPages: progress.length,
      artifactBytes:artifact.length, motionCount:task.summary!.motionCount, artifactSha256:hash(artifact),
      verificationStatus:verified.summary?.status}, null, 2));
  }, 900_000);
  it.each([
    ['m3/rectangle', 'endmill', 'complete'], ['m3/no-access', 'endmill', 'empty'],
    ['m3/unsupported-entry', 'endmill', 'incomplete'], ['m3/resource-limit', 'endmill', 'inconclusive'],
    ['m4/finite-tip', 'combined', 'complete'],
  ] as const)('matches CLI artifact and recorded motions for %s', async (fixture, stage, status) => {
    const { job, id } = await identity(fixture, stage);
    const accepted = await service.startPlan!(job, id);
    expect(accepted.state).toBe('queued');
    const task = await finish(id);
    expect(task.state, JSON.stringify(task.diagnostic)).toBe('succeeded');
    expect(task.summary?.status).toBe(status);
    const result = await service.planResult!(id);
    const path = `${output}/${fixture.replace('/', '-')}-${stage}.plan.json`;
    runCli(['plan', `${workspace}fixtures/${fixture}.json`, '--stage', stage, '--output', path], ['complete', 'empty'].includes(status) ? 0 : 1);
    const cliPlan = JSON.parse(readFileSync(path, 'utf8'));
    expect(task.summary?.inputFingerprint).toBe(cliPlan.input_fingerprint);
    expect(task.summary?.motionFingerprint).toBe(cliPlan.motion_fingerprint);
    const motions = stage === 'combined' ? [...cliPlan.endmill.motions, ...cliPlan.vbit_motions] : cliPlan.motions;
    expect(result.motions).toEqual(motions);
    expect(task.summary?.motionCount).toBe(motions.length);
    const session = await (await fetch(`${base}/api/v1/session`)).json();
    const artifact = await fetch(`${base}/api/v1/tasks/${id.taskId}/artifact`, { headers: { 'X-Cam-Session': session.sessionToken } });
    expect(await artifact.json()).toEqual(cliPlan);
    // The CLI rebuilds analysis independently from the saved artifact. Display
    // rings must be exactly that engine geometry, including holes and placement.
    const inspectPlan = (artifactPath: string, suffix: string, expectedStatus: number) => {
      const report = `${path}.${suffix}.report.json`;
      runCli(['inspect', artifactPath, '--output', `${path}.${suffix}.svg`, '--report', report], expectedStatus);
      return JSON.parse(readFileSync(report, 'utf8')).analysis;
    };
    const analysis = inspectPlan(path, 'analysis', ['complete', 'empty'].includes(status) ? 0 : 1);
    let endmillAnalysis = analysis;
    if (stage === 'combined') {
      const endmillPath = `${path}.endmill.json`;
      writeFileSync(endmillPath, JSON.stringify(cliPlan.endmill));
      endmillAnalysis = inspectPlan(endmillPath, 'endmill', 0);
    }
    const regionKeys = { nominalTarget: 'nominal_section', remainingTarget: 'remaining_target', possibleOvercut: 'possible_overcut',
      accessibleFloor: 'accessible_floor', missingFloor: 'missing_floor_beyond_tolerance', requestedCenters: 'requested_centers' };
    expect(result.stockSlices.length).toBe(endmillAnalysis.layers.length + (stage === 'combined' ? analysis.slices.length : 0));
    for (const info of result.stockSlices) {
      const index = Number(info.id.split('-')[1]);
      const core = info.stage === 'endmill' ? endmillAnalysis.layers[index] : analysis.slices[index];
      const display = await service.stockSlice!(id, info);
      expect(display.slice.info).toEqual(info);
      expect(info.depthMm).toBe(core.depth_mm);
      expect(info.capsuleRadialErrorMm).toBe(core.removal.capsule_radial_error_mm);
      expect(info.contributingMotionCount).toBe(core.removal.contributing_motion_ids.length);
      expect(display.slice.geometry).not.toBeNull();
      for (const region of display.slice.geometry!) {
        const actual = region.key === 'removedLower' ? core.removal.lower : region.key === 'removedUpper' ? core.removal.upper : core[regionKeys[region.key]];
        expect(region.rings).toEqual(actual.rings.map((ring: { is_hole: boolean; points: { x: number; y: number }[] }) => ({
          hole: ring.is_hole, points: ring.points.map(p => ({ x: p.x / actual.grid.ticks_per_mm, y: p.y / actual.grid.ticks_per_mm })),
        })));
      }
    }
    await expect(service.stockSlice!(id, { ...result.stockSlices[0], id: 'endmill-999' })).rejects.toThrow(/SLICE_NOT_FOUND/);
    expect((await service.cancelPlan!(id)).state).toBe('succeeded');
    expect((await service.startPlan!(job, id)).taskId).toBe(id.taskId); // Retry does not calculate again.
    await expect(service.startPlan!({ ...job, name: 'edited' }, id)).rejects.toThrow(/STALE_DOCUMENT/);
    await expect(service.startPlan!(job, { ...id, revision: id.revision + 1 })).rejects.toThrow(/TASK_KEY_REUSED/);
  }, 45_000);
  it('checks the chosen stage in Rust and preserves its setup diagnostic', async () => {
    const { job, id } = await identity('m3/rectangle', 'combined');
    await service.startPlan!(job, id);
    const task = await finish(id);
    expect(task.state).toBe('failed');
    expect(task.diagnostic?.code).toBeTruthy();
    expect(task.summary).toBeNull();
    await expect(service.planResult!(id)).rejects.toThrow(/PLAN_RESULT_UNAVAILABLE/);
  });
  it('keeps validation responsive, reconnects without replay, and cancels running work', async () => {
    const { job, id } = await identity('m4/finite-tip', 'combined');
    // Keep enough real work after the stock/sample reuse optimizations to cover
    // validation and reconnect roundtrips before cancelling the running worker.
    // The 30 x 20 mm target has about 667k samples, below the explicit ceiling.
    job.vbit_planning!.quality_sample_spacing_mm = 0.03;
    job.vbit_planning!.max_quality_samples = 1_000_000;
    id.documentFingerprint = (await service.validateDraft(job, id.revision)).documentFingerprint!;
    await service.startPlan!(job, id);
    let task = await service.planTask!(id);
    while (task.state === 'queued') task = await service.planTask!(id);
    expect(task.state).toBe('running');
    expect((await service.validateDraft(job, 15)).valid).toBe(true);
    await service.capabilities();
    const cancelled = await service.cancelPlan!(id);
    expect(cancelled.state).toBe('cancelling');
    expect((await finish(id)).state).toBe('cancelled');
    expect((await service.cancelPlan!(id)).state).toBe('cancelled');
    await expect(service.planResult!(id)).rejects.toThrow(/PLAN_RESULT_UNAVAILABLE/);
    await expect(service.planTask!({ ...id, instanceId: '0'.repeat(32) })).rejects.toThrow(/previous service instance/);
  }, 15_000);
});

async function finishVerification(id: VerificationIdentity, timeoutMs = 45_000) {
  const deadline = Date.now() + timeoutMs;
  while (Date.now() < deadline) {
    const task = await service.verificationTask!(id);
    if (['succeeded','failed','cancelled'].includes(task.state)) return task;
    await new Promise(resolve => setTimeout(resolve, 20));
  }
  throw new Error('Verification did not finish within the test deadline');
}
describe('real continuous verification', () => {
  it.each([
    ['original', null, 1_000_000, 'passed'],
    ['rounded', 0, 1_000_000, 'failed'],
    ['limited', null, 1, 'inconclusive'],
  ] as const)('matches every CLI report field for %s evidence', async (name, places, cells, status) => {
    const {job,id} = await identity('m4/narrow-channel','combined');
    await service.startPlan!(job,id);
    const plan = await finish(id);
    const caps = await service.capabilities();
    const options = {...caps.verification!.defaultOptions, max_cells:cells, decimal_places:places};
    const verification = verificationIdentity(plan,options,crypto.randomUUID());
    await expect(service.startVerification!({...verification, revision:verification.revision + 1})).rejects.toThrow(/VERIFICATION_PLAN_IDENTITY/);
    expect((await service.startVerification!(verification)).state).toBe('queued');
    await expect(service.startVerification!({...verification,verification:{...verification.verification,options:{...options,max_depth:1}}})).rejects.toThrow(/TASK_KEY_REUSED/);
    const task = await finishVerification(verification);
    expect(task.state,JSON.stringify(task.diagnostic)).toBe('succeeded');
    expect(task.summary?.status).toBe(status);
    const result = await service.verificationResult!(verification);
    const session = await (await fetch(`${base}/api/v1/session`)).json();
    const artifact = await fetch(`${base}/api/v1/tasks/${id.taskId}/artifact`, {headers:{'X-Cam-Session':session.sessionToken}});
    const path = `${output}/verify-${name}.plan.json`;
    writeFileSync(path,await artifact.text());
    const reportPath = `${output}/verify-${name}.report.json`;
    runCli(['verify',path,'--output',reportPath,'--max-cells',String(cells),...(places === null ? [] : ['--decimal-places',String(places)])],status === 'passed' ? 0 : 1);
    expect(result.report).toEqual(JSON.parse(readFileSync(reportPath,'utf8')).verification);
    expect((await service.cancelVerification!(verification)).state).toBe('succeeded');
    expect((await service.startVerification!(verification)).taskId).toBe(verification.taskId);
    await expect(service.planResult!(verification)).rejects.toThrow(/TASK_KIND/);
    await service.capabilities(); // Reconnect preserves the result and does not replay.
    expect((await service.verificationResult!(verification)).report).toEqual(result.report);
  },60_000);
  it('cancels a running verification while document checks remain responsive', async () => {
    const {job,id} = await identity('m4/island','combined');
    await service.startPlan!(job,id);
    const plan = await finish(id);
    const options = (await service.capabilities()).verification!.defaultOptions;
    const verification = verificationIdentity(plan,{...options,decimal_places:6},crypto.randomUUID());
    await service.startVerification!(verification);
    let task = await service.verificationTask!(verification);
    while (task.state === 'queued') task = await service.verificationTask!(verification);
    expect(task.state).toBe('running');
    expect((await service.validateDraft(job,id.revision)).valid).toBe(true);
    expect((await service.cancelVerification!(verification)).state).toBe('cancelling');
    expect((await finishVerification(verification)).state).toBe('cancelled');
    await expect(service.verificationResult!(verification)).rejects.toThrow(/PLAN_RESULT_UNAVAILABLE/);
  },45_000);
});
