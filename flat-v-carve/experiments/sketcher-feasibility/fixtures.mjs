// Same dimensional systems and initial values for every backend/platform.
export function fixtures() {
  const entity=(id,kind,values,fixed=[])=>({id,kind,values,fixed});
  const constraint=(kind,a,b=0,value=0,id=`${kind}-${a}-${b}`)=>({id,kind,a,b,value});
  function rectangles(count,id) {
    const entities=[],constraints=[];
    for(let i=0;i<count;i++) {
      const x=i*60,n=entities.length;
      entities.push(entity(`bottom-${i}`,'line',[x,0,x+39,0.3],[true,true]),entity(`right-${i}`,'line',[x+40,.1,x+40.2,29.5]),entity(`top-${i}`,'line',[x+40,30,x+.2,30.4]),entity(`left-${i}`,'line',[x,30.1,x-.1,0]),entity(`hole-${i}`,'circle',[x+20,15,3.9],[true,true]));
      for(let j=0;j<4;j++)constraints.push(constraint('join',n+j,n+(j+1)%4));
      constraints.push(constraint('horizontal',n),constraint('vertical',n+1),constraint('horizontal',n+2),constraint('vertical',n+3),constraint('length',n,0,40),constraint('length',n+1,0,30),constraint('radius',n+4,0,4));
    }
    return {id,entities,constraints,warm_constraint:8,warm_step:.02,samples:21,expected:'valid',expected_dof:0};
  }
  const result=[rectangles(1,'rectangle'),rectangles(5,'scale-25'),rectangles(20,'scale-100'),rectangles(100,'scale-500')];
  const add=(id,entities,constraints,expected='valid',extra={})=>result.push({id,entities,constraints,expected,...extra});
  for(const count of [25,100]) {
    const entities=[],constraints=[];let x=0,y=0;
    for(let i=0;i<count;i++) {
      const nx=x+(i%2===0?10:0),ny=y+(i%2===1?10:0);
      entities.push(entity(`edge-${i}`,'line',[x,y,nx+.05,ny+.05],i===0?[true,true]:[]));
      constraints.push(constraint(i%2===0?'horizontal':'vertical',i),constraint('length',i,0,10));
      if(i>0)constraints.push(constraint('join',i-1,i));x=nx;y=ny;
    }
    add(`connected-${count}`,entities,constraints,'valid',{warm_constraint:1,warm_step:.02,samples:21,expected_dof:0});
  }
  add('drag-length',[entity('line','line',[0,0,10,0],[true,true])],[constraint('length',0,0,10)],'valid',{expected_dof:1,drag:Array.from({length:21},(_,i)=>[12*Math.cos((i+1)*.06),12*Math.sin((i+1)*.06)])});
  add('underconstrained',[entity('line','line',[0,0,8,2])],[constraint('length',0,0,10)],'valid',{expected_dof:3});
  add('angle',[entity('base','line',[0,0,10,0],[true,true,true,true]),entity('leg','line',[0,0,8,6],[true,true])],[constraint('length',1,0,10),constraint('angle',0,1,Math.PI/3)],'valid',{expected_dof:0});
  add('symmetry',[entity('a','point',[4,7],[true,true]),entity('b','point',[-3,6]),entity('axis','line',[0,-10,0,10],[true,true,true,true])],[{...constraint('symmetry',0,1),c:2}],'valid',{expected_dof:0});
  add('equal-holes',[entity('a','circle',[10,10,3.5],[true,true]),entity('b','circle',[30,10,4.8],[true,true])],[constraint('radius',0,0,5),constraint('equal_radius',0,1)],'valid',{expected_dof:0});
  add('tangent-arc',[entity('line','line',[10,0,-10,0],[true,true,true,true]),entity('arc','arc',[0,5,4.7,Math.PI,2*Math.PI],[true,true,false,true,true])],[constraint('tangent',0,1)],'valid',{expected_dof:0});
  const points=[entity('origin','point',[0,0],[true,true]),entity('p','point',[9,2])];
  add('redundant',points,[constraint('distance',0,1,10,'distance-a'),constraint('distance',0,1,10,'distance-b')],'valid',{expected_dof:1,diagnostic:'redundant'});
  add('conflicting',points,[constraint('distance',0,1,10,'distance-a'),constraint('distance',0,1,20,'distance-b')],'invalid',{diagnostic:'conflicting'});
  add('point-dimensions',points,[constraint('hdistance',1,0,10),constraint('vdistance',1,0,20)],'valid',{expected_dof:0});
  add('point-on-line',[entity('p','point',[5,2]),entity('l','line',[0,0,10,0],[true,true,true,true])],[constraint('point_on_line',0,1)],'valid',{expected_dof:1});
  add('midpoint',[entity('p','point',[5,2]),entity('l','line',[0,0,10,0],[true,true,true,true])],[constraint('midpoint',0,1)],'valid',{expected_dof:0});
  add('point-on-circle',[entity('p','point',[4,4]),entity('c','circle',[0,0,5],[true,true,true])],[constraint('point_on_circle',0,1)],'valid',{expected_dof:1});
  for(const kind of ['parallel','perpendicular','equal_length'])add(kind,[entity('a','line',[0,0,10,0],[true,true,true,true]),entity('b','line',[0,5,kind==='perpendicular'?.1:9,kind==='perpendicular'?14:5.2],[true,true])],[constraint(kind,0,1),...(kind==='equal_length'?[]:[constraint('length',1,0,10)])],'valid',{expected_dof:kind==='equal_length'?1:0});
  add('coincident',[entity('a','point',[0,0],[true,true]),entity('b','point',[2,3])],[constraint('coincident',0,1)],'valid',{expected_dof:0});
  result.push({...structuredClone(result.at(-1)),id:'coincident-default-drift',drift_isigma:1e-3,skip_polish:true,expected:'characterization'});
  add('degenerate-line',[entity('line','line',[0,0,0,0])],[constraint('length',0,0,10),constraint('horizontal',0)],'diagnostic');
  for(const scale of [.001,1000]) {
    const f=rectangles(1,`units-${scale}`);
    for(const e of f.entities)e.values=e.values.map(v=>v*scale);
    for(const c of f.constraints)c.value*=scale;
    f.warm_step*=scale;f.samples=5;result.push(f);
  }
  const poor=rectangles(1,'difficult-initial');poor.entities[1].values=[25,5,35,40];poor.samples=5;result.push(poor);
  // Deletion probes expose a bounded diagnostic strategy without claiming a
  // minimal conflict set or a proof that all nonconvergence is inconsistency.
  for(const f of result.filter(f=>f.diagnostic))for(let i=0;i<f.constraints.length;i++)result.push({...structuredClone(f),id:`${f.id}-remove-${i}`,constraints:f.constraints.filter((_,j)=>i!==j),expected:'valid',expected_dof:1,diagnostic:undefined});
  return result;
}
