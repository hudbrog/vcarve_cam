// Field order mirrors `camera::Camera::uniform` (two trailing pad slots keep
// the uniform's 16-byte alignment).
struct Camera { yaw: f32, tilt: f32, zoom: f32, pan_x: f32, pan_y: f32, aspect: f32, _pad0: f32, _pad1: f32 }
struct Grid { origin: vec2<f32>, cell: f32, thickness: f32, cols: f32, rows: f32, width: f32, height: f32, tiles_x: f32, _pad: f32 }
@group(0) @binding(0) var<uniform> camera: Camera;
@group(0) @binding(1) var<uniform> grid: Grid;
@group(0) @binding(2) var<storage, read> cells: array<u32>;
struct Output { @builtin(position) position: vec4<f32>, @location(0) color: vec4<f32> }
@vertex fn vs_stock(@builtin(vertex_index) id: u32) -> Output {
    let cols = u32(grid.cols);
    let rows = u32(grid.rows);
    let count = cols*rows;
    let cell_vertices = count*6u;
    // One wall quad per boundary cell: the four edges, corner cells carrying
    // two of them. The wall follows the material left in its own cell, so the
    // sides show the remaining stock instead of the original envelope.
    let wall_vertices = (2u*cols+2u*rows)*6u;
    var p: vec3<f32>;
    var color = vec3(0.34,0.29,0.22);
    if (id < cell_vertices) {
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
    } else if (id < cell_vertices+wall_vertices) {
        let wall = (id-cell_vertices)/6u;
        let corners = array<vec2<f32>,6>(vec2(0.,0.),vec2(1.,0.),vec2(0.,1.),vec2(0.,1.),vec2(1.,0.),vec2(1.,1.));
        let c = corners[(id-cell_vertices)%6u];
        // Walk the perimeter: the low-Y edge, the low-X edge, the high-Y edge,
        // then the high-X edge.
        var cell: vec2<u32>;
        var along_x: bool;
        var high: f32;
        if (wall < cols) {
            cell = vec2<u32>(wall,0u);
            along_x = true;
            high = 0.;
        } else if (wall < cols+rows) {
            cell = vec2<u32>(0u,wall-cols);
            along_x = false;
            high = 0.;
        } else if (wall < 2u*cols+rows) {
            cell = vec2<u32>(wall-(cols+rows),rows-1u);
            along_x = true;
            high = 1.;
        } else {
            cell = vec2<u32>(cols-1u,wall-(2u*cols+rows));
            along_x = false;
            high = 1.;
        }
        let tile = (cell.y/256u)*u32(grid.tiles_x)+(cell.x/256u);
        let packed = cells[tile*65536u+(cell.y%256u)*256u+(cell.x%256u)];
        let depth = f32(packed&65535u)/65535.;
        // From this cell's machined surface down to the stock bottom. A cell
        // cut through leaves no wall at all, so a through cut opens a real gap.
        // `along_extent` is the axis the wall runs along; the edge it stands on
        // is the *other* axis, so the two must not share an extent.
        let along_extent = select(grid.height,grid.width,along_x);
        let edge_extent = select(grid.width,grid.height,along_x);
        let along = min((select(f32(cell.y),f32(cell.x),along_x)+c.x)*grid.cell,along_extent);
        let fixed = high*edge_extent;
        p = vec3(
            grid.origin.x+select(fixed,along,along_x),
            grid.origin.y+select(along,fixed,along_x),
            mix(-depth*grid.thickness,-grid.thickness,c.y)
        );
    } else {
        // The bottom face keeps the full stock rectangle.
        let corner = array<vec3<f32>,8>(vec3(0.,0.,0.),vec3(1.,0.,0.),vec3(1.,1.,0.),vec3(0.,1.,0.),vec3(0.,0.,-1.),vec3(1.,0.,-1.),vec3(1.,1.,-1.),vec3(0.,1.,-1.));
        let indices = array<u32,6>(4u,5u,6u,4u,6u,7u);
        let c = corner[indices[id-cell_vertices-wall_vertices]];
        p = vec3(grid.origin+c.xy*vec2(grid.width,grid.height),c.z*grid.thickness);
    }
    let a = camera.yaw;
    let x = p.x*cos(a)-p.y*sin(a);
    let y = p.x*sin(a)+p.y*cos(a);
    let t = camera.tilt;
    let yy = y*cos(t)+p.z*sin(t);
    let depth = clamp(0.5-(p.z*cos(t)-y*sin(t))*0.2,0.01,0.99);
    var out: Output;
    out.position = vec4((x+camera.pan_x)*camera.zoom/camera.aspect,(yy+camera.pan_y)*camera.zoom,depth,1.);
    out.color = vec4(color,1.);
    return out;
}
@fragment fn fs_stock(in: Output) -> @location(0) vec4<f32> {return in.color;}
