// Pointer/keyboard acceptance through the real browser solver Worker.
import {isDeepStrictEqual} from 'node:util';
import {readFileSync,existsSync} from 'node:fs';
export async function constraintScenario({control,state,waitFor,send,evaluate,sleep,record,screenshot,pressKey,click,path,out,chooseFile}) {
  const assert=(v,m)=>{if(!v)throw Error(m);};
  const model=s=>s.workspace.sketch.model;
  const settled=()=>waitFor(s=>s.sketch&&!s.sketch.solving&&!s.sketch.pending,'constraint solve');
  const wheel=async(r,deltaY)=>{const x=(r[0]+r[2])/2,y=(r[1]+r[3])/2;await send('Input.dispatchMouseEvent',{type:'mouseMoved',x,y});await send('Input.dispatchMouseEvent',{type:'mouseWheel',x,y,deltaX:0,deltaY});await sleep(250);};
  const button=async label=>{
    for(let i=0;i<12;i++) {
      const s=await state(),r=s.controls[label];assert(r,`Missing ${label}`);
      const p=s.controls[label.startsWith('Constraint kind ')?'Constraint choices':'Constraint panel'];
      const toolbar=/^(Sketch |Apply sketch|Cancel sketch|Constraints & dimensions|New sketch|Edit sketch)/.test(label);
      if(!toolbar&&p&&r[0]>=p[0]&&(r[1]<p[1]||r[3]>p[3])) {
        await wheel([r[0],p[1],r[2],p[3]],r[1]<p[1]?-180:180);continue;
      }
      await click((r[0]+r[2])/2,(r[1]+r[3])/2);await sleep(150);return;
    }
    throw Error(`Cannot reveal ${label}`);
  };
  const xy=async(x,y)=>{const s=await state(),r=s.controls['Sketch canvas'],v=s.sketch;return [(r[0]+r[2])/2+(x-v.center[0])*v.scale,(r[1]+r[3])/2-(y-v.center[1])*v.scale];};
  const point=async(x,y)=>{await click(...await xy(x,y));await sleep(100);};
  const kind=async name=>{await button('Constraint kind');await button(`Constraint kind ${name}`);};
  const value=async text=>{await button('Constraint value');await pressKey('a','KeyA',2);await send('Input.insertText',{text});await sleep(200);assert((await state()).workspace.sketch.constraint_form.value===text,`Dimension input did not receive ${text}`);await pressKey('Tab','Tab');await sleep(100);};
  const add=async()=>{await button('Add constraint');return settled();};
  const update=async()=>{await button('Update constraint');return settled();};
  const end=s=>model(s).entities[0].geometry.end;
  const near=(p,x,y)=>Math.hypot(p.x-x,p.y-y)<1e-3;
  const drag=async(a,b)=>{
    const start=await xy(...a),end=await xy(...b);
    await send('Input.dispatchMouseEvent',{type:'mouseMoved',x:start[0],y:start[1]});
    await send('Input.dispatchMouseEvent',{type:'mousePressed',x:start[0],y:start[1],button:'left',clickCount:1});
    for(let i=1;i<=6;i++) {await send('Input.dispatchMouseEvent',{type:'mouseMoved',x:start[0]+(end[0]-start[0])*i/6,y:start[1]+(end[1]-start[1])*i/6,button:'left',buttons:1});await sleep(70);}
    await send('Input.dispatchMouseEvent',{type:'mouseReleased',x:end[0],y:end[1],button:'left',clickCount:1});return settled();
  };
  await control('File');await control('New job');await waitFor(s=>s.job&&!s.active,'empty job');
  const original=await state();await button('New sketch');await waitFor(s=>s.sketch,'new sketch');
  await button('Sketch Line');await point(20,30);await point(60,30);await settled();
  await button('Sketch Select');
  if(!(await state()).sketch.show_constraints)await button('Constraints & dimensions');
  await wheel((await state()).controls['Sketch canvas'],450);await sleep(500);
  await point(20,30);await kind('Fix point');
  let s=await add();assert(s.sketch.solver.dof===2,'fixed point must leave two DOF');
  await point(40,30);await kind('Horizontal');s=await add();assert(s.sketch.solver.dof===1,'horizontal line should have one DOF');
  await point(40,30);await kind('Length');await value('40');s=await add();assert(s.sketch.solver.dof===0,'dimensioned line should be fully constrained');
  assert(isDeepStrictEqual(s.job,original.job),'constraint edits mutated the job');
  await value('50');s=await update();assert(near(end(s),70,30),'driving length did not resize');
  await button('Sketch undo');s=await settled();assert(near(end(s),60,30),'dimension undo failed');
  await button('Sketch redo');s=await settled();assert(near(end(s),70,30),'dimension redo failed');
  await point(45,30);await button('Reference dimension');s=await add();assert(s.sketch.solver.dof===0,'reference changed DOF');
  await button('Constraint row 3');await value('60');s=await update();assert(near(end(s),80,30),'second dimension edit failed');
  await point(50,30);
  // Selecting a body starts a new form; set a conflicting driving length.
  s=await state();if(s.workspace.sketch.constraint_form.reference)await button('Reference dimension');
  await value('80');s=await add();assert(!s.sketch.solver.accepted,'conflicting length was accepted');
  assert(near(end(s),80,30),'failed solve overwrote geometry');assert(s.sketch.solver.issues.length>0,'missing conflict diagnostics');
  await screenshot('sketch-constraint-conflict.png');
  await button('Remove constraint');s=await settled();assert(s.sketch.solver.accepted,'removing conflict did not recover');
  const count=s.sketch.undo;
  s=await drag([80,30],[90,40]);assert(near(end(s),80,30),'fully constrained endpoint moved');
  assert(s.sketch.solver_status.includes('limited'),'infeasible drag did not explain its limit');assert(s.sketch.undo===count,'blocked drag polluted undo');
  await button('Constraint row 3');await button('Reference dimension');s=await update();assert(s.sketch.solver.dof===1,'driving/reference conversion failed');
  s=await drag([80,30],[90,40]);assert(near(end(s),90,30),'constrained drag failed to project onto horizontal line');
  await button('Sketch undo');s=await settled();assert(near(end(s),80,30),'drag undo failed');
  await button('Sketch redo');s=await settled();assert(near(end(s),90,30),'drag redo failed');
  record('driving/reference, fixed points, constrained drag, conflict recovery and local undo',s.sketch);
  await screenshot('sketch-constraints.png');
  // Unsaved form text is recovered too; it never enters the solver as a number.
  await button('Constraint row 3');await button('Reference dimension');await value('unfinished');
  const recovery=model(await state());await sleep(2200);await send('Page.reload',{ignoreCache:true});
  await waitFor(s=>s.controls?.['Restore draft'],'recovery offer');await control('Restore draft');s=await settled();
  assert(isDeepStrictEqual(model(s),recovery),'recovery changed solved geometry');
  assert(s.workspace.sketch.constraint_form.value==='unfinished','recovery lost numeric draft');
  if(!(await state()).sketch.show_constraints)await button('Constraints & dimensions');await button('Clear operands');
  await button('Apply sketch');s=await waitFor(s=>!s.sketch&&!s.active&&s.job.artworks.length===1,'applied constrained sketch');
  assert(s.undo===original.undo+1,'Apply was not one job transaction');
  await evaluate('globalThis.showSaveFilePicker=undefined');await control('Save job');
  await waitFor(s=>s.status.includes('Download requested'),'saved sketch');
  const file=path.join(out,'carving.gui2.job.json');for(let i=0;i<40&&!existsSync(file);i++)await sleep(250);
  const saved=JSON.parse(readFileSync(file,'utf8'));assert(saved.artwork[0].content.constraints.length===4,'saved constraints missing');
  await control('File');await chooseFile('Open job',file);await waitFor(s=>!s.active&&s.job?.artworks?.length===1,'reopened sketch');
  await control('Artwork');await button('Edit sketch');s=await settled();assert(model(s).constraints.length===4,'reopen lost constraints');
  if(!(await state()).sketch.show_constraints)await button('Constraints & dimensions');
  await send('Emulation.setDeviceMetricsOverride',{width:640,height:480,deviceScaleFactor:1,mobile:false});await sleep(300);
  await screenshot('sketch-constraints-compact.png');
  s=await state();for(const name of ['Apply sketch','Cancel sketch','Sketch canvas']){const r=s.controls[name];assert(r&&r[2]<=640&&r[3]<=480,`compact layout clipped ${name}`);}
  await button('Cancel sketch');record('saved and recovered constraint model on browser',await state());
  // The loadable plate exercises closed-wire joins, a hole and full constraint status.
  await send('Emulation.setDeviceMetricsOverride',{width:1280,height:800,deviceScaleFactor:1,mobile:false});await sleep(200);
  await control('File');await chooseFile('Open job',path.resolve('fixtures/v5/sketch-job.json'));
  await waitFor(s=>!s.active&&s.job?.artworks?.[0]?.id==='sketch','plate fixture');
  await control('Artwork');await button('Edit sketch');s=await settled();assert(s.sketch.solver.dof===0,'plate not fully constrained');
  if(!(await state()).sketch.show_constraints)await button('Constraints & dimensions');await button('Constraint row 9');await value('25');s=await update();
  assert(s.sketch.solver.accepted&&s.sketch.solver.dof===0,'plate dimension resize failed');
  const lines=model(s).entities.filter(e=>e.geometry.kind==='line').slice(0,4);
  for(let i=0;i<4;i++)assert(near(lines[i].geometry.end,lines[(i+1)%4].geometry.start.x,lines[(i+1)%4].geometry.start.y),'plate wire separated');
  await button('Constraint row 13');await button('Remove constraint');await settled();
  await point(12,7);await kind('Diameter');await value('6');s=await add();
  assert(Math.abs(model(s).entities.find(e=>e.geometry.kind==='circle').geometry.radius_mm-3)<1e-6,'diameter did not drive the hole radius');
  await point(10,7);await kind('Fix point');s=await add();assert(s.sketch.solver.dof===0,'plate with dimensioned hole not fully constrained');
  await screenshot('sketch-constrained-plate.png');
  await button('Apply sketch');await waitFor(s=>!s.sketch&&!s.active,'plate applied');
  record('fully constrained plate resize, connected wire and diameter-driven hole',await state());
}
