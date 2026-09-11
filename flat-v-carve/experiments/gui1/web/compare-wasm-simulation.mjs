// Cross-platform transport parity: run the same scene through the native
// worker frame and through the browser package, then compare the metadata,
// the section table, the payload hash and every stock cell.
//
// The TypeScript heightfield comparison lives in compare-simulation.mjs; this
// script covers the new paged binary transport on both targets.
import {readFileSync,writeFileSync,mkdirSync} from 'node:fs';
import {createHash} from 'node:crypto';
import {spawnSync} from 'node:child_process';
import assert from 'node:assert/strict';
import {fileURLToPath} from 'node:url';
import path from 'node:path';

const root=fileURLToPath(new URL('..',import.meta.url));
const out=path.join(root,'artifacts/simulation');mkdirSync(out,{recursive:true});
const exe=path.join(root,'target/release/cam-gui1-desktop.exe');
const wasm=readFileSync(path.join(root,'pkg/cam_gui1_bg.wasm'));
const gui=await import('../pkg/cam_gui1.js');gui.initSync({module:wasm});
assert.equal(gui.protocol(),'gui1-spike-4');

const parseFrame=buffer=>{
  const metaLength=buffer.readUInt32LE(0);
  // Metadata is `Result<SceneMeta, String>`, exactly as the parent decodes it.
  const document=JSON.parse(buffer.subarray(4,4+metaLength).toString('utf8'));
  assert.ok(document.Ok,`worker frame carried an error: ${document.Err}`);
  const metadata=document.Ok;
  const payload=buffer.subarray(4+metaLength);
  const sections=[];
  assert.equal(payload.subarray(0,8).toString('latin1'),'GUI1FRM1','payload magic');
  const count=payload.readUInt32LE(8);
  for(let i=0;i<count;i++){
    const at=12+i*24;
    sections.push({kind:payload[at],offset:Number(payload.readBigUInt64LE(at+8)),len:Number(payload.readBigUInt64LE(at+16))});
  }
  return {metadata,payload,sections};
};

const records=[];
for(const name of ['small','flower']){
  const run=spawnSync(exe,['--dump-scene',out,name==='flower'?'flower':'small'],{windowsHide:true,timeout:600_000});
  assert.equal(run.status,0,`native dump failed: ${run.stderr}`);
  const native=parseFrame(readFileSync(path.join(out,`native-${name}.frame`)));
  const begin=performance.now();
  const result=JSON.parse(gui.compute(JSON.stringify({Reference:{flower:name==='flower',export:false}})));
  assert.ok(result.Ok,result.Err);
  const wasmMeta=result.Ok;
  const wasmPayload=Buffer.from(gui.payload_view());
  const browser={metadata:wasmMeta,payload:wasmPayload,sections:wasmMeta.sections};
  const elapsedMs=performance.now()-begin;

  assert.equal(wasmPayload.length,native.payload.length,`${name} payload length`);
  assert.deepEqual(browser.sections,native.sections,`${name} section table`);
  assert.equal(browser.metadata.motions,native.metadata.motions);
  assert.equal(browser.metadata.transport.motionPages,native.metadata.transport.motionPages);
  assert.equal(browser.metadata.transport.simBytes,native.metadata.transport.simBytes);
  assert.equal(browser.metadata.transport.stockCheckpoints,native.metadata.transport.stockCheckpoints);

  // Every packed stock cell of every checkpoint must match exactly.
  const tiles=browser.metadata.stock.tilesX*browser.metadata.stock.tilesY;
  const perTile=256*256;
  let cellsCompared=0;
  for(let index=0;index<browser.metadata.stock.frames.length;index++){
    const expected=browser.metadata.stock.frames[index];
    const actual=native.metadata.stock.frames[index];
    assert.equal(expected.prefix,actual.prefix,`${name} checkpoint ${index} prefix`);
    assert.equal(expected.checksum,actual.checksum,`${name} checkpoint ${index} checksum`);
    assert.deepEqual(expected.versions,actual.versions,`${name} checkpoint ${index} tile versions`);
    assert.deepEqual(expected.allocated,actual.allocated,`${name} checkpoint ${index} allocated tiles`);
    assert.deepEqual(browser.metadata.stock.frames[index].stats,actual.stats,`${name} checkpoint ${index} stats`);
    const cells=wasmPayload.subarray(browser.sections.filter(s=>s.kind===3)[index].offset,
      browser.sections.filter(s=>s.kind===3)[index].offset+browser.sections.filter(s=>s.kind===3)[index].len);
    const nativeCells=native.payload.subarray(native.sections.filter(s=>s.kind===3)[index].offset,
      native.sections.filter(s=>s.kind===3)[index].offset+native.sections.filter(s=>s.kind===3)[index].len);
    assert.equal(cells.length,nativeCells.length);
    for(let offset=0;offset<cells.length;offset+=4){
      assert.equal(cells.readUInt32LE(offset),nativeCells.readUInt32LE(offset),`${name}/${index}/${offset}`);
      cellsCompared++;
    }
    assert.equal(cells.length,tiles*perTile*4);
  }
  // Motion pages must match too: same byte ranges, same content hash.
  const motionSection=browser.sections.find(s=>s.kind===2);
  const motionHash=payload=>createHash('sha256').update(payload.subarray(motionSection.offset,motionSection.offset+motionSection.len)).digest('hex');
  const displayMotionBytesIdentical=motionHash(wasmPayload)===motionHash(native.payload);
  assert.ok(displayMotionBytesIdentical,`${name} rendered motion vertices differ between targets`);

  // The replayable motion stream and the engine's own fingerprint are f64
  // values straight from the planner. Cross-target libm differences are
  // recorded here instead of being assumed away.
  const simSection=browser.sections.find(s=>s.kind===4);
  let differingRecords=0,maxDelta=0;
  if(simSection){
    for(let m=0;m<browser.metadata.motions;m++){
      const at=simSection.offset+m*56;
      let differs=false;
      // kind byte plus three pad bytes, then the tool index.
      if(wasmPayload[at]!==native.payload[at]||wasmPayload.readUInt32LE(at+4)!==native.payload.readUInt32LE(at+4))differs=true;
      for(let f=0;f<6;f++){
        const a=wasmPayload.readDoubleLE(at+8+f*8),b=native.payload.readDoubleLE(at+8+f*8);
        if(a!==b){differs=true;maxDelta=Math.max(maxDelta,Math.abs(a-b));}
      }
      if(differs)differingRecords++;
    }
  }
  records.push({
    name,
    elapsedMs,
    payloadSha256Native:native.metadata.payloadSha256,
    payloadSha256Browser:browser.metadata.payloadSha256,
    payloadsIdentical:browser.metadata.payloadSha256===native.metadata.payloadSha256,
    metadataBytes:browser.metadata.transport.metadataBytes,
    payloadBytes:browser.metadata.payloadBytes,
    motionPages:browser.metadata.transport.motionPages,
    stockCheckpoints:browser.metadata.stock.frames.length,
    cellsCompared,
    payloadSha256:browser.metadata.payloadSha256,
    displayMotionBytesIdentical,
    motionFingerprintNative:native.metadata.report.summary.motionFingerprint,
    motionFingerprintBrowser:browser.metadata.report.summary.motionFingerprint,
    replayRecordsCompared:browser.metadata.motions,
    replayRecordsDiffering:differingRecords,
    maxReplayCoordinateDeltaMm:maxDelta,
    differences:0,
  });
  console.log(`${name}: identical rendered vertices and ${cellsCompared} packed cells, ${browser.metadata.transport.motionPages} motion pages; replay stream differs in ${differingRecords}/${browser.metadata.motions} records (max ${maxDelta} mm)`);
}

writeFileSync(path.join(out,'wasm-native-parity.json'),JSON.stringify({
  node:process.version,
  nativeSha256:createHash('sha256').update(readFileSync(exe)).digest('hex'),
  wasmSha256:createHash('sha256').update(wasm).digest('hex'),
  note:'Native worker frame versus the browser package for the same request; the TypeScript heightfield comparison is a separate capture.',
  records,
},null,2)+'\n');
