// V5 real-widget acceptance: display menus, compact transport, sections and
// pinned stale/current identity. No test-only mutation of application state.
export async function inspectionReviewScenario({control,edit,state,waitFor,send,evaluate,sleep,record,screenshot,readFileSync,pressKey}) {
  await send('Emulation.setDeviceMetricsOverride',{width:1280,height:800,deviceScaleFactor:1,mobile:false});
  const job=readFileSync('fixtures/gui4/lettering.job.json','utf8');
  await evaluate(`(()=>{const t=new DataTransfer();t.items.add(new File([${JSON.stringify(job)}],'lettering.job.json',{type:'application/json'}));document.getElementById('cam').dispatchEvent(new DragEvent('drop',{dataTransfer:t,bubbles:true,cancelable:true}));})()`);
  await waitFor(s=>s.job?.name&&!s.active,'inspection fixture');
  await control('Generate');await waitFor(s=>s.current&&!s.active,'inspection generation');
  await control('Simulate');await control('After endmill');await waitFor(s=>!s.active&&s.stockPrefix===s.motions,'final stock');
  const original=await state();
  for(const label of ['Stock color By tool','Stock walls Depth gradient','X-ray stock','Layer Travel']) {
    await control(label);await pressKey('Escape','Escape');
  }
  const styled=await state();
  if(styled.revision!==original.revision||styled.stockPrefix!==original.stockPrefix||!styled.current||styled.display.key!==original.display.key)throw new Error('Display menus changed the plan or stock position');
  if(styled.workspace.view.stock_style.surface!=='by_tool'||styled.workspace.view.stock_style.walls!=='by_depth'||styled.display.style.appearance!=='xray'||styled.display.style.showTravel===original.display.style.showTravel)throw new Error('A display menu choice was not applied');
  if(JSON.stringify(styled.workspace.view.inspection_xy)!==JSON.stringify(original.workspace.view.inspection_xy))throw new Error('A display menu click leaked into viewport picking');
  record('display menus preserve document and retained simulation',styled);
  await control('Layers menu');await control('View key');await sleep(200);
  await screenshot('v5-view-key.png');await pressKey('Escape','Escape');
  await control('Solid stock');await pressKey('Escape','Escape');
  await control('Display resolution');await control('Display resolution Fine');
  const fine=await waitFor(s=>!s.active&&s.display.preset==='fine','display resolution from transport options');
  if(fine.revision!==original.revision||fine.stockPrefix!==original.stockPrefix)throw new Error('Display quality changed the document or playhead');
  await pressKey('Escape','Escape');
  record('display resolution is reachable without changing the plan',fine);
  await control('Pin comparison');await waitFor(s=>s.inspection?.pinned,'pin comparison');
  await control('Inspect Section');await control('Section Y');
  const section=await state();
  if(!section.controls['Section plot']||!section.workspace.view.section_y)throw new Error('Y section did not render');
  await screenshot('v5-section-y.png');
  await control('Section X');
  for(const [width,height,scale] of [[1280,800,1],[1440,900,1],[1920,1080,1],[960,600,1],[640,400,1]]) {
    await send('Emulation.setDeviceMetricsOverride',{width,height,deviceScaleFactor:scale,mobile:false});
    await sleep(400);
    const s=await state(),bar=s.controls['View toolbar'],transport=s.controls['Simulation transport'];
    const logicalWidth=width/scale,logicalHeight=height/scale;
    if(!bar||bar[0]<0||bar[2]>logicalWidth+1||bar[3]-bar[1]>36)throw new Error(`Toolbar exceeds viewport at ${width}`);
    if(!transport||transport[2]>logicalWidth+1||transport[3]-transport[1]>(transport[2]-transport[0]<400?135:100))throw new Error(`Transport exceeds budget at ${width}`);
    await screenshot(`v5-section-${width}x${height}-${scale}.png`);
    if(width===640){await control('Section plot');await screenshot('v5-section-compact-scrolled.png');}
    await control('Playback speed');await sleep(200);
    const rates=await state();
    for(const name of ['Playback 0.5x','Playback 1000x','Playback Fit']) {
      const r=rates.controls[name];if(!r||r[1]<0||r[3]>logicalHeight)throw new Error(`${name} clipped at ${width}`);
    }
    await screenshot(`v5-speeds-${width}x${height}-${scale}.png`);
    await control('Playback 1x');
    record(`bounded inspection ${width}x${height}@${scale}`,s);
  }
  await send('Emulation.setDeviceMetricsOverride',{width:1280,height:800,deviceScaleFactor:1,mobile:false});await sleep(400);
  await control('Cutting');await edit('Roughing feed','1300');
  const stale=await waitFor(s=>!s.current&&!s.active,'stale result after feed edit');
  if(!stale.inspection.pinned)throw new Error('Editing lost the pinned comparison');
  await control('Inspect result');await screenshot('v5-stale-comparison.png');
  await control('Generate');await waitFor(s=>s.current&&!s.active,'regenerated comparison');
  await control('Inspect result');await control('Match pinned position');
  const compared=await waitFor(s=>!s.active&&s.inspection.comparisonPrefix===s.stockPrefix,'matched stage comparison');
  await screenshot('v5-current-comparison.png');
  record('pinned identity survives stale edit and regeneration',compared);
  const many=JSON.parse(job);
  many.operations=Array.from({length:12},(_,index)=>({...structuredClone(many.operations[0]),id:`review-${index+1}`,name:`Repeated carve ${index+1}`}));
  await evaluate(`(()=>{const t=new DataTransfer();t.items.add(new File([${JSON.stringify(JSON.stringify(many))}],'twelve-stages.job.json',{type:'application/json'}));document.getElementById('cam').dispatchEvent(new DragEvent('drop',{dataTransfer:t,bubbles:true,cancelable:true}));})()`);
  await waitFor(s=>s.job?.operations?.length===12&&!s.active,'twelve-stage fixture');
  await control('Generate');await waitFor(s=>s.current&&!s.active,'twelve-stage generation',180);
  await control('Simulate');await control('Stage jumps');
  await waitFor(s=>s.timelineRows===8,'bounded first stage page');
  await control('Later stages');
  const later=await waitFor(s=>s.timelineRows===8&&Object.keys(s.controls).some(k=>k.startsWith('After ')&&k.includes('(12 of 12)')),'later stage page stays open');
  await screenshot('v5-stage-menu.png');
  const finalStage=Object.keys(later.controls).find(k=>k.startsWith('After ')&&k.includes('(12 of 12)'));
  await control(finalStage);await waitFor(s=>!s.active&&s.stockPrefix===s.motions,'last stage seek');
  await control('Program time');
  for(const type of ['keyDown','keyUp'])await send('Input.dispatchKeyEvent',{type,key:'Home',code:'Home',windowsVirtualKeyCode:36,nativeVirtualKeyCode:36});
  await waitFor(s=>!s.active&&s.stockPrefix===0,'timeline keyboard Home');
  for(const type of ['keyDown','keyUp'])await send('Input.dispatchKeyEvent',{type,key:'End',code:'End',windowsVirtualKeyCode:35,nativeVirtualKeyCode:35});
  await waitFor(s=>!s.active&&s.stockPrefix===s.motions,'timeline keyboard End');
  record('stage paging and keyboard seeking use the retained execution',await state());
}
