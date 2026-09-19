import {createHash} from 'node:crypto';
import {readdirSync} from 'node:fs';

export async function exceptionReviewScenario({control,state,waitFor,evaluate,sleep,record,screenshot,readFileSync,pressKey,send,chooseFile}) {
  await control('Tool library');await waitFor(s=>s.resources.ready&&!s.resources.busy,'empty local library');
  await screenshot('v6-empty-library.png');record('empty library exposes create and import actions',await state());
  await chooseFile('Import library','fixtures/gui5/library.json');
  await waitFor(s=>s.resources.catalog.id==='gui5-lettering-library'&&!s.resources.busy,'library import');
  await control('Library search');await send('Input.insertText',{text:'no matching cutter'});await sleep(300);
  if((await state()).controls['Library tool endmill']||(await state()).controls['Library tool vbit'])throw Error('No-match filter retained a tool row');
  await screenshot('v6-no-matching-library.png');record('no matching library items',await state());
  await control('Library search');await pressKey('a','KeyA',2);await pressKey('Backspace','Backspace');
  await control('Save library');await waitFor(s=>!s.resources.busy&&!s.resources.dirty,'save private review library before replacing it');
  const catalog=(await state()).resources.catalog;
  const previousStatus=(await state()).resources.status;
  await chooseFile('Import library','fixtures/gui4/broken.svg');await waitFor(s=>!s.resources.busy&&s.resources.status!==previousStatus,'invalid library import');
  if(JSON.stringify((await state()).resources.catalog)!==JSON.stringify(catalog))throw Error('Failed import changed the library draft');
  await screenshot('v6-library-import-error.png');record('failed library import retains draft',await state());
  await control('Close library');
  const job=JSON.parse(readFileSync('fixtures/gui4/lettering.job.json','utf8'));job.machine_configuration=null;
  await evaluate(`(()=>{const t=new DataTransfer();t.items.add(new File([${JSON.stringify(JSON.stringify(job))}],'unmapped.job.json',{type:'application/json'}));document.getElementById('cam').dispatchEvent(new DragEvent('drop',{dataTransfer:t,bubbles:true,cancelable:true}));})()`);
  await waitFor(s=>s.job?.name&&!s.active,'unmapped job');await control('Generate');await waitFor(s=>s.current&&!s.active,'unmapped generated plan');
  await control('Prepare checked output');await waitFor(s=>!s.active&&s.status.includes('MACHINE_CONFIGURATION_ABSENT'),'export validation failure');
  if((await state()).prepared)throw Error('Unmapped job acquired checked bytes');
  await screenshot('v6-export-validation-error.png');record('missing machine blocks output without losing the job',await state());
}

// V6 review drives real widgets; only the platform file-picker failure is injected.
export async function completionReviewScenario({control,edit,state,waitFor,send,evaluate,sleep,record,screenshot,readFileSync,pressKey,path,out,scale}) {
  await screenshot('v6-new-job.png');
  if(scale===1) {
    await control('Start blank job');await waitFor(s=>s.job?.operations?.length===0&&!s.active,'blank job');
    await screenshot('v6-empty-job.png');record('blank job is reachable without artwork',await state());
  }
  const base=JSON.parse(readFileSync('fixtures/gui4/lettering.job.json','utf8'));
  const second=structuredClone(base.operations[0]);second.id='later-operation';second.name='Later operation';base.operations.push(second);
  const drop=async(job)=>{
    const revision=(await state()).revision;
    await evaluate(`(()=>{const t=new DataTransfer();t.items.add(new File([${JSON.stringify(JSON.stringify(job))}],'review.job.json',{type:'application/json'}));document.getElementById('cam').dispatchEvent(new DragEvent('drop',{dataTransfer:t,bubbles:true,cancelable:true}));})()`);
    await waitFor(s=>s.revision>revision&&!s.active,'review job open');
  };
  await drop(base);
  if((await state()).workspace.navigator_collapsed)await control('Toggle navigator');
  await control('Operation row '+base.operations[0].id);
  await control('Generation scope');await control('Through selected operation');
  await waitFor(s=>s.current&&!s.active&&s.planScope?.kind==='throughOperation','retained prefix');
  await control('Prepare checked output');await waitFor(s=>s.prepared&&!s.active,'checked prefix');
  const prepared=await state(),sha=prepared.preparedSha256;
  const stages=Object.keys(prepared.controls).filter(name=>name.startsWith('Export stage '));
  if(stages.length!==1||stages.some(name=>name.includes(second.id)))throw Error('Export summary widened prefix: '+stages);
  record('prepared prefix summary excludes later enabled operation',prepared);
  const sizes=scale===1?[[1280,800],[1440,900],[1920,1080],[640,400]]:[[1280,800]];
  for(const [width,height] of sizes) {
    await send('Emulation.setDeviceMetricsOverride',{width,height,deviceScaleFactor:scale,mobile:false});await sleep(700);
    const s=await state(),r=s.controls['Export dialog'],save=s.controls['Save as…'];
    const dimensions=await evaluate('({width:innerWidth,height:innerHeight,dpr:devicePixelRatio,canvas:[document.getElementById("cam").width,document.getElementById("cam").height]})');
    if(!r||r[0]<0||r[1]<0||r[2]>width/scale+1||r[3]>height/scale+1||save[3]>height/scale)throw Error('Export modal clips: '+JSON.stringify({r,save,dimensions}));
    await screenshot(`v6-export-${width}x${height}-${scale}.png`);record(`export bounds ${width}x${height}@${scale}`,{dimensions,dialog:r,save});
  }
  await send('Emulation.setDeviceMetricsOverride',{width:1280,height:800,deviceScaleFactor:scale,mobile:false});await sleep(400);
  await evaluate('globalThis.showSaveFilePicker=async()=>{throw new Error("Review destination unavailable")}');
  await control('Save as…');await waitFor(s=>s.status.includes('Review destination unavailable'),'save failure');
  if((await state()).preparedSha256!==sha)throw Error('Failed save lost prepared bytes');
  await screenshot('v6-export-save-failure.png');record('failed save retains checked prefix',await state());
  await evaluate('globalThis.showSaveFilePicker=undefined');await control('Save as…');await waitFor(s=>s.status.includes('Download requested')&&!s.controls['Export dialog'],'retry and close');
  await sleep(700);const files=readdirSync(out).filter(name=>name.endsWith('.ngc'));
  if(files.length!==1||createHash('sha256').update(readFileSync(path.join(out,files[0]))).digest('hex')!==sha)throw Error('Retry changed checked bytes');
  record('retry downloads exact prepared bytes and closes review',await state());
  if(scale!==1)return;
  await control('Simulate');await control('Renderer diagnostics');await control('Inject renderer failure');
  await waitFor(s=>s.renderer.unavailable,'renderer failure');await screenshot('v6-renderer-failure.png');
  await control('Restore viewport');await waitFor(s=>!s.renderer.unavailable,'restore viewport');
  record('visible viewport action restores retained renderer',await state());
  await control('Cutting');await edit('Roughing feed','-');
  await waitFor(s=>s.pending,'partial input');await screenshot('v6-partial.png');
  await sleep(2000);await send('Page.reload');await waitFor(s=>s.controls?.['Restore draft'],'recovery offer');
  await screenshot('v6-recovery-offer.png');await control('Restore draft');
  await waitFor(s=>s.pending&&!s.active,'restored partial field');record('recovery restores partial draft',await state());
  const invalid=structuredClone(base);delete invalid.operations[1].settings.settings.endmill.cutting_feed_mm_min;
  await drop(invalid);
  await control('Operation row '+base.operations[0].id);await control('Generate');
  const failed=await waitFor(s=>!s.active&&s.issues?.some(i=>i.operation_id===second.id),'diagnostic for another operation');
  const index=failed.issues.findIndex(i=>i.operation_id===second.id&&i.field_path);
  if(index<0)throw Error('Expected located operation issue');
  await screenshot('v6-issues.png');await control('Issue '+index);
  const routed=await waitFor(s=>s.job.selectedOperation===second.id,'issue selects owner');
  record('issue action selects owning operation',routed);await screenshot('v6-issue-destination.png');
}
