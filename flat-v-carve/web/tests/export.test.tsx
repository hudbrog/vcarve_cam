import { describe, expect, it } from 'vitest';
import { readFileSync } from 'node:fs';
import { renderToStaticMarkup } from 'react-dom/server';
import { apiVersion } from '../src/contracts/wire';
import { taskSchema } from '../src/contracts/planning';
import { acceptExport, checkExportBytes, currentExport, exportResultSchema, profileSchema, type ExportIdentity } from '../src/contracts/export';
import { parseProfileDraft, profileDraft, recoverProfile, reviewedProfile } from '../src/state/profile';
import { createHttpService } from '../src/service/http';
import { ExportPanel } from '../src/components/ExportPanel';
import type { useExport } from '../src/service/useExport';
import { parseJob } from '../src/contracts/job';

const captured = JSON.parse(readFileSync(new URL('./fixtures/m6-passed.json',import.meta.url),'utf8'));
const failed = exportResultSchema.parse(JSON.parse(readFileSync(new URL('./fixtures/m6-coarse.json',import.meta.url),'utf8')).result);
const plan = taskSchema.parse(captured.plan);
const result = exportResultSchema.parse(captured.result);
const {profile,layout,options} = result.task.export;
const job = parseJob(JSON.parse(readFileSync(new URL('../../fixtures/m4/narrow-channel.json',import.meta.url),'utf8')));

describe('LinuxCNC profile and checked output', () => {
  it('roundtrips all profile contract variants and preserves unfinished text without machine defaults', () => {
    const table = profileSchema.parse(JSON.parse(readFileSync(new URL('../../fixtures/m6/tool-table-synthetic.json',import.meta.url),'utf8')));
    for (const p of [profile,table,{...table,m6:{...table.m6,return_position:{kind:'caller_position' as const}}}])
      expect(parseProfileDraft(profileDraft(p)).profile).toEqual(p);
    const blank = recoverProfile({getItem:() => null});
    expect(parseProfileDraft(blank).profile).toBeNull(); expect(reviewedProfile(parseProfileDraft(blank).profile)).toBe(false);
    const unfinished = {...profileDraft(profile),clearance_z_mm:'1e-'};
    expect(recoverProfile({getItem:() => JSON.stringify(unfinished)})).toEqual(unfinished);
    expect(parseProfileDraft(unfinished).profile).toBeNull();
    expect(reviewedProfile({...profile,m6:{...profile.m6,reviewed:false}})).toBe(false);
    expect(reviewedProfile({...profile,m6:{...profile.m6,reference:' '}})).toBe(false);
    expect(profileSchema.safeParse({...profile,unknown:'G0 Z0'}).success).toBe(false);
  });
  it('invalidates program downloads for job edits, plan changes, profile/precision/layout/budget changes', () => {
    expect(currentExport(result,plan,true,profile,layout,options)).toBe(true);
    expect(currentExport(result,plan,false,profile,layout,options)).toBe(false);
    expect(currentExport(result,plan,true,null,layout,options)).toBe(false);
    expect(currentExport(result,plan,true,profile,'per_tool',options)).toBe(false);
    expect(currentExport(result,plan,true,profile,layout,{...options,max_cells:1})).toBe(false);
    for (const patch of [{work_offset:'G55' as const},{decimal_places:0},{m6:{...profile.m6,reviewed:false}},
      {tools:[{...profile.tools[0],tool_number:99},profile.tools[1]]}])
      expect(currentExport(result,plan,true,{...profile,...patch},layout,options)).toBe(false);
    for (const patch of [{revision:plan.revision+1},{taskId:'other'},{instanceId:'b'.repeat(32)},{engineVersion:'other'}])
      expect(currentExport(result,{...plan,...patch},true,profile,layout,options)).toBe(false);
  });
  it('distinguishes job IDs from machine numbers before export, including recovered and changed jobs', () => {
    const draft = profileDraft(profile);
    expect(parseProfileDraft(draft,job).profile).toEqual(profile);
    const wrongIds = {...draft,'tools.0.tool_id':'1','tools.1.tool_id':'2'};
    const recovered = recoverProfile({getItem:()=>JSON.stringify(wrongIds)});
    const invalid = parseProfileDraft(recovered,job);
    expect(invalid.profile).toBeNull();
    expect(invalid.errors['tools.0.tool_id']).toContain('Select endmill “endmill” or V-bit “vbit”');
    expect(invalid.errors['tools.1.tool_id']).toContain('enter the machine number in the T field');
    expect(recovered).toEqual(wrongIds);
    expect(parseProfileDraft({...draft,'tools.1.tool_id':'endmill'},job).errors['tools.1.tool_id']).toContain('mapped twice');
    expect(parseProfileDraft({...draft,'tools.1.tool_number':'01'},job).errors['tools.1.tool_number']).toContain('T1 is assigned twice');
    const changedJob = {...job,operation:{...job.operation,endmill_id:'cutter-a',vbit_id:'cutter-b'}};
    expect(parseProfileDraft(draft,changedJob).profile).toBeNull();
    const numericIds = {...job,operation:{...job.operation,endmill_id:'1',vbit_id:'2'}};
    expect(parseProfileDraft(wrongIds,numericIds).profile?.tools.map(t=>t.tool_id)).toEqual(['1','2']);
    expect(parseProfileDraft(profileDraft({...profile,tools:[profile.tools[1],profile.tools[0]]}),job).profile).not.toBeNull();
  });
  it('requires measured-length H entries only in tool-table mode and preserves them across mode changes', () => {
    const draft = {...profileDraft(profile),length_compensation:'tool_table','tools.0.length_offset_number':'1','tools.1.length_offset_number':'2'};
    expect(parseProfileDraft(draft,job).profile?.tools.map(t=>t.length_offset_number)).toEqual([1,2]);
    for (const h of ['', '0', '100000', '1e-']) {
      const invalid = parseProfileDraft({...draft,'tools.1.length_offset_number':h},job);
      expect(invalid.profile).toBeNull();
      expect(invalid.errors['tools.1.length_offset_number']).toContain('Tool-table compensation requires an H number');
    }
    const macro = {...draft,length_compensation:'macro_managed'};
    expect(parseProfileDraft(macro,job).profile?.tools.map(t=>t.length_offset_number)).toEqual([null,null]);
    expect(macro['tools.0.length_offset_number']).toBe('1');
    expect(parseProfileDraft({...macro,length_compensation:'tool_table'},job).profile?.tools.map(t=>t.length_offset_number)).toEqual([1,2]);
  });
  it('checks exact bytes and authenticates the source plan using the core report identity', async () => {
    await expect(checkExportBytes(result)).resolves.toBeUndefined();
    await expect(checkExportBytes(failed)).resolves.toBeUndefined();
    const changed = structuredClone(result); changed.programs[0].gcode += '\n';
    await expect(checkExportBytes(changed)).rejects.toThrow(/SHA-256/);
    await expect(checkExportBytes({...result,reportJson:result.reportJson+'\n'})).rejects.toThrow(/report bytes/);
    const other = structuredClone(result); other.task.export.motionFingerprint = 'f'.repeat(64);
    await expect(checkExportBytes(other)).rejects.toThrow(/report bytes/);
  });
  it('preserves a passed original and failed emitted check without releasing programs', () => {
    expect(failed.task.state).toBe('succeeded'); expect(failed.report.plan_verification.status).toBe('passed');
    expect(failed.report.status).toBe('failed'); expect(failed.programs).toHaveLength(0);
    expect(exportResultSchema.safeParse({...failed,programs:result.programs}).success).toBe(false);
    expect(exportResultSchema.safeParse({...result,programs:[]}).success).toBe(false);
    const contradiction = structuredClone(result); contradiction.report.emitted_verification!.status = 'failed';
    expect(exportResultSchema.safeParse(contradiction).success).toBe(false);
    expect(acceptExport(result.task,{...result.task,sequence:1},result.task)).toBe(result.task);
    expect(() => acceptExport(result.task,{...result.task,sequence:99,state:'running'},result.task)).toThrow(/finished export/);
  });
  it('rejects substituted task profiles and corrupted program bodies at the HTTP boundary', async () => {
    let response = result;
    const service = createHttpService(async url => {
      if (String(url).endsWith('session')) return Response.json({apiVersion,engineVersion:plan.engineVersion,sessionToken:'e'.repeat(64)});
      if (String(url).endsWith('capabilities')) return Response.json({apiVersion,mode:'live',engineVersion:plan.engineVersion,
        importArtwork:true,openJob:true,validateDraft:true,planningStages:['combined'],verificationScopes:['continuous-stock'],exportFormats:['linuxcnc'],
        export:{profileBytes:64_000,programBytes:8_000_000,layouts:['combined','per_tool']}, verification:{defaultOptions:options},
        limits:{svgBytes:32_000_000,jobBytes:64_000_000,requestBytes:128_100_000,concurrentInspections:2},
        planning:{instanceId:plan.instanceId,concurrentPlans:1,maxPending:4,maxTasks:128,retainedResults:4,timeoutSeconds:300,previewMotions:20_000,artifactBytes:16_000_000,stockSlices:true,sliceVertices:60_000,inspectionVertices:200_000}});
      return Response.json(response);
    });
    await service.capabilities();
    expect((await service.exportResult!(result.task)).programs).toEqual(result.programs);
    const identity:ExportIdentity = {...result.task,export:{...result.task.export,layout:'per_tool'}};
    await expect(service.exportResult!(identity)).rejects.toThrow(/different plan, profile, layout/);
    response = structuredClone(result); response.programs[0].gcode = 'G0 Z-100';
    await expect(service.exportResult!(result.task)).rejects.toThrow(/SHA-256/);
  });
  it('keeps stale previews labeled and their program buttons disabled, while keeping reports downloadable', () => {
    const output:ReturnType<typeof useExport> = {draft:profileDraft(profile),setDraft:()=>{},profile,errors:{},layout,setLayout:()=>{},recoveryError:'',options:{...options,decimal_places:null},
      available:true,canStart:true,current:false,downloadable:false,active:false,submitting:false,task:result.task,result,error:'',lost:false,
      start:()=>{},cancel:async()=>{},check:()=>{},retry:null,label:'Previous output stale'};
    const markup = renderToStaticMarkup(<ExportPanel output={output} job={job} planCurrent={true} />);
    expect(markup).toContain('Previous output · stale');
    expect(markup).toMatch(/disabled="">Download combined.ngc/);
    expect(markup).toContain('Download export report');
    expect(markup).toContain('<select id="profile-tools.0.tool_id"');
    expect(markup).toContain('Endmill · endmill');
    expect(markup).toContain('V-bit · vbit');
    expect(markup).toContain('the exported profile contains no H mappings');
    expect(markup).not.toContain('id="profile-tools.0.length_offset_number"');

    const draft = {...profileDraft(profile),length_compensation:'tool_table','tools.0.tool_id':'1'};
    const parsed = parseProfileDraft(draft,job);
    const invalidMarkup = renderToStaticMarkup(<ExportPanel output={{...output,draft,...parsed,canStart:false}} job={job} planCurrent={true} />);
    expect(invalidMarkup).toContain('<option value="1" disabled="" selected="">1 · unavailable</option>');
    expect(invalidMarkup).toContain('id="profile-tools.0.tool_id-error"');
    expect(invalidMarkup).toContain('id="profile-tools.0.length_offset_number-error"');
    expect(invalidMarkup).toContain('Both tools need an H number');
  });
});
