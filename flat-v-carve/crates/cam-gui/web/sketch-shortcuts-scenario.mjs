// Keyboard/pointer input against the shipped WebGPU app and production solver Worker.
export async function sketchShortcutsScenario({control,state,waitFor,send,sleep,record,screenshot,pressKey,click}) {
  const assert=(v,m)=>{if(!v)throw Error(m);};
  const model=s=>s.workspace.sketch.model;
  const form=s=>s.workspace.sketch.constraint_form;
  const settled=()=>waitFor(s=>s.sketch&&!s.sketch.solving&&!s.sketch.pending,'sketch solve');
  const key=async k=>{await pressKey(k,/^[0-9]$/.test(k)?`Digit${k}`:`Key${k.toUpperCase()}`);await sleep(120);};
  const sequence=async keys=>{for(const k of keys)await key(k);};
  const tap=async(x,y,shift=false)=>{
    if(!shift)await click(x,y);
    else {await send('Input.dispatchMouseEvent',{type:'mousePressed',x,y,button:'left',clickCount:1,modifiers:8});await sleep(60);await send('Input.dispatchMouseEvent',{type:'mouseReleased',x,y,button:'left',clickCount:1,modifiers:8});}
    await sleep(160);
  };
  const button=async(label,shift=false)=>{
    for(let i=0;i<28;i++) {
      const s=await state(),r=s.controls[label];assert(r,`Missing ${label}`);
      const name=label.startsWith('Geometry row ')?'Geometry panel':label.startsWith('Sketch ')?'Sketch tools panel':'Constraint panel';
      const p=s.controls[name];
      if(p&&r[0]>=p[0]&&r[2]<=p[2]&&(r[1]<p[1]||r[3]>p[3])) {
        const x=(r[0]+r[2])/2,y=(p[1]+p[3])/2,deltaY=r[1]<p[1]?Math.max(-90,r[1]-p[1]-8):Math.min(90,r[3]-p[3]+8);
        await send('Input.dispatchMouseEvent',{type:'mouseMoved',x,y});await send('Input.dispatchMouseEvent',{type:'mouseWheel',x,y,deltaX:0,deltaY});await sleep(220);continue;
      }
      await tap((r[0]+r[2])/2,(r[1]+r[3])/2,shift);return;
    }throw Error(`Cannot reveal ${label}`);
  };
  const point=async(x,y)=>{const s=await state(),r=s.controls['Sketch canvas'],v=s.sketch;await tap((r[0]+r[2])/2+(x-v.center[0])*v.scale,(r[1]+r[3])/2-(y-v.center[1])*v.scale);};
  await control('File');await control('New job');await waitFor(s=>s.job&&!s.active,'new job');await button('New sketch');await settled();
  await sequence('gl');await point(10,10);await point(40,12);await button('Sketch Select');
  if(!(await state()).sketch.show_geometry)await button('Geometry list');
  const line=model(await state()).entities[0].id;
  await button(`Geometry row ${line}`);await key('a');let s=await settled();
  assert(model(s).constraints[0].definition.kind==='horizontal'&&s.sketch.solver.accepted,'A must solve automatic horizontal constraint');
  await sequence('zr');s=await settled();assert(model(s).constraints.length===0,'Z,R removes alignment');
  await key('h');s=await settled();assert(model(s).constraints.length===1,'H adds alignment');
  await sequence('gc');await point(20,30);await point(24,30);
  await sequence('gc');await point(40,30);await point(46,30);await button('Sketch Select');
  const [c1,c2]=model(await state()).entities.slice(-2).map(e=>e.id);
  await button(`Geometry row ${c1}`);await button(`Geometry row ${c2}`,true);await key('e');s=await settled();
  assert(model(s).constraints.at(-1).definition.kind==='equal_radius'&&s.sketch.solver.accepted,'E must choose equal radii');
  await button(`Geometry row ${c1}`);await key('r');s=await state();assert(form(s).choice==='Diameter','R must choose diameter for circle');
  await send('Input.insertText',{text:'10'});await pressKey('Enter','Enter');s=await settled();
  assert(model(s).constraints.at(-1).definition.value_mm===10,'R value should focus and save');
  await sequence('kx');s=await settled();assert(model(s).constraints.at(-1).reference,'K,X converts existing dimension immediately');
  await sequence('ze');s=await state();assert(s.sketch.selected.includes(c1),'Z,E selects dimension geometry');
  await sequence('gy');await point(10,20);await button('Sketch Select');
  const p=model(await state()).entities.at(-1).id;
  await button(`Geometry row ${p}`);await sequence('kl');s=await settled();
  assert(model(s).constraints.slice(-2).map(c=>c.definition.kind).join(',')==='horizontal_distance,vertical_distance','K,L creates both position dimensions');
  const count=model(s).constraints.length;
  await button('Sketch undo');s=await settled();assert(model(s).constraints.length===count-2,'Lock position undo must be atomic');
  await button(`Geometry row ${p}`);await sequence('zo');await key('c');await sleep(850);s=await settled();
  assert(model(s).constraints.at(-1).definition.kind==='coincident','C uses origin selection');
  assert(Math.hypot(model(s).entities.at(-1).geometry.position.x,model(s).entities.at(-1).geometry.position.y)<1e-6,'Origin coincidence must solve');
  await sequence('zh');await sequence('zv');s=await state();assert(s.sketch.selected.includes('@x_axis')&&s.sketch.selected.includes('@y_axis'),'Axis shortcuts must add to selection');
  await pressKey('a','KeyA',2);s=await state();assert(s.sketch.selected.length===4&&!s.sketch.selected.includes('@origin'),'Ctrl+A selects authored geometry');
  const before=JSON.stringify(model(s));await sequence('gea');s=await state();
  assert(s.sketch.message.includes('Elliptical Arc')&&JSON.stringify(model(s))===before,'Unsupported G,E,A must not apply E or A');
  await sequence('g3a');assert((await state()).sketch.tool==='Arc','G,3,A chooses three-point arc');await pressKey('Escape','Escape');
  await button('Sketch shortcuts');await screenshot('sketch-shortcuts-reference.png');await pressKey('Escape','Escape');
  await screenshot('sketch-shortcuts.png');record('FreeCAD sequences, A/H, E radii, R dimension focus, KX, ZE, KL atomic undo, origin/axis selection, delayed C, Ctrl+A, missing commands and help', (await state()).sketch);
  await button('Cancel sketch');
}
