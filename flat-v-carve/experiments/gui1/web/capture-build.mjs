// Record the current local artifacts without rerunning timing/engine captures.
import {readFileSync,writeFileSync} from 'node:fs';
import {createHash} from 'node:crypto';
import {gzipSync} from 'node:zlib';
const root=new URL('../',import.meta.url);
const files=['pkg/cam_gui1_bg.wasm','pkg/cam_gui1.js','target/release/cam-gui1-desktop.exe'];
const manifest=files.map(path=>{
  const bytes=readFileSync(new URL(path,root));
  return {path,bytes:bytes.length,gzipBytes:gzipSync(bytes).length,sha256:createHash('sha256').update(bytes).digest('hex')};
});
writeFileSync(new URL('../../../docs/flat-v-carve/gui1-evidence/build-manifest.json',root),JSON.stringify(manifest,null,2)+'\n');
console.log(JSON.stringify(manifest,null,2));
