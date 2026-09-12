// Explicit synthetic review data, never New-job defaults. Run from the Rust
// workspace after `cargo build -p cam-gui --locked`.
import {spawn} from 'node:child_process';
import {mkdtempSync,readFileSync,writeFileSync,renameSync,existsSync,unlinkSync} from 'node:fs';
import {tmpdir} from 'node:os';
import path from 'node:path';
const folder=mkdtempSync(path.join(tmpdir(),'gui6-fixture-'));
const flower=process.argv.includes('--flower');
const executable=process.argv.slice(2).find(a=>!a.startsWith('--'))??'target/debug/cam-gui.exe';
const worker=spawn(path.resolve(executable),['--worker',folder],{windowsHide:true,stdio:'ignore'});
const sleep=ms=>new Promise(resolve=>setTimeout(resolve,ms));
async function request(command) {
  writeFileSync(path.join(folder,'pending'),JSON.stringify({Gui2:command}));
  renameSync(path.join(folder,'pending'),path.join(folder,'request'));
  const response=path.join(folder,'response'),deadline=Date.now()+60000;
  while(!existsSync(response)){if(worker.exitCode!==null||Date.now()>deadline)throw Error('Fixture worker failed or timed out');await sleep(20);}
  const bytes=readFileSync(response);unlinkSync(response);
  const metadata=JSON.parse(bytes.subarray(4,4+bytes.readUInt32LE(0)).toString());
  if(metadata.Err)throw Error(metadata.Err);
  return metadata.Ok;
}
try {
  const filename=flower?'flower-centerlines.svg':'chains.svg';
  let svg=readFileSync(flower?'../real_data/flower_box.svg':'fixtures/gui6/chains.svg','utf8');
  if(flower) {
    // Deliberate review-source conversion: preserve every path coordinate and
    // page dimension; explicitly request cutting the drawn path as a centerline.
    if(!svg.includes('style="fill:#000000"'))throw Error('Flower source style changed; review conversion');
    svg=svg.replace('style="fill:#000000"','style="fill:none;stroke:#000000;stroke-width:0.1"');
    writeFileSync('fixtures/gui6/flower-centerlines.svg',svg);
  }
  const imported=await request({ImportKnifeSvg:{filename,svg}});
  let job=JSON.parse(imported.job);
  const selectedChains=flower?[...imported.report.gui2.chains].filter(c=>c.closed).sort((a,b)=>a.vertices.length-b.vertices.length).slice(0,1):imported.report.gui2.chains;
  const selected=await request({Artwork:{job:JSON.stringify(job),action:{KnifeSelection:{references:selectedChains.map(c=>c.reference)}}}});
  job=JSON.parse(selected.job);
  job.name=flower?'GUI6 flower knife review':'GUI6 knife review';job.setup.stock.thickness_mm=2;job.setup.clearance_above_stock_mm=5;job.setup.start_xy_mm={x:0,y:0};
  job.tools[0].geometry={kind:'drag_knife',dimensions:{blade_offset_mm:1,max_cut_depth_mm:2}};
  Object.assign(job.operations[0].settings.settings,{stepdown_mm:1,swivel_depth_mm:0.5,corner_threshold_deg:20,alignment:{initial_heading_deg:180}});
  job.operations[0].settings.settings.bottom.offset_mm=-1;
  Object.assign(job.operations[0].settings.settings.assignment,{cutting_feed_mm_min:150,plunge_feed_mm_min:50,swivel_feed_mm_min:75,max_stepdown_mm:1});
  const machine=JSON.parse(readFileSync('fixtures/gui2/machine.json','utf8'));
  machine.id='gui6-review-machine';machine.tools[0].tool_id='knife-tool';machine.tools.length=1;
  const applied=await request({ApplyProfile:{job:JSON.stringify(job),json:JSON.stringify(machine)}});
  const generated=await request({Generate:{job:applied.job}});
  if(!generated.report.gui2.checks?.exportReady)throw Error(JSON.stringify(generated.report));
  writeFileSync(flower?'fixtures/gui6/flower-knife.job.json':'fixtures/gui6/knife.job.json',applied.job+'\n');
  writeFileSync('fixtures/gui6/machine.json',JSON.stringify(machine,null,2)+'\n');
  console.log(`Created checked GUI6 fixture: ${generated.motions} motions; selected ${selectedChains.map(c=>c.reference.local_geometry_id).join(', ')}. Temporary mailbox: ${folder}`);
} finally {worker.kill();}
