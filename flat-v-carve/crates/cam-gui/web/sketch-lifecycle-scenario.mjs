// Milestone 4: solve -> atomic Apply -> repair -> mixed-source checked output.
import {isDeepStrictEqual} from 'node:util';
import {readFileSync, existsSync} from 'node:fs';
import {createHash} from 'node:crypto';

export async function sketchLifecycleScenario({control,state,waitFor,send,evaluate,sleep,record,screenshot,pressKey,click,path,out,chooseFile}) {
  const assert=(v,m)=>{if(!v)throw Error(m);};
  const idle=()=>waitFor(s=>!s.active&&!s.pending,'idle document');
  const button=async label=>{
    for(let i=0;i<12;i++) {
      const s=await state(),r=s.controls[label];assert(r,`Missing ${label}`);
      const p=s.controls['Constraint panel'];
      if(p&&!/^(Sketch |Apply sketch|Cancel sketch|Constraints & dimensions|Edit sketch)/.test(label)&&(r[1]<p[1]||r[3]>p[3])) {
        const x=(r[0]+r[2])/2,y=(p[1]+p[3])/2;
        await send('Input.dispatchMouseEvent',{type:'mouseMoved',x,y});
        await send('Input.dispatchMouseEvent',{type:'mouseWheel',x,y,deltaX:0,deltaY:r[1]<p[1]?-180:180});await sleep(200);continue;
      }
      await click((r[0]+r[2])/2,(r[1]+r[3])/2);await sleep(150);return;
    }
    throw Error(`Cannot reveal ${label}`);
  };
  const choice=async suffix=>{
    const label=Object.keys((await state()).controls).find(k=>k.endsWith(suffix));
    assert(label,`Missing contour choice ${suffix}`);await control(label);
  };
  const download=async filename=>{
    const file=path.join(out,filename);
    for(let i=0;i<80&&!existsSync(file);i++)await sleep(150);
    assert(existsSync(file),`Missing download ${filename}`);return readFileSync(file);
  };
  await control('File');await chooseFile('Open job',path.resolve('fixtures/v5/sketch-lifecycle-job.json'));
  await waitFor(s=>!s.active&&s.job?.artworks?.length===2,'mixed-source fixture');
  await control('Operation row profile-1');await control('Operation Tabs & entry');
  await control('Anchor the start');await choice('contour-plate · 25%');await idle();
  await control('Generation scope');await control('All enabled operations');
  await control('Generate');await waitFor(s=>s.current&&!s.active,'initial mixed-source plan');
  await control('Prepare checked output');await waitFor(s=>s.prepared&&!s.active,'initial checked output');
  await control('Close export');
  const original=await state();assert(original.job.start.kind==='anchor','start not anchored');
  record('SVG and sketch contours share a generated profile; SVG knife also planned',original.planScope);

  await control('Artwork sketch');await control('Edit sketch');
  await waitFor(s=>s.sketch&&!s.sketch.solving,'sketch solve');
  if(!(await state()).sketch.show_constraints)await button('Constraints & dimensions');await button('Constraint row 9');await button('Constraint value');
  await pressKey('a','KeyA',2);await send('Input.insertText',{text:'25'});await sleep(200);await pressKey('Tab','Tab');
  await button('Update constraint');
  await waitFor(s=>s.sketch&&!s.sketch.solving&&s.sketch.solver.accepted,'resized sketch');
  await button('Apply sketch');
  let s=await waitFor(s=>!s.sketch&&!s.active&&s.sketchImpact,'applied sketch report');
  assert(s.undo===original.undo+1,'Apply was not one transaction');
  assert(!s.current&&!s.prepared,'old plan or prepared bytes survived Apply');
  assert(s.sketchImpact.bindings.filter(b=>b.change==='preserved').length===1,'profile selection not preserved');
  assert(s.sketchImpact.bindings.filter(b=>b.change==='boundary_changed').length===1,'anchor change not reported');
  assert(isDeepStrictEqual(s.job.artworks[0],original.job.artworks[0]),'SVG source changed');
  assert(isDeepStrictEqual(s.job.profile.contours[0],original.job.profile.contours[0]),'SVG selection changed');
  assert(isDeepStrictEqual(s.job.start,original.job.start),'anchor was silently rebound');
  const changed=structuredClone(s.job);
  await screenshot('sketch-apply-impact.png');
  await control('Repair sketch binding 2');await sleep(250);
  s=await state();assert(s.controls['Unresolved start anchor'],'repair did not reveal the anchor');
  await screenshot('sketch-anchor-repair.png');
  await control('Undo');await idle();s=await state();
  assert(isDeepStrictEqual(s.job.artworks,original.job.artworks)&&isDeepStrictEqual(s.job.profile,original.job.profile),'undo lost geometry or bindings');
  await control('Redo');await idle();s=await state();
  assert(isDeepStrictEqual(s.job.artworks,changed.artworks)&&isDeepStrictEqual(s.job.profile,changed.profile),'redo lost geometry or bindings');
  assert(!s.current&&!s.prepared,'redo admitted stale output');
  await control('Operation Tabs & entry');await control('Reattach start anchor');await choice('/ contour-plate');await idle();
  s=await state();assert(isDeepStrictEqual(s.job.start.geometry.source_revision,s.job.profile.contours[1].geometry.source_revision),'explicit repair did not rebind');
  record('dimension Apply preserves selections, requires anchor repair and supports atomic undo/redo',s.job.profile);

  await control('Generate');await waitFor(s=>s.current&&!s.active,'regenerated mixed-source job');
  await control('Prepare checked output');s=await waitFor(s=>s.prepared&&!s.active,'regenerated checked output');
  const digest=s.preparedSha256;
  await evaluate('globalThis.showSaveFilePicker=undefined');await control('Save as…');
  await waitFor(s=>s.status.includes('Download requested'),'G-code download');
  const gcode=await download('sequence.ngc');
  assert(createHash('sha256').update(gcode).digest('hex')===digest,'G-code differs from checked bytes');
  await screenshot('sketch-mixed-checked-output.png');
  if((await state()).controls['Close export'])await control('Close export');
  await control('Artwork sketch');
  await evaluate('globalThis.showSaveFilePicker=async()=>{throw new Error("SVG destination unavailable")}');
  await control('Export artwork SVG');await waitFor(s=>s.status.includes('SVG destination unavailable'),'denied SVG destination');
  await evaluate('globalThis.showSaveFilePicker=undefined');await control('Retry previous save');
  await waitFor(s=>s.status.includes('Download requested'),'retried SVG download');
  const svg=(await download('sketch.svg')).toString();
  assert(svg.includes('width="40mm"')&&svg.includes('fill-rule="evenodd"')&&svg.includes('fill="none"'),'SVG omitted units, holes or open geometry');
  await control('Save job');await waitFor(s=>s.status.includes('Download requested'),'saved mixed-source job');
  const saved=JSON.parse((await download('carving.gui2.job.json')).toString());
  assert(saved.artwork[1].content.constraints[8].definition.value_mm===25,'editable dimension was not saved');
  await control('File');await chooseFile('Open job',path.join(out,'carving.gui2.job.json'));await idle();
  s=await state();assert(isDeepStrictEqual(s.job.artworks,saved.artwork),'mixed-source save/reopen changed artwork');
  assert(isDeepStrictEqual(s.job.profile,saved.operations[0].settings.settings),'save/reopen changed references');
  record('checked G-code bytes, SVG save failure/retry, editable mixed-source save/reopen',{digest,svgBytes:svg.length});
}
