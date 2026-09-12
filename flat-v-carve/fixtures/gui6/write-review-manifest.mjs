// Record local review packages and their exact input source tree. Run from the
// Rust workspace after both final builds. Generated evidence stays in artifacts.
import {execFileSync} from 'node:child_process';
import {readFileSync,writeFileSync} from 'node:fs';
import {createHash} from 'node:crypto';
const sha=bytes=>createHash('sha256').update(bytes).digest('hex');
const files=[...new Set(execFileSync('git',['ls-files','-co','--exclude-standard'],{encoding:'utf8'}).trim().split(/\r?\n/))]
  .filter(p=>/^(crates|fixtures|scripts)\//.test(p)||/^Cargo\.(toml|lock)$/.test(p)).sort();
const sources=files.map(path=>({path,sha256:sha(readFileSync(path))}));
const nativePath=process.argv.find(a=>a.startsWith('--native='))?.slice(9)??'artifacts/gui6/review/native/cam-gui.exe';
const offline=JSON.parse(readFileSync('crates/cam-gui/web/offline-manifest.js','utf8').split('=',2)[1].replace(/;\s*$/,''));
const result={createdAt:new Date().toISOString(),head:execFileSync('git',['rev-parse','HEAD'],{encoding:'utf8'}).trim(),uncommitted:true,
  sourceTreeSha256:sha(JSON.stringify(sources)),native:{path:nativePath,sha256:sha(readFileSync(nativePath))},
  browser:{path:'artifacts/gui6/review/browser',offlineBuild:offline.version},sources};
writeFileSync('artifacts/gui6/review/manifest.json',JSON.stringify(result,null,2)+'\n');
console.log(JSON.stringify({...result,sources:`${sources.length} files`},null,2));
