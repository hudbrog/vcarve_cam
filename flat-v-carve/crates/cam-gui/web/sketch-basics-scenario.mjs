// New geometry semantics exercised through the real UI and production WASM Worker.
import {readFileSync,existsSync} from 'node:fs';
import {isDeepStrictEqual} from 'node:util';
export async function sketchBasicsScenario({control,state,waitFor,send,evaluate,sleep,record,screenshot,pressKey,click,path,out,chooseFile}) {
  const assert=(v,m)=>{if(!v)throw Error(m);};
  const model=s=>s.workspace.sketch.model;
  const settled=()=>waitFor(s=>s.sketch&&!s.sketch.solving&&!s.sketch.pending,'sketch solve');
  const tap=async(x,y,shift=false)=>{
    if(!shift)await click(x,y);
    else {await send('Input.dispatchMouseEvent',{type:'mousePressed',x,y,button:'left',clickCount:1,modifiers:8});await sleep(70);await send('Input.dispatchMouseEvent',{type:'mouseReleased',x,y,button:'left',clickCount:1,modifiers:8});}
    await sleep(180);
  };
  const button=async(label,shift=false)=>{
    for(let i=0;i<24;i++) {
      const s=await state(),r=s.controls[label];assert(r,`Missing ${label}`);
      const names=label.startsWith('Constraint kind ')?['Constraint choices']:label.startsWith('Geometry row ')?['Geometry panel']:/^(Constraint |Clear operands|Add constraint|Update constraint|Reference dimension)/.test(label)?['Constraint panel']:[];
      const pane=names.map(n=>s.controls[n]).find(p=>p&&r[0]>=p[0]&&r[2]<=p[2]&&(r[1]<p[1]||r[3]>p[3]));
      if(pane){const x=(r[0]+r[2])/2,y=(pane[1]+pane[3])/2,deltaY=r[1]<pane[1]?Math.max(-100,r[1]-pane[1]-8):Math.min(100,r[3]-pane[3]+8);await send('Input.dispatchMouseEvent',{type:'mouseMoved',x,y});await send('Input.dispatchMouseEvent',{type:'mouseWheel',x,y,deltaX:0,deltaY});await sleep(220);continue;}
      await tap((r[0]+r[2])/2,(r[1]+r[3])/2,shift);return;
    }throw Error(`Cannot reveal ${label}`);
  };
  const point=async(x,y,shift=false)=>{const s=await state(),r=s.controls['Sketch canvas'],v=s.sketch;await tap((r[0]+r[2])/2+(x-v.center[0])*v.scale,(r[1]+r[3])/2-(y-v.center[1])*v.scale,shift);};
  const kind=async name=>{await button('Constraint kind');await button(`Constraint kind ${name}`);};
  const value=async text=>{await button('Constraint value');await pressKey('a','KeyA',2);await send('Input.insertText',{text});await sleep(180);assert((await state()).workspace.sketch.constraint_form.value===text,'Value input missed');await pressKey('Tab','Tab');};
  const add=async()=>{await button('Add constraint');const s=await settled();assert(s.sketch.solver.accepted,s.sketch.solver_status);return s;};
  await control('File');await control('New job');await waitFor(s=>s.job&&!s.active,'new job');
  await button('New sketch');await settled();
  await button('Sketch Rectangle');await point(10,10);await point(50,40);
  let s=await settled();assert(model(s).constraints.length===4&&s.sketch.solver.dof===4,'Rectangle needs four shape constraints and four DOF');
  await button('Sketch undo');s=await settled();assert(model(s).entities.length===0&&model(s).constraints.length===0,'Rectangle undo must remove constraints');
  await button('Sketch redo');await settled();await button('Sketch Select');
  if(!(await state()).sketch.show_constraints)await button('Constraints & dimensions');
  if(!(await state()).sketch.show_geometry)await button('Geometry list');
  await point(10,10);await button('Geometry row @origin',true);await kind('Coincident');s=await add();
  assert(s.sketch.solver.dof===2,'Origin coincidence must leave width and height free');
  assert(Math.hypot(model(s).entities[0].geometry.start.x,model(s).entities[0].geometry.start.y)<1e-6,'Rectangle corner did not meet origin');
  // Command-first through the dropdown: previously asked for two points.
  await button('Clear operands');await kind('Distance');
  const widthId=model(s).entities[0].id;await button(`Geometry row ${widthId}`);await value('40');s=await add();
  assert(model(s).constraints.at(-1).definition.kind==='length'&&s.sketch.solver.dof===1,'Distance must resolve a selected line');
  const heightId=model(s).entities[1].id;await button(`Geometry row ${heightId}`);await kind('Vertical distance');await value('30');s=await add();
  assert(s.sketch.solver.dof===0,'Single-line Y dimension must finish constraining rectangle');
  await screenshot('sketch-basics-rectangle.png');
  // Point-to-axis offsets in both selection orders, without saved axis entities.
  await button('Sketch Point');await point(20,20);await button('Sketch Select');
  const pointId=model(await state()).entities.at(-1).id;
  await point(20,20);await button('Geometry row @x_axis',true);await kind('Distance');await value('8');s=await add();
  assert(model(s).constraints.at(-1).definition.kind==='vertical_distance','X axis should give Y offset');
  await button('Geometry row @y_axis');const p=model(s).entities.at(-1).geometry.position;await point(p.x,p.y,true);await kind('Distance');await value('12');s=await add();
  const result=model(s).entities.find(e=>e.id===pointId).geometry.position;
  assert(Math.hypot(result.x-12,result.y-8)<1e-6&&s.sketch.solver.dof===0,'Signed axis distances did not solve');
  assert(model(s).entities.length===5,'Fixed references leaked into authored entities');
  const savedModel=structuredClone(model(s));await screenshot('sketch-basics-axes.png');
  record('rectangle constraints, origin coincidence, single-line Distance and Y distance, both axis selection orders',s.sketch);
  await button('Apply sketch');await waitFor(s=>!s.sketch&&!s.active&&s.job.artworks.length===1,'applied sketch');
  await evaluate('globalThis.showSaveFilePicker=undefined');await control('Save job');await waitFor(s=>s.status.includes('Download requested'),'saved job');
  const file=path.join(out,'carving.gui2.job.json');for(let i=0;i<40&&!existsSync(file);i++)await sleep(250);
  const saved=JSON.parse(readFileSync(file,'utf8'));assert(saved.artwork[0].content.entities.length===5,'Saved implicit reference entities');
  await control('File');await chooseFile('Open job',file);await waitFor(s=>!s.active&&s.job?.artworks?.length===1,'open saved job');
  await control('Artwork');await button('Edit sketch');s=await settled();
  assert(isDeepStrictEqual(model(s),savedModel),'Save/reopen changed references or solved geometry');
  assert(s.sketch.solver.dof===0&&s.sketch.solver.accepted,'Reopened reference constraints failed');
  record('saved fixed-reference constraints reopen and solve with no CAM reference entities',s.sketch);
  await button('Cancel sketch');
}
