// Executes the unchanged TS engine/setup via Node's built-in type erasure.
// Native outputs include every cell, not selected samples or image similarities.
import {stripTypeScriptTypes} from 'node:module';
import {readFileSync,writeFileSync,mkdirSync} from 'node:fs';
import {spawnSync} from 'node:child_process';
import {createHash} from 'node:crypto';
import {fileURLToPath} from 'node:url';
import path from 'node:path';
import assert from 'node:assert/strict';
const root=fileURLToPath(new URL('..',import.meta.url));
const out=path.join(root,'artifacts/simulation');mkdirSync(out,{recursive:true});
const enginePath=path.resolve(root,'../../web/src/sim/engine.ts');
const setupPath=path.resolve(root,'../../web/src/sim/setup.ts');
const uri=s=>'data:text/javascript;base64,'+Buffer.from(s).toString('base64');
const engineUri=uri(stripTypeScriptTypes(readFileSync(enginePath,'utf8')));
const engine=await import(engineUri);
const setup=await import(uri(stripTypeScriptTypes(readFileSync(setupPath,'utf8')).replace("'./engine'",JSON.stringify(engineUri))));
const hash=b=>createHash('sha256').update(b).digest('hex');
const exe=path.join(root,'target/release/cam-gui1-desktop.exe');
function run(args){const r=spawnSync(exe,args,{windowsHide:true,timeout:600_000,encoding:'utf8'});assert.equal(r.status,0,`${r.error??''} ${r.stderr}`);}
function bytes(field){const b=Buffer.alloc(field.cols*field.rows*3);let offset=0;
 for(let row=0;row<field.rows;row++)for(let col=0;col<field.cols;col++){
  const tile=(row>>8)*field.tilesX+(col>>8),local=((row&255)<<8)|(col&255);
  b.writeUInt16LE(field.heights[tile]?.[local]??0,offset);b[offset+2]=field.cellOwner[tile]?.[local]??0;offset+=3;
 }return b;
}
const evidence={node:process.version,engineSha256:hash(readFileSync(enginePath)),setupSha256:hash(readFileSync(setupPath)),nativeSha256:hash(readFileSync(exe)),cases:[]};
function compare(name,input){
 const directory=path.join(out,name);mkdirSync(directory,{recursive:true});
 const inputPath=path.join(directory,'input.json');writeFileSync(inputPath,JSON.stringify(input));
 run(['--simulate',inputPath,directory]);
 const native=JSON.parse(readFileSync(path.join(directory,'report.json'),'utf8'));
 const record={name,inputSha256:hash(readFileSync(inputPath)),cellMm:input.resolution.cellMm,stock:input.stock,snapshots:[]};
 let field=engine.createField(input.stock,input.tools.map(engine.normalizeTool),input.resolution);let position=0;
 for(let index=0;index<input.prefixes.length;index++){
  const prefix=input.prefixes[index];
  if(prefix<position){field=engine.createField(input.stock,input.tools.map(engine.normalizeTool),input.resolution);position=0;}
  const begin=performance.now();while(position<prefix)engine.applyMotion(field,input.motions[position++]);const tsMs=performance.now()-begin;
  const expected=bytes(field),actual=readFileSync(path.join(directory,`${index}.cells`));
  assert.equal(actual.length,expected.length,`${name}/${prefix} cell count`);
  let differingDepth=0,differingOwner=0,maxLevels=0;
  for(let offset=0;offset<actual.length;offset+=3){const delta=Math.abs(actual.readUInt16LE(offset)-expected.readUInt16LE(offset));if(delta){differingDepth++;maxLevels=Math.max(maxLevels,delta);}if(actual[offset+2]!==expected[offset+2])differingOwner++;}
  assert.equal(differingDepth,0,`${name}/${prefix}: ${differingDepth} depth differences (max ${maxLevels} levels)`);
  assert.equal(differingOwner,0,`${name}/${prefix}: owner differences`);
  assert.equal(native[index].checksum,engine.checksum(field),`${name}/${prefix} versions/checksum`);
  assert.deepEqual(native[index].versions,Array.from(field.tileVersions));
  for(const key of ['appliedMotions','cuttingMotions','dirtyCells'])assert.equal(native[index].stats[key],field.stats[key]);
  for(const [a,b] of [[native[index].stats.removedVolumeMm3,field.stats.removedVolumeMm3],...native[index].stats.stageRemovedMm3.map((v,i)=>[v,field.stats.stageRemovedMm3[i]])])assert.ok(Math.abs(a-b)<=1e-10*Math.max(1,Math.abs(b)));
  record.snapshots.push({...native[index],versions:undefined,tsMs,depthDifferences:differingDepth,ownerDifferences:differingOwner});
  console.log(`${name} prefix ${prefix}: ${field.cols}×${field.rows}, all cells/owners/versions equal; Rust ${native[index].seekMs.toFixed(1)} ms / TS ${tsMs.toFixed(1)} ms`);
 }
 evidence.cases.push(record);writeFileSync(path.join(out,process.argv.includes('--preview-only')?'preview-comparison.json':'comparison.json'),JSON.stringify(evidence,null,2)+'\n');
}
// Tile edges, ramps both ways, finite/pointed tips, over-thickness cuts,
// off-stock and non-cutting moves, with deterministic randomized coverage.
const motions=[];let seed=0x5eed;const rand=()=>{seed=(Math.imul(seed,1664525)+1013904223)>>>0;return seed/2**32;};
for(let i=0;i<80;i++){const x=(rand()-0.5)*12,y=(rand()-0.5)*12;
 motions.push({kind:i%7===0?'rapid_x_y':i%5===0?'plunge':'ramp',tool:i%3,x0:x,y0:y,z0:rand()*2-4,x1:i%5===0?x:(rand()-0.5)*12,y1:i%5===0?y:(rand()-0.5)*12,z1:rand()*4-5});}
compare('analytic-random',{stock:{x0:-5,y0:-5,x1:5,y1:5,thicknessMm:4},tools:[{kind:'endmill',diameterMm:2},{kind:'vbit',includedAngleDeg:90,tipDiameterMm:0.4,maxCuttingDiameterMm:4,cuttingHeightMm:1},{kind:'vbit',includedAngleDeg:60,tipDiameterMm:0,maxCuttingDiameterMm:4,cuttingHeightMm:3}],resolution:{cellMm:0.025,cappedByTexels:false,cappedByBudget:false},motions,prefixes:[0,20,80,7,55,80]});
for(const name of process.argv.includes('--small-only')?['small']:['small','flower']){
 const fixturePath=path.join(out,`${name}-fixture.json`);run(['--sim-fixture',name,fixturePath]);
 const fixture=JSON.parse(readFileSync(fixturePath,'utf8'));
 const tsSetup=setup.buildSimSetup(fixture.job,fixture.motions,fixture.slices);
 assert.deepEqual(fixture.input.stock,tsSetup.stock,`${name} stock setup`);
 assert.deepEqual(fixture.input.resolution,tsSetup.resolution,`${name} grid setup`);
 assert.deepEqual(fixture.input.tools,tsSetup.tools,`${name} tools`);
 if(!process.argv.includes('--preview-only'))compare(name,fixture.input);
 const preview=structuredClone(fixture.input);
 preview.resolution.cellMm=Math.max(preview.resolution.cellMm,(preview.stock.x1-preview.stock.x0)/512,(preview.stock.y1-preview.stock.y0)/512);
 preview.prefixes=[...new Set([...Array.from({length:17},(_,i)=>Math.floor(preview.motions.length*i/16)),fixture.input.prefixes[1]])].sort((a,b)=>a-b);
 compare(`${name}-preview`,preview);
}
console.log(`Evidence: ${path.join(out,process.argv.includes('--preview-only')?'preview-comparison.json':'comparison.json')}`);
