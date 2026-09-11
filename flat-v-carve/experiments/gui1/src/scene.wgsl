struct Camera { iso: f32, aspect: f32, zoom: f32, yaw: f32 }
@group(0) @binding(0) var<uniform> camera: Camera;
struct Output { @builtin(position) position: vec4<f32>, @location(0) color: vec4<f32> }
@vertex fn vs_main(@location(0) p: vec3<f32>, @location(1) color: vec4<f32>) -> Output {
    let a = camera.yaw;
    let x = p.x*cos(a)-p.y*sin(a);
    let y = p.x*sin(a)+p.y*cos(a);
    let yy = mix(y, y*0.65+p.z*0.76, camera.iso);
    let depth = clamp(0.5-mix(p.z, p.z*0.65-y*0.76, camera.iso)*0.2, 0.01, 0.99);
    var out: Output;
    out.position = vec4(x*camera.zoom/camera.aspect, yy*camera.zoom, depth, 1.0);
    out.color = color;
    return out;
}
@fragment fn fs_main(in: Output) -> @location(0) vec4<f32> { return in.color; }
