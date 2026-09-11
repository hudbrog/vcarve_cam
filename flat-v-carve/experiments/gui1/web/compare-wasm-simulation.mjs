// Run after compare-simulation.mjs --preview-only and wasm-pack build.
// Exercise the actual Rust WASM compute entry point, then compare every returned
// packed cell against the native/TypeScript-qualified preview snapshots.
import {readFileSync,writeFileSync} from 'node:fs';
import {createHash} from 'node:crypto';
import assert from 'node:assert/strict';
import {fileURLToPath} from 'node:url';
import path from 'node:path';
const root=fileURLToPath(new URL('..',import.meta.url));
const wasm=readFileSync(path.join(root,'pkg/cam_gui1_bg.wasm'));
const gui=await import('../pkg/cam_gui1.js');gui.initSync({module:wasm});
assert.equal(gui.protocol(),'gui1-spike-2');
const out=path.join(root,'artifacts/simulation');const records=[];
for(const name of ['small','flower']){
 const begin=performance.now();const result=JSON.parse(gui.compute(JSON.stringify({Reference:{flower:name==='flower',export:false}})));
 assert.ok(result.Ok,result.Err);const preview=result.Ok.stock_preview;
 assert.ok(preview);assert.ok(preview.retained_bytes<=20*1024*1024);
 const native=JSON.parse(readFileSync(path.join(out,`${name}-preview/report.json`),'utf8'));
 assert.equal(preview.frames.length,native.length);
 const snapshots=[];
 for(let i=0;i<native.length;i++){
  const frame=preview.frames[i],expected=readFileSync(path.join(out,`${name}-preview/${i}.cells`));
  assert.equal(frame.prefix,native[i].prefix);assert.equal(frame.checksum,native[i].checksum);
  assert.equal(frame.cells.length*3,expected.length);
  for(let j=0;j<frame.cells.length;j++)assert.equal(frame.cells[j],expected.readUInt16LE(j*3)|(expected[j*3+2]<<16),`${name}/${i}/${j}`);
  snapshots.push({prefix:frame.prefix,checksum:frame.checksum,cellCount:frame.cells.length,depthDifferences:0,ownerDifferences:0});
 }
 records.push({name,elapsedMs:performance.now()-begin,cols:preview.cols,rows:preview.rows,cellMm:preview.cell_mm,retainedCellBytes:preview.retained_bytes,snapshots});
 console.log(`${name}: WASM matched all ${snapshots.length} native/TS-qualified checkpoints`);
}
writeFileSync(path.join(out,'wasm-comparison.json'),JSON.stringify({node:process.version,wasmSha256:createHash('sha256').update(wasm).digest('hex'),records},null,2)+'\n');
