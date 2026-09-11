import {inflateSync} from 'node:zlib';

// Decode Chrome's lossless screenshot to check actual canvas pixels, not only
// worker metadata. This caught source contours that were never uploaded to GPU.
export function selectedArtworkPixels(png,rect) {
  let width,height,channels;const chunks=[];
  for(let offset=8;offset<png.length;) {
    const length=png.readUInt32BE(offset),type=png.toString('ascii',offset+4,offset+8);
    const data=png.subarray(offset+8,offset+8+length);
    if(type==='IHDR') {
      width=data.readUInt32BE(0);height=data.readUInt32BE(4);
      if(data[8]!==8 || ![2,6].includes(data[9]) || data[12]!==0)throw new Error('Unsupported screenshot PNG');
      channels=data[9]===6?4:3;
    }
    if(type==='IDAT')chunks.push(data);
    offset+=length+12;
  }
  const raw=inflateSync(Buffer.concat(chunks)),stride=width*channels;
  const pixels=Buffer.alloc(height*stride);
  const paeth=(a,b,c)=>{const p=a+b-c,pa=Math.abs(p-a),pb=Math.abs(p-b),pc=Math.abs(p-c);return pa<=pb&&pa<=pc?a:pb<=pc?b:c;};
  for(let y=0;y<height;y++) {
    const filter=raw[y*(stride+1)];
    for(let x=0;x<stride;x++) {
      const at=y*stride+x,a=x>=channels?pixels[at-channels]:0,b=y?pixels[at-stride]:0,c=y&&x>=channels?pixels[at-stride-channels]:0;
      const prediction=[0,a,b,Math.floor((a+b)/2),paeth(a,b,c)][filter];
      if(prediction===undefined)throw new Error('Invalid PNG row filter');
      pixels[at]=(raw[y*(stride+1)+1+x]+prediction)&255;
    }
  }
  let count=0;
  for(let y=Math.max(0,Math.ceil(rect[1])+5);y<Math.min(height,rect[3]-5);y++)for(let x=Math.max(0,Math.ceil(rect[0])+5);x<Math.min(width,rect[2]-5);x++) {
    const at=(y*width+x)*channels,[r,g,b]=pixels.subarray(at,at+3);
    if(g>150 && g>r*1.2 && b>r*1.2)count++;
  }
  if(count<20)throw new Error(`Selected source artwork is not visible: ${count} cyan pixels in the viewport`);
  return count;
}
