// Production SketchDocument fixtures shared by native and browser measurements.
// Every four-edge wire has seven driving constraints and one reference dimension.
export function rectangles(entityCount) {
  if(entityCount%4)throw Error('Entity count must be a multiple of four');
  const model={sketch_version:1,frame:{min:{x:0,y:0},width_mm:400,height_mm:300},entities:[],constraints:[],wires:[]};
  const p=(x,y)=>({x,y});
  for(let i=0;i<entityCount/4;i++) {
    const x=4+(i%10)*36,y=4+Math.floor(i/10)*22;
    const corners=[p(x,y),p(x+24,y),p(x+24,y+14),p(x,y+14)];
    const ids=corners.map((_,j)=>`r${i}-e${j}`);
    corners.forEach((start,j)=>model.entities.push({id:ids[j],construction:false,geometry:{kind:'line',start,end:corners[(j+1)%4]}}));
    model.wires.push({id:`r${i}`,closed:true,edges:ids.map(entity_id=>({entity_id,reversed:false}))});
    const add=(suffix,definition,reference=false)=>model.constraints.push({id:`r${i}-${suffix}`,reference,definition});
    ids.forEach((line,j)=>add(`axis${j}`,{kind:j%2?'vertical':'horizontal',line}));
    add('width',{kind:'length',line:ids[0],value_mm:24});
    add('height',{kind:'length',line:ids[1],value_mm:14});
    add('origin',{kind:'fixed_point',point:{entity_id:ids[0],point:'start'},position:corners[0]});
    add('reference',{kind:'length',line:ids[2],value_mm:24},true);
  }
  return model;
}

export function performanceFixtures() {
  return [24,100,500].map(count=>({name:`rectangles-${count}`,model:rectangles(count)}));
}
export function resize(model,step) {
  const copy=structuredClone(model);
  copy.constraints.find(c=>c.id==='r0-width').definition.value_mm=24+step*.02;
  return copy;
}
export function percentile(values,p) {
  const sorted=[...values].sort((a,b)=>a-b);
  return sorted[Math.min(sorted.length-1,Math.ceil(p*sorted.length)-1)];
}
