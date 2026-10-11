export async function loadPlane(url) {
  const init=(await import(url)).default;
  let memory;
  const mod=await init({locateFile:p=>new URL(p,url).href,instantiateWasm(imports,receive){
    WebAssembly.instantiateStreaming(fetch(new URL('planegcs.wasm',url)),imports).then(({instance,module})=>{
      memory=Object.values(instance.exports).find(v=>v instanceof WebAssembly.Memory);receive(instance,module);
    });return {};
  }});
  mod.heapBytes=()=>memory?.buffer.byteLength;return mod;
}
export function runPlane(mod,f) {
  const begin=performance.now(),s=new mod.GcsSystem(),objects=[];
  const own=x=>(objects.push(x),x);
  const indices=f.entities.map(e=>e.values.map((v,i)=>s.push_p_param(v,e.fixed?.[i]??false)));
  const points=indices.map(i=>own(s.make_point(i[0],i[1])));
  const lines=indices.map((v,i)=>f.entities[i].kind==='line'?own(s.make_line(...v)):null);
  const circles=indices.map((v,i)=>['circle','arc'].includes(f.entities[i].kind)?own(s.make_circle(...v.slice(0,3))):null);
  for(let i=0;i<f.entities.length;i++) if(f.entities[i].kind==='arc') {
    const v=f.entities[i].values,ix=indices[i],endpoints=[v[0]+v[2]*Math.cos(v[3]),v[1]+v[2]*Math.sin(v[3]),v[0]+v[2]*Math.cos(v[4]),v[1]+v[2]*Math.sin(v[4])].map(v=>s.push_p_param(v,false));
    const ar=own(s.make_arc(ix[0],ix[1],...endpoints,ix[3],ix[4],ix[2]));
    s.add_constraint_arc_rules(ar,100000+i,true,1);
  }
  const dimensions=f.constraints.map(c=>s.push_p_param(c.value??0,true));
  f.constraints.forEach((c,i)=>{
    const a=c.a,b=c.b??0,t=[i+1,true,1],d=dimensions[i];
    switch(c.kind) {
      case 'horizontal':s.add_constraint_horizontal_l(lines[a],...t);break;
      case 'vertical':s.add_constraint_vertical_l(lines[a],...t);break;
      case 'length':s.add_constraint_p2p_distance(points[a],own(s.make_point(indices[a][2],indices[a][3])),d,...t);break;
      case 'radius':s.add_constraint_circle_radius(circles[a],d,...t);break;
      case 'join':s.add_constraint_p2p_coincident(own(s.make_point(indices[a][2],indices[a][3])),points[b],...t);break;
      case 'parallel':s.add_constraint_parallel(lines[a],lines[b],...t);break;
      case 'angle':s.add_constraint_l2l_angle_ll(lines[a],lines[b],d,...t);break;
      case 'symmetry':s.add_constraint_p2p_symmetric_ppl(points[a],points[b],lines[c.c],...t);break;
      case 'perpendicular':s.add_constraint_perpendicular_ll(lines[a],lines[b],...t);break;
      case 'equal_length':s.add_constraint_equal_length(lines[a],lines[b],...t);break;
      case 'equal_radius':s.add_constraint_equal_radius_cc(circles[a],circles[b],...t);break;
      case 'tangent':s.add_constraint_tangent_lc(lines[a],circles[b],...t);break;
      case 'distance':s.add_constraint_p2p_distance(points[a],points[b],d,...t);break;
      case 'hdistance':s.add_constraint_difference(indices[b][0],indices[a][0],d,...t);break;
      case 'vdistance':s.add_constraint_difference(indices[b][1],indices[a][1],d,...t);break;
      case 'coincident':s.add_constraint_p2p_coincident(points[a],points[b],...t);break;
      case 'point_on_line':s.add_constraint_point_on_line_pl(points[a],lines[b],...t);break;
      case 'midpoint':s.add_constraint_point_on_line_pl(points[a],lines[b],...t);s.add_constraint_point_on_perp_bisector_pl(points[a],lines[b],...t);break;
      case 'point_on_circle':s.add_constraint_point_on_circle(points[a],circles[b],...t);break;
      default:throw Error(`Unsupported ${c.kind}`);
    }
  });
  const build_ms=performance.now()-begin;
  const read=()=>indices.map(is=>is.map(i=>s.get_p_param(i)));
  const ints=v=>{try{return Array.from({length:v.size()},(_,i)=>v.get(i));}finally{v.delete();}};
  try {
    let t=performance.now();const status=s.solve_system(2);s.apply_solution();const cold_ms=performance.now()-t,values=read();
    t=performance.now();const dof=s.dof(),conflicting=ints(s.get_conflicting()),redundant=ints(s.get_redundant());const diagnostic_ms=performance.now()-t;
    const warm_ms=[],warm=[];
    if(f.warm_constraint!==undefined)for(let n=0;n<(f.samples??1);n++) {
      const ci=f.warm_constraint,value=f.constraints[ci].value+f.warm_step*(n+1);s.set_p_param(dimensions[ci],value,true);
      t=performance.now();const status=s.solve_system(2);s.apply_solution();warm_ms.push(performance.now()-t);warm.push({value,values:read(),status});
    }
    const drag=[];let released_dof=null;
    if(f.drag) {
      const x=s.push_p_param(0,true),y=s.push_p_param(0,true),p=own(s.make_point(indices[0][2],indices[0][3]));
      s.add_constraint_coordinate_x(p,x,-1,true,1);s.add_constraint_coordinate_y(p,y,-1,true,1);
      for(const cursor of f.drag) {
        s.set_p_param(x,cursor[0],true);s.set_p_param(y,cursor[1],true);t=performance.now();
        const status=s.solve_system(2);s.apply_solution();drag.push({cursor,ms:performance.now()-t,values:read(),status});
      }
      s.clear_by_id(-1);s.solve_system(2);released_dof=s.dof();
    }
    return {id:f.id,backend:'planegcs',status,values,dof,conflicting,redundant,build_ms,cold_ms,diagnostic_ms,warm_ms,warm,drag,released_dof,heap_bytes:mod.heapBytes()};
  } finally {s.clear_data();objects.reverse().forEach(o=>o.delete());s.delete();}
}
