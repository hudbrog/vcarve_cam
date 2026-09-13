// The simulated stock pass. One un-indexed draw: a flat quad per field cell, a
// wall quad per boundary cell, and the stock's bottom face. Colours come from
// the display style (`stock_style.rs`), never from the job.
//
// Camera and grid field orders mirror their Rust mirrors; `Style` mirrors
// `StockUniform` (the size is pinned by a Rust test).
struct Camera { yaw: f32, tilt: f32, zoom: f32, pan_x: f32, pan_y: f32, aspect: f32, _pad0: f32, _pad1: f32 }
struct Grid { origin: vec2<f32>, cell: f32, thickness: f32, cols: f32, rows: f32, width: f32, height: f32, tiles_x: f32, _pad: f32 }
struct Style {
    mode: u32,
    wall_mode: u32,
    appearance: u32,
    flags: u32,
    opacity: f32,
    wall_threshold: f32,
    ramp_top: f32,
    ramp_bottom: f32,
    light: vec4<f32>,
    to_camera: vec4<f32>,
    ramp_a: vec4<f32>,
    ramp_b: vec4<f32>,
    plain: vec4<f32>,
    plain_wall: vec4<f32>,
}
@group(0) @binding(0) var<uniform> camera: Camera;
@group(0) @binding(1) var<uniform> grid: Grid;
@group(0) @binding(2) var<storage, read> cells: array<u32>;
@group(0) @binding(3) var<uniform> style: Style;
@group(0) @binding(4) var<storage, read> palette: array<vec4<f32>>;
struct Output { @builtin(position) position: vec4<f32>, @location(0) color: vec4<f32> }

const PALETTE_STAGES: u32 = 256u;

// One packed cell: low 16 bits removed depth, then the stage index, then the
// tool index of the cutter that made this the current floor.
fn packed_at(col: u32, row: u32) -> u32 {
    let tile = (row/256u)*u32(grid.tiles_x)+(col/256u);
    return cells[tile*65536u+(row%256u)*256u+(col%256u)];
}
fn depth_of(packed: u32) -> f32 { return f32(packed&65535u)/65535.; }
fn stage_of(packed: u32) -> u32 { return (packed>>16u)&255u; }
fn tool_of(packed: u32) -> u32 { return (packed>>24u)&255u; }

// Floors keep the plain colour where no cutter has been, whatever the mode:
// an untouched cell has no operation to attribute it to.
fn floor_color(packed: u32, depth: f32) -> vec3<f32> {
    if (depth <= 0.) { return style.plain.rgb; }
    if (style.mode == 1u) { return palette[stage_of(packed)].rgb; }
    if (style.mode == 2u) { return palette[PALETTE_STAGES+tool_of(packed)].rgb; }
    if (style.mode == 3u) { return mix(style.ramp_a.rgb,style.ramp_b.rgb,clamp(depth,0.,1.)); }
    return style.plain.rgb;
}
// A wall's identity is the cell that removed the material beside it, so its
// own-operation colour is the cutter that created the face, not the surface
// above it. `height` is the fragment's fraction of the stock thickness.
fn wall_color(packed: u32, depth: f32, height: f32) -> vec3<f32> {
    if (style.wall_mode == 1u) { return palette[stage_of(packed)].rgb; }
    if (style.wall_mode == 2u) { return mix(style.ramp_a.rgb,style.ramp_b.rgb,clamp(height,0.,1.)); }
    if (style.mode == 0u) { return style.plain_wall.rgb; }
    return floor_color(packed,max(depth,1.));
}
// One directional key light plus ambient, both supplied by the camera so
// orbiting never turns a face black. Culling is off, so a face whose normal
// points away from the viewer is flipped: what the viewer sees is shaded.
fn shade(normal: vec3<f32>, color: vec3<f32>) -> vec3<f32> {
    var n = normalize(normal);
    if (dot(n,style.to_camera.xyz) < 0.) { n = -n; }
    let ambient = clamp(style.light.w,0.,1.);
    let direct = max(dot(n,normalize(style.light.xyz)),0.);
    return color*(ambient+(1.-ambient)*direct);
}
fn output_alpha() -> f32 {
    if (style.appearance == 1u) { return clamp(style.opacity,0.05,1.); }
    return 1.;
}

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
    var normal = vec3(0.,0.,1.);
    var color = style.plain_wall.rgb;
    if (id < cell_vertices) {
        let index = id/6u;
        let col = index%cols;
        let row = index/cols;
        let corners = array<vec2<f32>,6>(vec2(0.,0.),vec2(1.,0.),vec2(0.,1.),vec2(0.,1.),vec2(1.,0.),vec2(1.,1.));
        let xy = (vec2(f32(col),f32(row))+corners[id%6u])*grid.cell;
        let packed = packed_at(col,row);
        let depth = depth_of(packed);
        p = vec3(grid.origin+min(xy,vec2(grid.width,grid.height)), -depth*grid.thickness);
        // Surface normal from the field gradient, in stock millimetres, so a
        // V-bit slope reads as a slope and a step reads as an edge. The border
        // repeats its own cell instead of reading out of bounds.
        let border = min(grid.cell*0.5,1e-4);
        let step = max(2.*grid.cell,border);
        let dx = (neighbour_depth(col,row,1,0)-neighbour_depth(col,row,-1,0))*grid.thickness/step;
        let dy = (neighbour_depth(col,row,0,1)-neighbour_depth(col,row,0,-1))*grid.thickness/step;
        normal = vec3(dx,dy,1.);
        color = floor_color(packed,depth);
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
        let packed = packed_at(cell.x,cell.y);
        let depth = depth_of(packed);
        // From this cell's machined surface down to the stock bottom. A cell
        // cut through leaves no wall at all, so a through cut opens a real gap.
        // `along_extent` is the axis the wall runs along; the edge it stands on
        // is the *other* axis, so the two must not share an extent.
        let along_extent = select(grid.height,grid.width,along_x);
        let edge_extent = select(grid.width,grid.height,along_x);
        let along = min((select(f32(cell.y),f32(cell.x),along_x)+c.x)*grid.cell,along_extent);
        let fixed = high*edge_extent;
        let height = mix(depth,1.,c.y);
        p = vec3(
            grid.origin.x+select(fixed,along,along_x),
            grid.origin.y+select(along,fixed,along_x),
            mix(-depth*grid.thickness,-grid.thickness,c.y)
        );
        normal = select(vec3(0.,select(-1.,1.,high > 0.5),0.),vec3(select(-1.,1.,high > 0.5),0.,0.),along_x);
        color = wall_color(packed,depth,height);
    } else {
        // The bottom face keeps the full stock rectangle.
        let corner = array<vec3<f32>,8>(vec3(0.,0.,0.),vec3(1.,0.,0.),vec3(1.,1.,0.),vec3(0.,1.,0.),vec3(0.,0.,-1.),vec3(1.,0.,-1.),vec3(1.,1.,-1.),vec3(0.,1.,-1.));
        let indices = array<u32,6>(4u,5u,6u,4u,6u,7u);
        let c = corner[indices[id-cell_vertices-wall_vertices]];
        p = vec3(grid.origin+c.xy*vec2(grid.width,grid.height),c.z*grid.thickness);
        normal = vec3(0.,0.,-1.);
    }
    let a = camera.yaw;
    let x = p.x*cos(a)-p.y*sin(a);
    let y = p.x*sin(a)+p.y*cos(a);
    let t = camera.tilt;
    let yy = y*cos(t)+p.z*sin(t);
    let depth = clamp(0.5-(p.z*cos(t)-y*sin(t))*0.2,0.01,0.99);
    var out: Output;
    out.position = vec4((x+camera.pan_x)*camera.zoom/camera.aspect,(yy+camera.pan_y)*camera.zoom,depth,1.);
    out.color = vec4(shade(normal,color),output_alpha());
    return out;
}
// Neighbouring cell depth for the surface normal; the border repeats itself.
fn neighbour_depth(col: u32, row: u32, dx: i32, dy: i32) -> f32 {
    let c = u32(clamp(i32(col)+dx,0,i32(grid.cols)-1));
    let r = u32(clamp(i32(row)+dy,0,i32(grid.rows)-1));
    return depth_of(packed_at(c,r));
}
@fragment fn fs_stock(in: Output) -> @location(0) vec4<f32> {return in.color;}
