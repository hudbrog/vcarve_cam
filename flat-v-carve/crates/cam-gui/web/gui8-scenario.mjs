import {existsSync} from 'node:fs';

// GUI8 profiles in a real browser: create a profile job from an SVG, select
// closed contours with explicit sides, cut it, add tabs and radial finishing,
// anchor the start and a ramp entry, provoke a located entry error and repair
// it, then prepare and reopen the checked output.
export async function gui8Scenario({control,edit,state,waitFor,send,evaluate,sleep,record,screenshot,readFileSync,pressKey,path,out,chooseFile}) {
  // Simulate navigates to the inspection tab, which has no field filter: return
  // to the operation editor first.
  const clearSearch=async()=>{
    if(!(await state()).controls['Filter fields'])await control('Cutting');
    await control('Filter fields');await pressKey('a','KeyA',2);await pressKey('Backspace','Backspace');await sleep(150);
  };
  // The profile inspector is one long scroll: the optional groups (tabs,
  // finishing, start and entry) sit below the fold, so scroll the inspector to
  // them before driving their controls.
  const scrollInspector=async(notches,direction=1)=>{
    const clip=(await state()).controls['Inspector viewport'];
    const x=(clip[0]+clip[2])/2,y=(clip[1]+clip[3])/2;
    for(let i=0;i<notches;i++){
      await send('Input.dispatchMouseEvent',{type:'mouseMoved',x,y});
      await sleep(40);
      await send('Input.dispatchMouseEvent',{type:'mouseWheel',x,y,deltaX:0,deltaY:direction*1200});
      await sleep(180);
    }
    record('inspector scroll',(await state()).workspace.operation_scroll[0]);
  };

  // The profile editor is one long inspector; give the page enough height that
  // every group is on screen, so the tour drives the shipped layout instead of
  // fighting its scroll area.
  await send('Emulation.setDeviceMetricsOverride',{width:1280,height:4200,deviceScaleFactor:1,mobile:false});
  await sleep(700);

  // --- GUI8a: a closed profile from an SVG. ---------------------------------
  await control('File');await chooseFile('New profile job from SVG','fixtures/gui3/lettering.svg');
  await waitFor(s=>s.job?.kind==='profile'&&!s.active,'new profile job');
  const fresh=await state();
  if(fresh.job.contours!==0||fresh.job.stepdown!=null)
    throw new Error('New profile job invented machining values: '+JSON.stringify({kind:fresh.job.kind,contours:fresh.job.contours,stepdown:fresh.job.stepdown}));
  if(fresh.job.artworks.length!==1)
    throw new Error('Profile job should carry the imported artwork: '+JSON.stringify(fresh.job.artworks?.length));
  await screenshot('gui8-profile-new.png');

  await control('Setup');
  await edit('Stock thickness','6');
  await clearSearch();
  await control('Cutting');
  await edit('Stepdown','1');
  await edit('Tool stepdown limit','1');
  await edit('Roughing feed','300');await edit('Plunge feed','100');
  await edit('Spindle speed','12000');
  await edit('Endmill diameter','3');await edit('Cutting length','8');
  await clearSearch();
  // Every closed boundary with the importer's advisory side, then climb milling.
  await control('Cutting');
  await control('Select all profile contours');
  await waitFor(s=>s.job.contours===3,'three closed contours selected');
  const sides=(await state()).job.sides;
  if(!sides.includes('outside')||!sides.includes('inside'))throw new Error('Contour sides were not preserved');
  await control('Cut Climb');
  await control('Profile CW');
  // The profile must state where the cut ends: cutting through the stock means
  // the bottom follows the physical stock bottom.
  await control('Bottom: stock bottom');
  await clearSearch();
  await screenshot('gui8-profile-selection.png');

  // The profile needs the applied machine configuration to be exportable.
  await control('Machine');await control('Example machine');await control('Apply flower machine profile');
  await waitFor(s=>s.job?.machine,'applied machine');
  await control('Cutting');
  await control('Generate');
  const generated=await waitFor(s=>s.current&&!s.active&&s.motions>0,'generated profile');
  record('profile motions',generated.motions);
  await screenshot('gui8-profile-generated.png');
  await control('Simulate');await control('After profile rough');
  const rough=await waitFor(s=>!s.active&&s.stockPrefix>0&&s.stockPrefix<=s.motions,'profile rough stock');
  record('profile rough stock',rough.stockPrefix);
  await screenshot('gui8-profile-stock.png');

  // --- GUI8b: tabs ----------------------------------------------------------
  await control('Cutting');
  await scrollInspector(8);
  await control('Operation Tabs');
  await control('Add tabs');
  await waitFor(s=>s.job.tabs!=null,'tabs enabled');
  await edit('Tab height','0.5');
  await edit('Tab width','3');
  await edit('Tab count','1');
  await clearSearch();
  await control('Generate');
  // The letter O's hole is 7 x 8 mm: with a 3 mm cutter there is no room for a
  // 3 mm tab, and the planner refuses it with a located reason instead of
  // cutting an unprotected part.
  const refused=await waitFor(s=>!s.active&&s.motions===0&&s.issues.length>0,'tabs refused on the small hole',120);
  if(!refused.issues.some(issue=>issue.code==='PROFILE_TAB_NO_SPACE'))
    throw new Error('Expected a located tab rejection: '+JSON.stringify(refused.issues));
  record('tab rejection located',refused.issues[0].message);
  await screenshot('gui8-tabs-refused.png');
  // Removing the hole from the selection repairs it: the outer contours have
  // room for the tab.
  await control('Profile contour artwork-1 / letter-o-0-hole-0');
  await waitFor(s=>s.job.contours===2&&!s.active,'hole removed from the tab selection');
  await control('Generate');
  const tabbed=await waitFor(s=>s.current&&!s.active&&s.motions>0&&s.exportReady,'regenerated with tabs',120);
  record('tab motions',tabbed.motions);
  await screenshot('gui8-tabs-generated.png');

  // --- GUI8c: radial finishing ---------------------------------------------
  await control('Cutting');
  await scrollInspector(8);
  await control('Operation Radial finishing');
  await control('Add radial finishing');
  await waitFor(s=>s.job.finish?.enabled===true,'finishing enabled');
  await edit('Finish allowance','0.4');
  await edit('Finish feed','150');
  await clearSearch();
  await control('Generate');
  const finished=await waitFor(s=>s.current&&!s.active,'regenerated with finishing');
  record('finished motions',finished.motions);
  // Rough and finishing passes each publish a stage of their own, so the
  // timeline offers a checkpoint per pass.
  await control('Simulate');
  const jumps=(await state()).controls;
  if(!Object.keys(jumps).some(key=>key.startsWith('After profile finish')))
    throw new Error('The finishing pass has no stock checkpoint');
  await screenshot('gui8-finishing-stock.png');

  // --- GUI8d: start and entry ----------------------------------------------
  await control('Cutting');
  await scrollInspector(10);
  await control('Anchor the start');await control('lettering.svg / letter-o-0-outer · 50%');
  const anchored=await waitFor(s=>s.job.start?.kind==='anchor','anchored start');
  record('anchored start',anchored.job.start?.fraction_along_source_contour);
  await control('Ramp entry');
  await waitFor(s=>s.job.entry?.kind==='ramp','ramp entry selected');
  await edit('Entry ramp angle','45');await edit('Entry ramp feed','80');
  // The ramp needs a ramp-capable tool: an unset capability is a located
  // requirement, not a silent assumption.
  await clearSearch();
  await control('Generate');
  const blocked=await waitFor(s=>!s.active&&!s.current,'ramp needs a capability',120);
  if(!String(blocked.status).includes('ramp')&&!String(JSON.stringify(blocked.issues)).includes('ramp'))
    throw new Error('The unset ramp capability was not reported');
  await screenshot('gui8-ramp-needs-capability.png');
  await record('ramp requirement reported',blocked.status);
  await control('Cutting');
  await control('Ramp yes');
  await clearSearch();
  await control('Generate');
  const ramped=await waitFor(s=>s.current&&!s.active&&s.motions>0&&s.exportReady,'ramped profile',120);
  record('ramped motions',ramped.motions);
  await screenshot('gui8-ramp-entry.png');

  // --- checked output and reopen -------------------------------------------
  await control('Prepare checked output');
  const prepared=await waitFor(s=>s.prepared&&!s.active,'checked profile output',120);
  record('checked output',prepared.preparedSha256);
  await screenshot('gui8-export-ready.png');
  // Headless Chrome aborts the native save picker; the adapter's ordinary
  // download path is the supported browser baseline.
  await evaluate('globalThis.showSaveFilePicker=undefined');
  await control('Save as…');
  await waitFor(s=>s.status.includes('Download requested'),'program download');
  for(let i=0;i<100&&!existsSync(path.join(out,'sequence.ngc'));i++)await sleep(100);
  const gcode=readFileSync(path.join(out,'sequence.ngc'),'utf8');
  if(!gcode.includes('F150')||!gcode.includes('F80'))
    throw new Error('The program does not carry the finishing and ramp feeds');
  await control('Save job');
  await waitFor(s=>s.status.includes('Download requested'),'job download');
  for(let i=0;i<100&&!existsSync(path.join(out,'carving.gui2.job.json'));i++)await sleep(100);
  const saved=readFileSync(path.join(out,'carving.gui2.job.json'),'utf8');
  await evaluate(`(()=>{const t=new DataTransfer();t.items.add(new File([${JSON.stringify(saved)}],'profile-reopen.job.json',{type:'application/json'}));document.getElementById('cam').dispatchEvent(new DragEvent('drop',{dataTransfer:t,bubbles:true,cancelable:true}));})()`);
  const reopened=await waitFor(s=>s.job?.kind==='profile'&&!s.active&&!s.pending,'profile reopen');
  if(reopened.job.contours!==2||reopened.job.tabs==null||reopened.job.finish?.enabled!==true)
    throw new Error('The reopened profile lost its contours, tabs or finishing: '+JSON.stringify({contours:reopened.job.contours,tabs:reopened.job.tabs,finish:reopened.job.finish}));
  if(reopened.job.start?.kind!=='anchor'||reopened.job.entry?.kind!=='ramp')
    throw new Error('The reopened profile lost its start anchor or ramp entry');
  await screenshot('gui8-profile-reopened.png');
  record('profile reopened with tabs, finishing and entry',reopened.job.contours);

  await control('Cutting');await edit('Stepdown','-');
  await waitFor(s=>s.pending&&!s.current,'pending text invalidates the profile result');
  await screenshot('gui8-profile-pending.png');
}
