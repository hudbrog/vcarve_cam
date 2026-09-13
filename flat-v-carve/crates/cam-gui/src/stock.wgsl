// The simulated stock pass. One un-indexed draw: a flat quad per field cell, a
// wall quad per boundary cell, and the stock's bottom face. Colours come from
// the display style (`stock_style.rs`), never from the job.
//
// Camera and grid field orders mirror their Rust mirrors; `Style` mirrors
// `StockUniform` (the size is pinned by a Rust test).
struct Camera { yaw: f32, tilt: f32, zoom: f32, pan_x: f32, pan_y: f32, aspect: f32, _pad0: f32, _pad1: f32 }
struct Grid { origin: vec2<f32>, cell: f32, thickness: f32, cols: f32, rows: f32, width: f32, height: f32, tiles_x: f32, walls: f32 }
// One interior or edge wall, in grid-cell units: axis 0 stands at a fixed x
// and runs along y, axis 1 at a fixed y and runs along x.
struct Wall { start: vec2<f32>, length: f32, axis: u32, top: f32, bottom: f32, identity: u32, _pad: f32 }
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
@group(0) @binding(5) var<storage, read> walls: array<Wall>;
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
fn surface_color(stage: u32, tool: u32, depth: f32) -> vec3<f32> {
    if (depth <= 0.) { return style.plain.rgb; }
    if (style.mode == 1u) { return palette[stage].rgb; }
    if (style.mode == 2u) { return palette[PALETTE_STAGES+tool].rgb; }
    if (style.mode == 3u) { return mix(style.ramp_a.rgb,style.ramp_b.rgb,clamp(depth,0.,1.)); }
    return style.plain.rgb;
}
// A wall's identity is the cell that removed the material beside it, so its
// own-operation colour is the cutter that created the face, not the surface
// above it. `identity` is `stage | tool << 8`, or "no cutter" for material a
// tool never touched (the stock's own untouched edge). `height` is the
// fragment's fraction of the stock thickness.
fn wall_color(identity: u32, height: f32) -> vec3<f32> {
    if (identity == 0xffffffffu) { return style.plain_wall.rgb; }
    let stage = identity&255u;
    let tool = (identity>>8u)&255u;
    if (style.wall_mode == 1u) { return palette[stage].rgb; }
    if (style.wall_mode == 2u) { return mix(style.ramp_a.rgb,style.ramp_b.rgb,clamp(height,0.,1.)); }
    if (style.mode == 0u) { return style.plain_wall.rgb; }
    // A wall always has material beside it, so the surface modes colour it
    // exactly as they colour the floor it belongs to.
    return surface_color(stage,tool,1.);
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
    // One quad per wall instance the display built: interior steps and the
    // stock's own edges, both from the same detector (`stock_walls.rs`).
    let wall_vertices = u32(grid.walls)*6u;
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
        color = surface_color(stage_of(packed),tool_of(packed),depth);
    } else if (id < cell_vertices+wall_vertices) {
        let wall = (id-cell_vertices)/6u;
        let corners = array<vec2<f32>,6>(vec2(0.,0.),vec2(1.,0.),vec2(0.,1.),vec2(0.,1.),vec2(1.,0.),vec2(1.,1.));
        let c = corners[(id-cell_vertices)%6u];
        let instance = walls[wall];
        // Axis 0 stands at a fixed x and runs along y; axis 1 the other way.
        let along = instance.start+c.x*instance.length*select(vec2(0.,1.),vec2(1.,0.),instance.axis == 1u);
        let height = mix(instance.top,instance.bottom,c.y);
        p = vec3(grid.origin+along*grid.cell,-height*grid.thickness);
        // The normal is the wall's own plane; the shader flips it toward the
        // viewer below, so an edge seen from either side still shades.
        normal = select(vec3(0.,1.,0.),vec3(1.,0.,0.),instance.axis == 0u);
        color = wall_color(instance.identity,height);
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
    // In a true elevation the cell quads are edge-on: they would draw as a
    // dense wireframe inside the section, so the pass drops them and keeps the
    // section quads and the bottom face.
    if ((style.flags&2u) != 0u && id < cell_vertices) {
        out.position = vec4(2.,2.,2.,1.);
    }
    return out;
}
// Neighbouring cell depth for the surface normal; the border repeats itself.
fn neighbour_depth(col: u32, row: u32, dx: i32, dy: i32) -> f32 {
    let c = u32(clamp(i32(col)+dx,0,i32(grid.cols)-1));
    let r = u32(clamp(i32(row)+dy,0,i32(grid.rows)-1));
    return depth_of(packed_at(c,r));
}
@fragment fn fs_stock(in: Output) -> @location(0) vec4<f32> {return in.color;}
