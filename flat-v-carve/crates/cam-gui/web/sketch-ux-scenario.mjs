import {isDeepStrictEqual} from 'node:util';
export async function sketchUxScenario({control,state,waitFor,send,evaluate,sleep,record,screenshot,pressKey,click,path,chooseFile}) {
  const assert=(v,m)=>{if(!v)throw Error(m);};
  const settled=()=>waitFor(s=>s.sketch&&!s.sketch.solving&&!s.sketch.pending,'settled sketch');
  const model=s=>s.workspace.sketch.model;
  const button=async label=>{
    for(let i=0;i<20;i++){
      const s=await state(),r=s.controls[label];assert(r,`Missing ${label}`);
      const pane=['Sketch tools panel','Geometry panel','Constraint panel'].map(n=>s.controls[n]).find(p=>p&&r[0]>=p[0]&&r[2]<=p[2]&&(r[1]<p[1]||r[3]>p[3]));
      if(pane){const x=(pane[0]+pane[2])/2,y=(pane[1]+pane[3])/2;await send('Input.dispatchMouseEvent',{type:'mouseMoved',x,y});await send('Input.dispatchMouseEvent',{type:'mouseWheel',x,y,deltaX:0,deltaY:r[1]<pane[1]?-180:180});await sleep(180);continue;}
      await click((r[0]+r[2])/2,(r[1]+r[3])/2);await sleep(180);return;
    }throw Error(`Cannot reveal ${label}`);
  };
  const double=async r=>{const x=(r[0]+r[2])/2,y=(r[1]+r[3])/2;await click(x,y);await sleep(80);await send('Input.dispatchMouseEvent',{type:'mousePressed',x,y,button:'left',clickCount:2});await sleep(60);await send('Input.dispatchMouseEvent',{type:'mouseReleased',x,y,button:'left',clickCount:2});await sleep(250);};
  const xy=async(x,y)=>{const s=await state(),r=s.controls['Sketch canvas'],v=s.sketch;return [(r[0]+r[2])/2+(x-v.center[0])*v.scale,(r[1]+r[3])/2-(y-v.center[1])*v.scale];};
  const point=async(x,y)=>{await click(...await xy(x,y));await sleep(200);};
  const key=async(k,code)=>{await pressKey(k,code);await sleep(100);};
  await control('File');await chooseFile('Open job',path.resolve('fixtures/v5/sketch-lifecycle-job.json'));
  await waitFor(s=>s.job?.artworks?.length===2&&!s.active,'mixed fixture');
  await double((await state()).controls['Artwork sketch']);let s=await settled();assert(s.sketch,'Double-click must reopen sketch');
  const baseline=structuredClone(model(s));
  if(s.sketch.show_constraints)await button('Constraints & dimensions');
  if((await state()).sketch.show_geometry)await button('Geometry list');
  await point(12,2);s=await state();assert(s.sketch.selected.length===1,'Closed lists must allow individual selection');
  const selected=structuredClone(s.sketch.selected);
  await button('Geometry list');await button('Constraints & dimensions');s=await state();assert(isDeepStrictEqual(s.sketch.selected,selected),'Panel toggle changed selection');
  await button('Geometry list');await button('Constraints & dimensions');
  const edge=await xy(12,2);await double([edge[0]-1,edge[1]-1,edge[0]+1,edge[1]+1]);
  s=await state();assert(s.sketch.selected.length===4,'Double-click must select complete wire');
  const from=await xy(-1,-1),to=await xy(24,14);
  await send('Input.dispatchMouseEvent',{type:'mouseMoved',x:from[0],y:from[1]});
  await send('Input.dispatchMouseEvent',{type:'mousePressed',x:from[0],y:from[1],button:'left',clickCount:1});
  for(let i=1;i<=5;i++){await send('Input.dispatchMouseEvent',{type:'mouseMoved',x:from[0]+(to[0]-from[0])*i/5,y:from[1]+(to[1]-from[1])*i/5,button:'left',buttons:1});await sleep(50);}
  await send('Input.dispatchMouseEvent',{type:'mouseReleased',x:to[0],y:to[1],button:'left',clickCount:1});await sleep(250);
  s=await state();assert(model(s).entities.slice(0,5).every(e=>s.sketch.selected.includes(e.id)),'Box selection missed contained plate/hole');
  await button('Geometry list');
  const circle=model(s).entities.find(e=>e.geometry.kind==='circle');await button(`Geometry row ${circle.id}`);
  s=await state();assert(isDeepStrictEqual(s.sketch.selected,[circle.id]),'Geometry list selection must match canvas');
  await button('Geometry list');
  // Canvas dimension labels remain editable with the list closed.
  const width=model(s).constraints[8].id;await double((await state()).controls[`Dimension label ${width}`]);
  s=await state();assert(s.workspace.sketch.constraint_form.editing===width,'Dimension label did not open editor');
  await button('Constraint value');await pressKey('a','KeyA',2);await send('Input.insertText',{text:'25'});await sleep(250);assert((await state()).workspace.sketch.constraint_form.value==='25','Width input did not receive text');await key('Tab','Tab');await button('Update constraint');
  s=await settled();assert(s.sketch.solver.accepted,'Dimension edit not accepted');assert(Math.abs(model(s).entities[0].geometry.end.x-model(s).entities[0].geometry.start.x-25)<1e-6,'Width did not change');
  await button('Constraint filter Dimensions');await screenshot('sketch-ux-panels.png');
  await button('Constraints & dimensions');
  await key('g','KeyG');assert((await state()).sketch.shortcut.includes('G →'),'Missing sequence hint');await key('c','KeyC');assert((await state()).sketch.tool==='Circle','G,C did not select Circle');
  await key('Escape','Escape');assert((await state()).sketch.tool==='Select','Esc did not select');
  await point(14,2);await key('k','KeyK');await key('d','KeyD');s=await state();assert(s.workspace.sketch.constraint_form.choice==='Length','K,D must dimension a line');
  await button('Constraint value');await send('Input.insertText',{text:'gc'});await sleep(200);s=await state();assert(s.sketch.tool==='Select'&&s.workspace.sketch.constraint_form.value.includes('gc'),'Typing invoked shortcuts');
  await button('Clear operands');await button('Constraints & dimensions');
  // Terminate only the test solver Worker, then exercise the real GUI failure/retry route.
  await evaluate(`globalThis.__originalWorker=globalThis.Worker; globalThis.Worker=class extends globalThis.__originalWorker { constructor(url,options){super(url,options);if(String(url).includes('sketch-worker')){this.terminate();setTimeout(()=>this.dispatchEvent(new ErrorEvent('error',{message:'qualification worker failure'})),50);}} };`);
  await button('Cancel sketch');await double((await state()).controls['Artwork sketch']);
  s=await waitFor(s=>s.sketch&&!s.sketch.solving&&s.sketch.solver_status.includes('failure'),'worker failure diagnostic');
  assert(isDeepStrictEqual(model(s),baseline),'Worker failure changed saved geometry');
  await evaluate('globalThis.Worker=globalThis.__originalWorker');
  if(!(await state()).sketch.show_constraints)await button('Constraints & dimensions');
  await button('Solve sketch');s=await settled();assert(s.sketch.solver.accepted,'Worker retry failed');
  await screenshot('sketch-ux-retry.png');await button('Cancel sketch');
  record('unified selection, independent lists, double-click reopening/dimensions, keyboard focus, worker failure/retry',s.sketch);
}
