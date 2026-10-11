import http from 'node:http';
import {readFile,writeFile} from 'node:fs/promises';
import path from 'node:path';
import {fileURLToPath} from 'node:url';
const root=fileURLToPath(new URL('.',import.meta.url));
const artifacts=path.resolve(root,'../../artifacts/sketcher-feasibility');
http.createServer(async(req,res)=>{
  try {
    const url=new URL(req.url,'http://127.0.0.1');
    if(req.method==='POST'&&/^\/results\/(arael-browser|planegcs-browser|browser-lifecycle|browser-error)\.json$/.test(url.pathname)) {
      let data='';for await(const part of req){data+=part;if(data.length>20000000)throw Error('result too large');}
      JSON.parse(data);await writeFile(path.join(artifacts,path.basename(url.pathname)),data);res.end('saved');return;
    }
    if(req.method!=='GET'){res.writeHead(405);res.end();return;}
    const base=url.pathname.startsWith('/plane/')?path.join(artifacts,'upstream/plane-package/package/dist/planegcs_dist'):root;
    const relative=url.pathname.startsWith('/plane/')?url.pathname.slice(7):url.pathname==='/'?'index.html':url.pathname.slice(1);
    const file=path.resolve(base,decodeURIComponent(relative));if(!file.startsWith(base.replace(/[\\/]$/,'')+path.sep))throw Error('path');
    res.setHeader('Content-Type',file.endsWith('.wasm')?'application/wasm':/\.m?js$/.test(file)?'text/javascript':file.endsWith('.html')?'text/html':'application/json');
    res.setHeader('Cache-Control','no-store');res.end(await readFile(file));
  }catch(e){res.writeHead(404);res.end(String(e));}
}).listen(5193,'127.0.0.1',()=>console.log('Sketcher experiment: http://127.0.0.1:5193/'));
