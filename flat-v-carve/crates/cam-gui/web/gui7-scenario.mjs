// GUI7 facing and ordered preparation in a real browser: a source-free face
// job from creation to checked output, then Face → Flat V-carve ordering with
// height dependencies, prefix generation, stock context and dependency repair.
import {createHash} from 'node:crypto';
import {readdirSync} from 'node:fs';

export async function gui7Scenario({control,edit,state,waitFor,send,evaluate,sleep,record,screenshot,readFileSync,pressKey,path,out}) {
  // Simulate navigates to the inspection tab, which has no field filter: return
  // to the operation editor first.
  const clearSearch=async()=>{
    if(!(await state()).controls['Filter fields'])await control('Cutting');
    await control('Filter fields');await pressKey('a','KeyA',2);await pressKey('Backspace','Backspace');await sleep(150);
  };

  // --- GUI7a: a source-free face job, created with nothing invented. -------
  await control('File');await control('New face job');
  await waitFor(s=>s.job?.kind==='face'&&!s.active,'new source-free face job');
  const fresh=await state();
  // Unset optional values are omitted from the document, not zeroed.
  if(fresh.job.face.stepdown_mm!=null||fresh.job.face.assignment.cutting_feed_mm_min!=null)throw new Error('New face job invented machining values');
  if(fresh.job.artworks.length)throw new Error('Face job carries artwork');
  await screenshot('gui7-face-new.png');

  await control('Setup');
  await edit('Stock thickness','6');
  await edit('Stock minimum X','0');await edit('Stock minimum Y','0');
  await edit('Stock width','40');await edit('Stock length','30');
  await clearSearch();
  await control('Cutting');
  await edit('Stepdown','1');await edit('Stepover','1.5');
  await edit('Tool stepdown limit','1');
  await edit('Roughing feed','300');await edit('Plunge feed','100');
  await edit('Spindle speed','12000');
  await edit('Face pass angle','0');
  await edit('Endmill diameter','3');await edit('Cutting length','8');
  await edit('Face bottom offset','-1');
  await edit('Face margin min X','0');
  await edit('Face entry overrun','0');
  await clearSearch();
  await screenshot('gui7-face-settings.png');
  await control('Face CW');
  await waitFor(s=>!s.active&&s.job?.face?.assignment?.spindle_direction==='clockwise','explicit face spindle direction');

  await control('Machine');await control('Example machine');
  await control('Apply flower machine profile');
  await waitFor(s=>!s.active&&s.job.machine,'applied machine for checked export');
  await control('Cutting');
  await control('Generate');
  await waitFor(s=>s.current&&!s.active&&s.motions>0,'face generation',180);
  const generated=await state();
  if(!generated.exportReady)throw new Error('Face generation is not export-ready: '+generated.status);
  record('source-free face generation',{motions:generated.motions,revision:generated.revision});

  // Actual heightfield playback: the facing depth is real removal.
  await control('Simulate');await control('After face');
  await waitFor(s=>!s.active&&s.stockPrefix===s.motions,'face stock playback');
  await screenshot('gui7-face-playback.png');
  const removed=(await state()).inspection?.sample?.depth??0;
  record('face playback removed depth',removed);

  await control('Prepare checked output');
  await waitFor(s=>s.prepared&&!s.active||!s.active&&s.status.includes('"code"'),'face output',180);
  if(!(await state()).prepared)throw new Error('Face export failed: '+(await state()).status);
  const sha=(await state()).preparedSha256;
  await evaluate('globalThis.showSaveFilePicker=undefined');
  await control('Save as…');
  await waitFor(s=>s.status.includes('Download requested'),'face output save');
  await sleep(500);
  const file=readdirSync(out).find(n=>n.endsWith('.ngc'));
  if(!file||createHash('sha256').update(readFileSync(path.join(out,file))).digest('hex')!==sha)throw new Error('Saved face bytes differ from the checked bytes');
  // Portable save and reopen: the face job survives as one schema-5 document.
  await control('Save job');await waitFor(s=>s.status.includes('Download requested'),'face job save');await sleep(500);
  const saved=readdirSync(out).find(n=>n.endsWith('.job.json'));
  const portable=readFileSync(path.join(out,saved),'utf8');
  await evaluate(`(()=>{const t=new DataTransfer();t.items.add(new File([${JSON.stringify(portable)}],'face-reopen.job.json',{type:'application/json'}));document.getElementById('cam').dispatchEvent(new DragEvent('drop',{dataTransfer:t,bubbles:true,cancelable:true}));})()`);
  await waitFor(s=>s.job?.kind==='face'&&!s.active&&!s.pending,'face job reopen');
  await control('Generate');await waitFor(s=>s.current&&!s.active&&s.exportReady,'reopened face generation',180);
  record('face job save and reopen',await state());

  // --- GUI7c: Face → Flat V-carve, prefix generation and repair. ----------
  const carving=readFileSync('fixtures/gui4/lettering.job.json','utf8');
  await evaluate(`(()=>{const t=new DataTransfer();t.items.add(new File([${JSON.stringify(carving)}],'lettering.job.json',{type:'application/json'}));document.getElementById('cam').dispatchEvent(new DragEvent('drop',{dataTransfer:t,bubbles:true,cancelable:true}));})()`);
  await waitFor(s=>s.job?.kind==='flat_vcarve'&&!s.active,'lettering fixture reopened');
  await control('Add operation');await control('Add Face — face-1');
  await waitFor(s=>s.job?.kind==='face'&&s.job.operations.length===2,'ordered face operation added');
  await edit('Stepdown','0.5');await edit('Stepover','1.5');
  await edit('Tool stepdown limit','0.5');
  await edit('Roughing feed','300');await edit('Plunge feed','100');await edit('Spindle speed','12000');
  await edit('Face bottom offset','-0.5');
  await edit('Endmill diameter','6');await edit('Cutting length','8');
  await clearSearch();
  await control('Face CW');
  await control('Move earlier');
  await waitFor(s=>!s.active&&s.job.operations[0].id==='face-1','face moved before the carving');
  await control('Generate');
  await waitFor(s=>s.current&&!s.active&&s.motions>0,'face then carve generation',240);
  const sequence=await state();
  if(!sequence.exportReady)throw new Error('Face then carve is not export-ready: '+sequence.status);
  await control('Simulate');await control('After face');
  await waitFor(s=>!s.active&&s.stockPrefix>0&&s.stockPrefix<sequence.motions,'stock after the face operation');
  await screenshot('gui7-face-then-carve.png');
  // The lettering carving is endmill-only, so its single stage is the last one.
  await control('After endmill');
  await waitFor(s=>!s.active&&s.stockPrefix===sequence.motions,'stock after the carving');
  record('ordered face then carve with per-operation stock',await state());

  // The carving may start from the plane the face published.
  await control('Operation row carving');
  await waitFor(s=>!s.active&&s.job.selectedOperation==='carving','carving selected');
  await control('Cutting');await control('Top: face-1 face result');
  await waitFor(s=>!s.active,'face plane reference selected');
  await control('Generate');
  await waitFor(s=>s.current&&!s.active,'carve from the faced plane',240);
  await clearSearch();
  // Reordering across the dependency is a saveable unresolved state, and the
  // located issue routes to the field that owns it.
  await control('Operation row face-1');
  await control('Move later');
  await waitFor(s=>!s.active&&s.job.operations[0].id!=='face-1','face moved after the carving');
  await control('Generate');
  await waitFor(s=>!s.active&&(s.issues??[]).some(issue=>['HEIGHT_REFERENCE_FORWARD','HEIGHT_REFERENCE_UNRESOLVED'].includes(issue.code)),'located unresolved height reference');
  await screenshot('gui7-unresolved-dependency.png');
  await control('Operation row face-1');
  await control('Move earlier');
  await waitFor(s=>!s.active&&s.job.operations[0].id==='face-1','face moved back before the carving');
  await control('Generate');
  await waitFor(s=>s.current&&!s.active&&s.exportReady,'dependency repaired by reordering',240);
  record('ordered preparation, per-operation stock and dependency repair',await state());

  // --- GUI7d: a supported Face → drag knife sequence. ---------------------
  const knife=readFileSync('fixtures/gui6/knife.job.json','utf8');
  await evaluate(`(()=>{const t=new DataTransfer();t.items.add(new File([${JSON.stringify(knife)}],'knife.job.json',{type:'application/json'}));document.getElementById('cam').dispatchEvent(new DragEvent('drop',{dataTransfer:t,bubbles:true,cancelable:true}));})()`);
  await waitFor(s=>s.job?.kind==='drag_knife'&&!s.active,'knife fixture reopened');
  await control('Add operation');await control('Add Face — face-1');
  await waitFor(s=>s.job?.kind==='face'&&s.job.operations.length===2,'face operation added to the knife job');
  await edit('Stepdown','0.5');await edit('Stepover','1.5');
  await edit('Tool stepdown limit','0.5');
  await edit('Roughing feed','300');await edit('Plunge feed','100');await edit('Spindle speed','12000');
  await edit('Face bottom offset','-0.5');
  await edit('Endmill diameter','6');await edit('Cutting length','8');
  await clearSearch();
  await control('Face CW');
  await control('Move earlier');
  await waitFor(s=>!s.active&&s.job.operations[0].id==='face-1','face moved before the knife');
  await control('Generate');
  await waitFor(s=>s.current&&!s.active&&s.motions>0,'face then knife generation',240);
  const mixed=(await state()).motions;
  await control('Simulate');await control('After face');
  await waitFor(s=>!s.active&&s.stockPrefix>0&&s.stockPrefix<mixed,'stock before the knife');
  await control('After knife');
  await waitFor(s=>!s.active,'knife playback over intact stock');
  await screenshot('gui7-face-then-knife.png');
  await clearSearch();
  // A blade planted in material the face already removed is rejected with a
  // located reason instead of authorizing the mixed sequence.
  await control('Operation row face-1');
  await edit('Face bottom offset','-2');
  await control('Generate');
  await waitFor(s=>!s.active&&(s.issues??[]).some(issue=>issue.code==='KNIFE_CONTACT_UNSUPPORTED'),'located knife contact rejection',240);
  await screenshot('gui7-knife-contact-rejected.png');
  await edit('Face bottom offset','-0.5');
  await control('Generate');
  await waitFor(s=>s.current&&!s.active&&s.exportReady,'supported face then knife repaired',240);
  record('supported mixed milling/knife sequence and contact rejection',await state());
}
