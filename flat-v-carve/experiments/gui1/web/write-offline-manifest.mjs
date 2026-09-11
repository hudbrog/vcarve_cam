import {readFileSync,writeFileSync,readdirSync} from 'node:fs';
import {createHash} from 'node:crypto';
import {fileURLToPath} from 'node:url';
import path from 'node:path';
const root=fileURLToPath(new URL('..',import.meta.url));
const assets=['web/index.html','web/worker.js','web/recovery-store.js'];
function walk(relative){for(const entry of readdirSync(path.join(root,relative),{withFileTypes:true})){
 const file=`${relative}/${entry.name}`;if(entry.isDirectory())walk(file);else if(/\.(js|wasm)$/.test(file))assets.push(file);
}}
walk('pkg');assets.sort();const hash=createHash('sha256');
for(const file of assets){hash.update(file);hash.update(readFileSync(path.join(root,file)));}
const manifest={version:hash.digest('hex'),assets:assets.map(file=>`../${file}`)};
writeFileSync(path.join(root,'web/offline-manifest.js'),`self.GUI1_OFFLINE=${JSON.stringify(manifest)};\n`);
console.log(`Offline build ${manifest.version.slice(0,12)} · ${assets.length} assets`);
