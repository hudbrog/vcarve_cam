// Dependency-free localhost review server; handles wasm MIME and rejects traversal.
import http from 'node:http';
import {readFile} from 'node:fs/promises';
import {fileURLToPath} from 'node:url';
import path from 'node:path';
const root=path.resolve(fileURLToPath(new URL('..',import.meta.url)));
http.createServer(async(req,res)=>{
  try{
    const url=new URL(req.url,'http://127.0.0.1');
    const relative=decodeURIComponent(url.pathname==='/'?'/web/index.html':url.pathname);
    const file=path.resolve(root,`.${relative}`);
    if(!file.startsWith(root+path.sep)) {res.writeHead(403);res.end();return;}
    const bytes=await readFile(file);
    res.setHeader('Content-Type',file.endsWith('.wasm')?'application/wasm':file.endsWith('.js')||file.endsWith('.mjs')?'text/javascript':file.endsWith('.html')?'text/html':'application/octet-stream');
    res.setHeader('Cache-Control','no-cache');res.end(bytes);
  }catch{res.writeHead(404);res.end('Not found');}
}).listen(5181,'127.0.0.1',()=>console.log('GUI1: http://127.0.0.1:5181/web/index.html'));
