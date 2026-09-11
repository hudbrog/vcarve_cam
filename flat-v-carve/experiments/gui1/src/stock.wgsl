struct Camera { iso: f32, aspect: f32, zoom: f32, yaw: f32 }
struct Grid { origin: vec2<f32>, cell: f32, thickness: f32, cols: f32, rows: f32, width: f32, height: f32, tiles_x: f32, _pad: f32 }
@group(0) @binding(0) var<uniform> camera: Camera;
@group(0) @binding(1) var<uniform> grid: Grid;
@group(0) @binding(2) var<storage, read> cells: array<u32>;
struct Output { @builtin(position) position: vec4<f32>, @location(0) color: vec4<f32> }
@vertex fn vs_stock(@builtin(vertex_index) id: u32) -> Output {
    let count = u32(grid.cols)*u32(grid.rows);
    var p: vec3<f32>;
    var color = vec3(0.34,0.29,0.22);
    if (id < count*6u) {
        let index = id/6u;
        let corners = array<vec2<f32>,6>(vec2(0.,0.),vec2(1.,0.),vec2(0.,1.),vec2(0.,1.),vec2(1.,0.),vec2(1.,1.));
        let xy = (vec2(f32(index%u32(grid.cols)),f32(index/u32(grid.cols)))+corners[id%6u])*grid.cell;
        // Cells are stored tile-major: one contiguous 256x256 block per tile, so
        // the display process can refresh a dirty tile with a single copy.
        let col = index%u32(grid.cols);
        let row = index/u32(grid.cols);
        let tile = (row/256u)*u32(grid.tiles_x)+(col/256u);
        let packed = cells[tile*65536u+(row%256u)*256u+(col%256u)];
        let depth = f32(packed&65535u)/65535.;
        let role = (packed>>16u)&255u;
        p = vec3(grid.origin+min(xy,vec2(grid.width,grid.height)), -depth*grid.thickness);
        color = vec3(0.60,0.53,0.41);
        if (role == 1u) { color = vec3(0.13,0.48,0.54); }
        if (role == 2u) { color = vec3(0.77,0.42,0.12); }
        color *= 1.-min(depth*4.,0.5);
    } else {
        // Bottom and four sides retain the physical stock thickness.
        let corner = array<vec3<f32>,8>(vec3(0.,0.,0.),vec3(1.,0.,0.),vec3(1.,1.,0.),vec3(0.,1.,0.),vec3(0.,0.,-1.),vec3(1.,0.,-1.),vec3(1.,1.,-1.),vec3(0.,1.,-1.));
        let indices = array<u32,30>(4u,5u,6u,4u,6u,7u,0u,1u,5u,0u,5u,4u,1u,2u,6u,1u,6u,5u,2u,3u,7u,2u,7u,6u,3u,0u,4u,3u,4u,7u);
        let c = corner[indices[id-count*6u]];
        p = vec3(grid.origin+c.xy*vec2(grid.width,grid.height),c.z*grid.thickness);
    }
    let a = camera.yaw;
    let x = p.x*cos(a)-p.y*sin(a);
    let y = p.x*sin(a)+p.y*cos(a);
    let yy = mix(y,y*0.65+p.z*0.76,camera.iso);
    let depth = clamp(0.5-mix(p.z,p.z*0.65-y*0.76,camera.iso)*0.2,0.01,0.99);
    var out: Output;
    out.position = vec4(x*camera.zoom/camera.aspect,yy*camera.zoom,depth,1.);
    out.color = vec4(color,1.);
    return out;
}
@fragment fn fs_stock(in: Output) -> @location(0) vec4<f32> {return in.color;}
