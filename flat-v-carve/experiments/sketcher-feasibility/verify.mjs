import assert from 'node:assert/strict';
import {readFile,writeFile,mkdir} from 'node:fs/promises';
import {createHash} from 'node:crypto';
import os from 'node:os';
import {summarize,tolerance,residuals} from './validate.mjs';
const root=new URL('../../artifacts/sketcher-feasibility/',import.meta.url);
const read=async name=>JSON.parse((await readFile(new URL(name,root),'utf8')).replace(/^\uFEFF/,''));
const fixtures=JSON.parse(await readFile(new URL('./fixtures.json',import.meta.url)));
const output={date:'2026-10-11',host:{cpu:os.cpus()[0].model,logical_cpus:os.cpus().length,memory_bytes:os.totalmem(),os:os.release()},tolerance_mm:tolerance,fixtures:fixtures.map(f=>({id:f.id,entities:f.entities.length,constraints:f.constraints.length,expected:f.expected})),runs:{},parity:{},diagnostics:{},packages:{}};
const rawRuns={};
for(const backend of ['arael','planegcs'])for(const platform of ['native','browser']) {
  const key=`${backend}-${platform}`,file=await read(`${key}.json`),raw=file.results??file;
  rawRuns[key]=raw;const summary=raw.map(r=>summarize(fixtures.find(f=>f.id===r.id),r));
  const failures=file.failures??[];
  assert.deepEqual([...raw.map(r=>r.id),...failures.map(r=>r.id)].sort(),fixtures.map(f=>f.id).sort(),`${key}: stale/missing/duplicate cases`);
  for(const r of summary) {
    if(r.expected==='valid') {
      assert(r.accepted,`${key}/${r.id}: residual failed`);assert(r.dof_ok,`${key}/${r.id}: DOF failed`);
      assert(r.warm_max_residual_mm<=tolerance,`${key}/${r.id}: warm residual failed`);
      assert(r.drag_max_residual_mm<=tolerance,`${key}/${r.id}: driving constraint lost during drag`);
      if(r.id==='drag-length'){assert.equal(r.released_dof,1);assert(r.drag_max_direction_error_rad<.001);}
    } else if(['invalid','diagnostic'].includes(r.expected))assert(!r.accepted,`${key}/${r.id}: invalid geometry accepted`);
  }
  // Characterize the known published WASM allocation limit; fail on new errors.
  assert.deepEqual(failures.map(f=>f.id),key==='planegcs-browser'?['scale-500']:[]);
  output.runs[key]={summary,failures,platform:file.platform,loads_ms:file.loads_ms,heap_bytes:raw.filter(r=>r.heap_bytes).map(r=>({id:r.id,bytes:r.heap_bytes}))};
  if(file.geometry){assert(file.geometry.passed);output.geometry_browser=file.geometry;}
}
for(const backend of ['arael','planegcs']) {
  const comparisons=[];
  for(const f of fixtures.filter(f=>f.expected==='valid')) {
    const a=rawRuns[`${backend}-native`].find(r=>r.id===f.id),b=rawRuns[`${backend}-browser`].find(r=>r.id===f.id);
    if(!b)continue;
    const delta=(a,b)=>Math.max(0,...a.flat().map((v,i)=>Math.abs(v-b.flat()[i])));
    const cold=delta(a.values,b.values),warm=Math.max(0,...a.warm.map((w,i)=>delta(w.values,b.warm[i].values)));
    if(f.expected_dof===0){assert(cold<=tolerance,`${backend}/${f.id}: solution branch differs`);assert(warm<=tolerance);}
    comparisons.push({id:f.id,cold_max_parameter_delta:cold,warm_max_parameter_delta:warm,determined:f.expected_dof===0});
  }
  output.parity[backend]=comparisons;
  // In these two-constraint cases, successful single deletions identify repair choices.
  const base=output.runs[`${backend}-native`].summary;
  output.diagnostics[backend]=['redundant','conflicting'].map(id=>{
    const raw=rawRuns[`${backend}-native`].find(r=>r.id===id);
    return {id,status:raw.status,dof:base.find(r=>r.id===id).dof,accepted:base.find(r=>r.id===id).accepted,backend_conflicting_tags:raw.conflicting,backend_redundant_tags:raw.redundant,repair_candidates:fixtures.filter(f=>f.id.startsWith(`${id}-remove-`)).filter(f=>base.find(r=>r.id===f.id).accepted).map(f=>fixtures.find(x=>x.id===id).constraints[Number(f.id.at(-1))].id)};
  });
}
output.geometry_native=await read('geometry-native.json');assert.deepEqual(output.geometry_native,output.geometry_browser);
output.browser_lifecycle=await read('browser-lifecycle.json');assert(output.browser_lifecycle.stale_rejected&&output.browser_lifecycle.document_guard&&output.browser_lifecycle.sketch_guard);assert(output.browser_lifecycle.cancel.every(r=>r.ui_ticks>0));
output.native_lifecycle=await read('native-lifecycle.json');assert(output.native_lifecycle.results.every(r=>r.process_killed));
for(const [name,path] of [['arael_probe','./pkg/sketcher_feasibility_bg.wasm'],['plane_wasm','../../artifacts/sketcher-feasibility/upstream/plane-package/package/dist/planegcs_dist/planegcs.wasm']]) {
  const data=await readFile(new URL(path,import.meta.url));output.packages[name]={bytes:data.length,sha256:createHash('sha256').update(data).digest('hex')};
}
// Validator sanity checks: success strings must never override physical residuals.
const f=fixtures.find(f=>f.id==='point-dimensions');assert(residuals(f,[[0,0],[10,20]]).every(r=>r.error<=tolerance));assert(residuals(f,[[0,0],[-10,-20]]).some(r=>r.error>tolerance));assert(residuals(f,[[0,0],[null,20]]).some(r=>!Number.isFinite(r.error)));
await mkdir(new URL('./results/',import.meta.url),{recursive:true});
await writeFile(new URL('./results/qualification.json',import.meta.url),JSON.stringify(output,null,2)+'\n');
console.log(`Verified ${fixtures.length} cases per backend/platform, including expected failures; geometry, parity, drag, and lifecycle contracts pass. PlaneGCS browser 500-entity OOM remains a qualification failure.`);
