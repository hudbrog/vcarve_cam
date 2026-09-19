import {isDeepStrictEqual as same} from 'node:util';

export async function machineAuthoringScenario({control,edit,state,waitFor,send,evaluate,sleep,record,screenshot,readFileSync,pressKey}) {
  await send('Emulation.setDeviceMetricsOverride',{width:1280,height:800,deviceScaleFactor:1,mobile:false});
  await sleep(300);
  const job=JSON.parse(readFileSync('fixtures/gui4/lettering.job.json','utf8'));
  job.name='Machine mapping review';
  job.operations.push({...structuredClone(job.operations[0]),id:'second-carve',name:'Shared cutter'});
  for(let i=1;i<=14;i++)job.tools.push({...structuredClone(job.tools[0]),id:`spare-${i}`,name:`Spare cutter ${i}`});
  const clear=async()=>{await control('Filter fields');await pressKey('a','KeyA',2);await pressKey('Backspace','Backspace');await sleep(180);};
  await evaluate(`(()=>{const t=new DataTransfer();t.items.add(new File([${JSON.stringify(JSON.stringify(job))}],'machine-review.job.json',{type:'application/json'}));document.getElementById('cam').dispatchEvent(new DragEvent('drop',{dataTransfer:t,bubbles:true,cancelable:true}));})()`);
  await waitFor(s=>s.job?.name===job.name&&!s.active,'shared tools fixture');
  const zero=(await state()).job.workZero;
  const mapping=(s,id)=>s.job.machineSnapshot.tools.find(t=>t.job_tool_id===id);
  await control('Machine');
  await control('Job compensation Tool table');
  await edit('Mapping endmill T','7');
  await waitFor(s=>!s.active&&mapping(s,'endmill')?.tool_number===7,'shared cutter mapping');
  await clear();await control('Operation row second-carve');await control('Machine');
  if(mapping(await state(),'endmill').tool_number!==7)throw Error('Shared tool mapping changed with operation selection');
  await edit('Mapping vbit H','-');await edit('Mapping spare-14 H','-');await clear();
  await control('Use T numbers for H entries');
  await waitFor(s=>!s.active&&mapping(s,'endmill').length_offset_number===7,'explicit matching H for assigned cutters');
  if(!(await state()).pending)throw Error('Matching H discarded unrelated unused-tool draft');
  await edit('Mapping spare-14 H','12');await clear();
  await waitFor(s=>!s.active&&!s.pending,'unused cutter partial repaired');
  const list=(await state()).controls['Machine mapping viewport'];
  if(!list||list[3]-list[1]>221)throw Error('Large tool mapping table is not bounded');
  record('job-wide mapping, shared cutters and independent partial H drafts',await state());
  await screenshot('machine-mappings-1280.png');

  await edit('Mapping spare-14 T','-');await clear();
  await control('Job settings');await control('Machine');
  if(!(await state()).pending)throw Error('Navigation discarded a mapping draft');
  await sleep(1200);await send('Page.reload');await sleep(700);
  await waitFor(s=>s.controls?.['Restore draft'],'mapping draft recovery offered');await control('Restore draft');
  await waitFor(s=>!s.active&&s.job?.name===job.name&&s.pending,'mapping draft restored');
  await control('Undo');await waitFor(s=>!s.active&&!s.pending,'recovered Undo removes last partial T');
  if(mapping(await state(),'spare-14').length_offset_number!==12)throw Error('Undo changed another mapping column');
  record('mapping draft navigation, browser recovery and Undo',await state());

  await control('Machine');await edit('Blend tolerance','-');await clear();await control('Job work offset G55');
  await waitFor(s=>!s.active&&s.job.machineSnapshot.work_offset==='G55','work offset selector');
  if(!(await state()).pending)throw Error('Work offset selection discarded an unrelated blend draft');
  await edit('Blend tolerance','0.05');await clear();
  await edit('Mapping spare-14 T','-');await clear();
  await control('Example machine');await control('Apply flower machine profile');
  await waitFor(s=>!s.active&&!s.pending&&s.job.machineSnapshot.work_offset==='G54','profile application replaces mappings and raw drafts');
  if(!same((await state()).job.workZero,zero))throw Error('Applying machine changed the work zero');
  await control('Undo');await waitFor(s=>!s.active&&s.pending&&s.job.machineSnapshot.work_offset==='G55','Undo restores profile and partial mapping');
  await control('Redo');await waitFor(s=>!s.active&&!s.pending,'Redo reapplies profile');
  if(!(await state()).controls['Reusable machine settings'])await control('Machine advanced');
  const clip=(await state()).controls['Inspector viewport'];
  await send('Input.dispatchMouseEvent',{type:'mouseMoved',x:(clip[0]+clip[2])/2,y:clip[3]-20});
  await send('Input.dispatchMouseEvent',{type:'mouseWheel',x:(clip[0]+clip[2])/2,y:clip[3]-20,deltaX:0,deltaY:400});
  await sleep(300);await screenshot('machine-advanced-1280.png');
  record('explicit machine replacement preserves datum and has reversible drafts',await state());
}
