// Dependency-free localhost review server; handles wasm MIME and rejects traversal.
import http from 'node:http';
import {readFile} from 'node:fs/promises';
import {fileURLToPath} from 'node:url';
import path from 'node:path';
const root=path.resolve(fileURLToPath(new URL('..',import.meta.url)));
// The browser probe pages load the unchanged repository fixtures, which live
// beside the experiment directory rather than inside it.
const fixtures=path.resolve(root,'../../fixtures');
http.createServer(async(req,res)=>{
  try{
    const url=new URL(req.url,'http://127.0.0.1');
    const relative=decodeURIComponent(url.pathname==='/'?'/web/index.html':url.pathname);
    const fixturesRequest=relative.startsWith('/fixtures/');
    const base=fixturesRequest?fixtures:root;
    const file=path.resolve(base,fixturesRequest?`.${relative.slice('/fixtures'.length)}`:`.${relative}`);
    if(!file.startsWith(base+path.sep)) {res.writeHead(403);res.end();return;}
    const bytes=await readFile(file);
    res.setHeader('Content-Type',file.endsWith('.wasm')?'application/wasm':file.endsWith('.js')||file.endsWith('.mjs')?'text/javascript':file.endsWith('.html')?'text/html':'application/octet-stream');
    res.setHeader('Cache-Control','no-cache');res.end(bytes);
  }catch{res.writeHead(404);res.end('Not found');}
}).listen(5182,'127.0.0.1',()=>console.log('CAM_GUI: http://127.0.0.1:5182/web/index.html'));
