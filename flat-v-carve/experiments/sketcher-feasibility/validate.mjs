// Independent geometric residuals in mm, not a solver success flag/cost.
export const tolerance=1e-6;
const dist=(a,b)=>Math.hypot(a[0]-b[0],a[1]-b[1]);
const len=l=>dist(l,l.slice(2));
const pointLine=(p,l)=>Math.abs((p[0]-l[0])*(l[3]-l[1])-(p[1]-l[1])*(l[2]-l[0]))/len(l);
export function residuals(f,values) {
  if(!values||values.flat().some(x=>x===null||!Number.isFinite(x)))return [{id:'finite',error:Infinity}];
  const r=f.constraints.map(c=>{
    const a=values[c.a],b=values[c.b??0],value=c.value??0;
    let error;
    switch(c.kind) {
      case 'horizontal':error=a[3]-a[1];break;
      case 'vertical':error=a[2]-a[0];break;
      case 'length':error=len(a)-value;break;
      case 'radius':error=a[2]-value;break;
      case 'join':error=dist(a.slice(2),b);break;
      case 'distance':error=dist(a,b)-value;break;
      case 'hdistance':error=a[0]-b[0]-value;break;
      case 'vdistance':error=a[1]-b[1]-value;break;
      case 'coincident':error=dist(a,b);break;
      case 'equal_length':error=len(a)-len(b);break;
      case 'equal_radius':error=a[2]-b[2];break;
      case 'tangent':error=pointLine(b,a)-b[2];break;
      case 'point_on_line':error=pointLine(a,b);break;
      case 'point_on_circle':error=dist(a,b)-b[2];break;
      case 'midpoint':error=dist(a,[(b[0]+b[2])/2,(b[1]+b[3])/2]);break;
      case 'parallel':error=((a[2]-a[0])*(b[3]-b[1])-(a[3]-a[1])*(b[2]-b[0]))/Math.max(len(a),len(b));break;
      case 'angle':{const delta=Math.atan2((a[2]-a[0])*(b[3]-b[1])-(a[3]-a[1])*(b[2]-b[0]),(a[2]-a[0])*(b[2]-b[0])+(a[3]-a[1])*(b[3]-b[1]))-value;error=Math.atan2(Math.sin(delta),Math.cos(delta))*Math.min(len(a),len(b));break;}
      case 'symmetry':{const l=values[c.c],dx=l[2]-l[0],dy=l[3]-l[1],q=((a[0]-l[0])*dx+(a[1]-l[1])*dy)/(dx*dx+dy*dy);error=dist(b,[2*(l[0]+q*dx)-a[0],2*(l[1]+q*dy)-a[1]]);break;}
      case 'perpendicular':error=((a[2]-a[0])*(b[2]-b[0])+(a[3]-a[1])*(b[3]-b[1]))/Math.max(len(a),len(b));break;
      default:throw Error(`Unvalidated constraint ${c.kind}`);
    }
    return {id:c.id,error:Math.abs(error)};
  });
  f.entities.forEach((e,i)=>{
    e.fixed?.forEach((fixed,j)=>{if(fixed)r.push({id:`${e.id}:fixed-${j}`,error:Math.abs(e.values[j]-values[i][j])});});
    if(e.kind==='line'&&len(values[i])<1e-9)r.push({id:`${e.id}:collapsed`,error:Infinity});
    if(['circle','arc'].includes(e.kind)&&values[i][2]<=0)r.push({id:`${e.id}:radius`,error:Infinity});
  });return r;
}
export function summarize(f,r) {
  const errors=residuals(f,r.values),max=Math.max(0,...errors.map(e=>e.error));
  const warmErrors=(r.warm??[]).map(w=>{
    const current=structuredClone(f);current.constraints[f.warm_constraint].value=w.value;
    return Math.max(0,...residuals(current,w.values).map(e=>e.error));
  });
  const times=[...(r.warm_ms??[])].sort((a,b)=>a-b);
  const dragErrors=(r.drag??[]).map(d=>Math.max(0,...residuals(f,d.values).map(e=>e.error)));
  const dragDirections=(r.drag??[]).map(d=>{const l=d.values[0],angle=Math.atan2(l[3]-l[1],l[2]-l[0]),target=Math.atan2(d.cursor[1],d.cursor[0]);return Math.abs(Math.atan2(Math.sin(angle-target),Math.cos(angle-target)));});
  const dragTimes=(r.drag??[]).map(d=>d.ms).sort((a,b)=>a-b);
  return {id:f.id,accepted:max<=tolerance,max_residual_mm:max,failed:errors.filter(e=>e.error>tolerance),warm_max_residual_mm:Math.max(0,...warmErrors),drag_max_residual_mm:Math.max(0,...dragErrors),drag_max_direction_error_rad:Math.max(0,...dragDirections),drag_p95_ms:dragTimes[Math.min(dragTimes.length-1,Math.ceil(dragTimes.length*.95)-1)]??null,released_dof:r.released_dof,dof:r.dof,expected_dof:f.expected_dof,dof_ok:f.expected_dof===undefined||f.expected_dof===r.dof,expected:f.expected,status:r.status,cold_ms:r.cold_ms,diagnostic_ms:r.diagnostic_ms,warm_p50_ms:times[Math.floor(times.length*.5)]??null,warm_p95_ms:times[Math.min(times.length-1,Math.ceil(times.length*.95)-1)]??null};
}
