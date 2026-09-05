import { describe, expect, it } from 'vitest';
import { readFileSync } from 'node:fs';
import { renderToStaticMarkup } from 'react-dom/server';
import { verificationReportSchema, verificationResultSchema, verificationIdentity, acceptVerification, currentVerification, type VerificationTask } from '../src/contracts/verification';
import { taskSchema } from '../src/contracts/planning';
import { apiVersion } from '../src/contracts/wire';
import { createHttpService } from '../src/service/http';
import { parseVerificationOptions } from '../src/service/useVerification';
import { fixtureService } from '../src/service/fixture';
import { Viewport } from '../src/components/Viewport';

// Captured from the same-engine CLI, with a deliberately exhausted cell budget.
const captured = JSON.parse(readFileSync(new URL('./fixtures/m5-limited.json', import.meta.url),'utf8'));
const report = verificationReportSchema.parse(captured.report);
const plan = taskSchema.parse({apiVersion,engineVersion:report.engine_version,instanceId:'a'.repeat(32),taskId:'source-plan',revision:3,
  documentFingerprint:'b'.repeat(64),stage:'combined',sequence:4,state:'succeeded',resultAvailable:true,diagnostic:null,
  summary:{engineVersion:report.engine_version,status:'complete',inputFingerprint:captured.planInputFingerprint,motionFingerprint:captured.planMotionFingerprint,
    meaning:'M4 planning',limitations:[],motionCount:0,cuttingMotionCount:0,previewMotionCount:0,omittedMotionCount:0,diagnostics:[],omittedDiagnostics:0,generationIssues:[],omittedGenerationIssues:0}});
const identity = verificationIdentity(plan,report.options,'verification-task');
const task: VerificationTask = {...identity,apiVersion,sequence:5,state:'succeeded',resultAvailable:true,diagnostic:null,
  summary:{engineVersion:report.engine_version,status:report.status,verificationFingerprint:report.verification_fingerprint,
    originalStatus:report.original.status,roundedStatus:null}};
const result = verificationResultSchema.parse({task,coordinateSpace:'workpiece-mm-z-up',report});

describe('verification evidence and freshness', () => {
  it('preserves an inconclusive outcome and rejects contradictory bounds, scope, or report identities', () => {
    expect(result.task.state).toBe('succeeded');
    expect(result.report.status).toBe('inconclusive');
    expect(result.report.original.unresolved_cells).toBeGreaterThan(0);
    expect(verificationReportSchema.safeParse({...report,status:'passed'}).success).toBe(false);
    expect(verificationReportSchema.safeParse({...report,options:{...report.options,decimal_places:0}}).success).toBe(false);
    const wrong = structuredClone(report); wrong.original.bounds.overcut_mm = {lower:2,upper:1};
    expect(verificationReportSchema.safeParse(wrong).success).toBe(false);
    expect(verificationResultSchema.safeParse({...result,report:{...report,verification_fingerprint:'f'.repeat(64)}}).success).toBe(false);
    expect(verificationReportSchema.safeParse({...report,authenticated_plan_fingerprint:null}).success).toBe(false);
  });
  it('invalidates report locations for edits, pending validation, different plans, settings, and services', () => {
    expect(currentVerification(result,plan,true,report.options)).toBe(true);
    for (const current of [false]) expect(currentVerification(result,plan,current,report.options)).toBe(false);
    expect(currentVerification(result,plan,true,null)).toBe(false);
    expect(currentVerification(result,plan,true,{...report.options,max_cells:2})).toBe(false);
    expect(currentVerification(result,plan,true,{...report.options,decimal_places:0})).toBe(false);
    for (const changed of [{...plan,revision:4},{...plan,taskId:'new-plan'}, {...plan,instanceId:'c'.repeat(32)}, {...plan,engineVersion:'new'},
      {...plan,summary:{...plan.summary!,motionFingerprint:'d'.repeat(64)}}]) expect(currentVerification(result,changed,true,report.options)).toBe(false);
  });
  it('rejects reordered terminal transitions and responses from another plan or option set', () => {
    expect(acceptVerification(task,{...task,sequence:4},identity)).toBe(task);
    expect(() => acceptVerification(task,{...task,sequence:6,state:'running'},identity)).toThrow(/finished verification/);
    expect(() => acceptVerification(null,task,{...identity,verification:{...identity.verification,planTaskId:'other'}})).toThrow(/different task, plan, or settings/);
    expect(() => acceptVerification(null,task,{...identity,verification:{...identity.verification,options:{...report.options,max_depth:1}}})).toThrow(/different task, plan, or settings/);
  });
  it('keeps zero decimal places distinct from blank and rejects unfinished or out-of-range settings', () => {
    const text = {max_cells:'1',max_depth:'24',reachability_max_cells:'4096',max_depth_bands:'512',max_findings:'64',decimal_places:''};
    expect(parseVerificationOptions(text)?.decimal_places).toBeNull();
    expect(parseVerificationOptions({...text,decimal_places:'0'})?.decimal_places).toBe(0);
    expect(parseVerificationOptions({...text,max_cells:''})).toBeNull();
    expect(parseVerificationOptions({...text,max_cells:'1e'})).toBeNull();
    expect(parseVerificationOptions({...text,max_cells:'2000001'})).toBeNull();
  });
  it('renders reported finding locations without applying source placement again', async () => {
    const {job,display} = await fixtureService.openExample();
    job.import.placement = {origin_mm:{x:40,y:20},scale:4,rotation_deg:30};
    const finding = report.original.findings[0];
    const markup = renderToStaticMarkup(<Viewport job={job} display={display} inspected={null} onInspect={() => {}} hidden={new Set()} verificationFinding={finding} />);
    expect(markup).toContain(`data-verification-finding="${finding.code}"`);
    expect(markup).toContain(`cx="${finding.location.x}" cy="${-finding.location.y}"`);
  });
});

describe('verification HTTP boundary', () => {
  it('checks the accepted report and rejects substituted settings before they reach the UI', async () => {
    let response = result;
    const service = createHttpService(async input => {
      const url = String(input);
      if (url.endsWith('session')) return Response.json({apiVersion,engineVersion:report.engine_version,sessionToken:'e'.repeat(64)});
      if (url.endsWith('capabilities')) return Response.json({apiVersion,mode:'live',engineVersion:report.engine_version,
        importArtwork:true,openJob:true,validateDraft:true,planningStages:['combined'],verificationScopes:['continuous-stock'],exportFormats:[],
        verification:{defaultOptions:report.options}, limits:{svgBytes:32_000_000,jobBytes:64_000_000,requestBytes:128_100_000,concurrentInspections:2},
        planning:{instanceId:plan.instanceId,concurrentPlans:1,maxPending:4,maxTasks:128,retainedResults:4,timeoutSeconds:300,previewMotions:20_000,artifactBytes:16_000_000,stockSlices:true,sliceVertices:60_000,inspectionVertices:200_000}});
      expect(url).toBe('/api/v1/tasks/verification-task/verification');
      return Response.json(response);
    });
    await service.capabilities();
    expect((await service.verificationResult!(identity)).report).toEqual(report);
    response = {...result,task:{...task,verification:{...task.verification,planTaskId:'other-plan'}}};
    await expect(service.verificationResult!(identity)).rejects.toThrow(/different task, plan, or settings/);
  });
});
