// Real keyboard/pointer input, production WASM solver; state probe is read-only.
import {readFileSync,existsSync} from 'node:fs';
import {isDeepStrictEqual} from 'node:util';
export async function sketchHotkeysScenario({control,state,waitFor,send,evaluate,sleep,record,screenshot,pressKey,click,path,out,chooseFile}) {
  const assert=(v,m)=>{if(!v)throw Error(m);};
  const model=s=>s.workspace.sketch.model;
  const form=s=>s.workspace.sketch.constraint_form;
  const settled=()=>waitFor(s=>s.sketch&&!s.sketch.solving&&!s.sketch.pending,'sketch solve');
  const button=async(label)=>{
    for(let i=0;i<24;i++) {
      const s=await state(),r=s.controls[label];assert(r,`Missing ${label}`);
      const names=label.startsWith('Geometry row ')?['Geometry panel']:/^(Constraint |Clear operands|Add constraint|Update constraint|Reference dimension)/.test(label)?['Constraint panel']:[];
      const pane=names.map(n=>s.controls[n]).find(p=>p&&r[0]>=p[0]&&r[2]<=p[2]&&(r[1]<p[1]||r[3]>p[3]));
      if(pane){const x=(r[0]+r[2])/2,y=(pane[1]+pane[3])/2,deltaY=r[1]<pane[1]?Math.max(-100,r[1]-pane[1]-8):Math.min(100,r[3]-pane[3]+8);await send('Input.dispatchMouseEvent',{type:'mouseMoved',x,y});await send('Input.dispatchMouseEvent',{type:'mouseWheel',x,y,deltaX:0,deltaY});await sleep(220);continue;}
      await click((r[0]+r[2])/2,(r[1]+r[3])/2);await sleep(180);return;
    }throw Error(`Cannot reveal ${label}`);
  };
  const point=async(x,y)=>{const s=await state(),r=s.controls['Sketch canvas'],v=s.sketch;await click((r[0]+r[2])/2+(x-v.center[0])*v.scale,(r[1]+r[3])/2-(y-v.center[1])*v.scale);await sleep(180);};
  const edge=async id=>{const g=model(await state()).entities.find(e=>e.id===id).geometry;await point((g.start.x+g.end.x)/2,(g.start.y+g.end.y)/2);};
  const type=async text=>{await send('Input.insertText',{text});await sleep(180);assert(form(await state()).value===text,`Focused value should be replaced by ${text}`);};
  const d=async()=>{await pressKey('d','KeyD');await sleep(180);};
  await control('File');await control('New job');await waitFor(s=>s.job&&!s.active,'new job');
  await button('New sketch');await settled();
  await button('Sketch Rectangle');await point(10,10);await point(50,40);
  await button('Sketch Circle');await point(30,25);await point(35,25);
  await button('Sketch Select');let s=await settled();
  const [width,height,,,circle]=model(s).entities.map(e=>e.id);
  await edge(width);
  await pressKey('d','KeyD',2);assert(form(await state()).choice!=='Length','Ctrl+D must not invoke bare D');
  await d();s=await state();assert(form(s).choice==='Length'&&form(s).value==='40','D should select current line length');
  await type('45');
  await edge(height);s=await settled();assert(model(s).constraints.at(-1).definition.value_mm===45,'Next canvas selection must save width');
  assert(s.sketch.selected.includes(height),'Next edge must remain selected');
  await d();await type('35');await pressKey('Enter','Enter');s=await settled();
  assert(model(s).constraints.at(-1).definition.value_mm===35,'Enter must commit height');
  const count=model(s).constraints.length;
  await d();assert(form(await state()).editing,'D should reopen existing height');
  await type('bad');await edge(width);s=await state();
  assert(form(s).value==='bad'&&form(s).operands[0].Entity===height,'Invalid input must retain selection and value');
  assert(s.sketch.message.includes('finite'),'Invalid value needs a visible diagnostic');
  await pressKey('Escape','Escape');assert(form(await state()).value==='35','Escape should restore stored height');
  await edge(width);await d();await type('46');
  if(!(await state()).sketch.show_geometry)await button('Geometry list');
  await button(`Geometry row ${height}`);s=await settled();
  assert(model(s).constraints.length===count&&model(s).constraints.find(c=>c.definition.line===width&&c.definition.kind==='length').definition.value_mm===46,'List selection should update original width without duplication');
  await button(`Geometry row ${circle}`);await d();assert(form(await state()).choice==='Diameter','D should dimension a circle diameter');
  await type('8');await pressKey('Enter','Enter');s=await settled();
  assert(s.sketch.solver.accepted,'Dimensions must solve in the production Worker');
  assert(Math.abs(model(s).entities.find(e=>e.id===circle).geometry.radius_mm-4)<1e-6,'Circle diameter did not solve');
  await screenshot('sketch-hotkeys.png');record('bare D focus, next canvas/list selection, Enter, Escape, invalid value, duplicate protection, circle diameter',s.sketch);
  const savedModel=structuredClone(model(s));
  await button('Apply sketch');await waitFor(s=>!s.sketch&&!s.active&&s.job.artworks.length===1,'applied sketch');
  await evaluate('globalThis.showSaveFilePicker=undefined');await control('Save job');await waitFor(s=>s.status.includes('Download requested'),'saved job');
  const file=path.join(out,'carving.gui2.job.json');for(let i=0;i<40&&!existsSync(file);i++)await sleep(250);
  assert(JSON.parse(readFileSync(file,'utf8')).artwork[0].content.constraints.length===7,'Saved dimensions missing');
  await control('File');await chooseFile('Open job',file);await waitFor(s=>!s.active&&s.job?.artworks?.length===1,'open saved job');
  await control('Artwork');await button('Edit sketch');s=await settled();
  assert(isDeepStrictEqual(model(s),savedModel),'Save/reopen changed dimensioned sketch');
  record('hotkey dimensions survive apply, save and reopen',s.sketch);
  await button('Cancel sketch');
}
