// Field order mirrors `camera::Camera::uniform` (two trailing pad slots keep
// the uniform's 16-byte alignment).
struct Camera { yaw: f32, tilt: f32, zoom: f32, pan_x: f32, pan_y: f32, aspect: f32, _pad0: f32, _pad1: f32 }
@group(0) @binding(0) var<uniform> camera: Camera;
struct Output { @builtin(position) position: vec4<f32>, @location(0) color: vec4<f32> }
@vertex fn vs_main(@location(0) p: vec3<f32>, @location(1) color: vec4<f32>) -> Output {
    let a = camera.yaw;
    let x = p.x*cos(a)-p.y*sin(a);
    let y = p.x*sin(a)+p.y*cos(a);
    let t = camera.tilt;
    let yy = y*cos(t)+p.z*sin(t);
    let depth = clamp(0.5-(p.z*cos(t)-y*sin(t))*0.2, 0.01, 0.99);
    var out: Output;
    out.position = vec4((x+camera.pan_x)*camera.zoom/camera.aspect, (yy+camera.pan_y)*camera.zoom, depth, 1.0);
    out.color = color;
    return out;
}
@fragment fn fs_main(in: Output) -> @location(0) vec4<f32> { return in.color; }
