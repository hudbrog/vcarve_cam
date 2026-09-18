// Stable frames of the production canvas. Used before and after each UI slice.
export async function visualReviewScenario({control,state,waitFor,send,evaluate,sleep,record,screenshot,readFileSync,chooseFile}) {
  const job=readFileSync('fixtures/gui4/lettering.job.json','utf8');
  await evaluate(`(()=>{const transfer=new DataTransfer();transfer.items.add(new File([${JSON.stringify(job)}],'lettering.job.json',{type:'application/json'}));document.getElementById('cam').dispatchEvent(new DragEvent('drop',{dataTransfer:transfer,bubbles:true,cancelable:true}));})()`);
  await waitFor(s=>s.job?.name&&!s.active,'review fixture');
  for(const [width,height,deviceScaleFactor] of [[1280,800,1],[1440,900,1],[1920,1080,1],[1280,800,1.5],[1280,800,2]]) {
    await send('Emulation.setDeviceMetricsOverride',{width,height,deviceScaleFactor,mobile:false});
    await sleep(250);
    for(const [label,name] of [['Artwork','artwork'],['Setup','stock'],['Cutting','operation'],['Machine','machine']]) {
      await control(label);await sleep(250);
      await screenshot(`visual-${name}-${width}x${height}-${deviceScaleFactor}.png`);
      record(`visual ${name} ${width}x${height}@${deviceScaleFactor}`,await state());
    }
  }
  await send('Emulation.setDeviceMetricsOverride',{width:1440,height:900,deviceScaleFactor:1,mobile:false});
  await sleep(500);
  await control('Generate');await waitFor(s=>s.current&&!s.active,'review generated',120);
  await control('Simulate');await control('After endmill');await waitFor(s=>!s.active&&s.stockPrefix===s.motions,'review final stock');
  await sleep(250);await screenshot('visual-simulation-1440x900-1.png');record('visual simulation',await state());
  await control('Tool library');await waitFor(s=>s.resources?.ready&&!s.resources?.busy,'review library');
  await chooseFile('Import library','fixtures/gui5/library.json');
  await waitFor(s=>s.resources.dirty&&s.resources.catalog.id==='gui5-lettering-library','review library imported');
  await sleep(250);await screenshot('visual-library-1440x900-1.png');record('visual library',await state());
}
