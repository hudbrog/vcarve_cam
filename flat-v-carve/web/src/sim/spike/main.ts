// U8 phase-1 experiment 3: browser rendering feasibility spike (dev-only).
// Served by `vite dev` at /spike.html; never imported by the app and not part
// of the production bundle. It checks the three.js rendering assumptions the
// plan depends on and reports every measurement on window.__spike.
import * as THREE from 'three';
import { OrbitControls } from 'three/addons/controls/OrbitControls.js';
import { applyMotion, beginEpoch, createField, normalizeTool } from '../engine';
import type { Resolution, SimMotion } from '../engine';

interface SpikeResults {
  started: string;
  done: boolean;
  webgl2?: boolean;
  maxVertexTextureImageUnits?: number;
  renderer?: string;
  scenario?: { motions: number; modelSeconds: number; grid: string; texture: string };
  initialUpload?: { texels: number; ms: number };
  probe?: { samples: number; maxByteError: number; pass: boolean };
  regionUpload?: { uploads: number; msPerUpload: number; verifyByteError: number; pass: boolean };
  vertexFetch?: { cpu: number; vertexByte: number; fragmentByte: number; pass: boolean };
  playback?: {
    motions: number; modelSeconds: number; wallSeconds: number; speed: number;
    sustained: number; frames: number; avgChangedTiles: number; avgUploadMs: number; pass: boolean;
  };
  fps?: { frames: number; seconds: number; fps: number; triangles: number; pass: boolean };
  cliffSkirts?: { quads: number; thresholdMm: number };
  errors?: string[];
}

declare global {
  interface Window {
    __spike?: SpikeResults;
    __probeDebug?: {
      probe: (x: number, y: number, expect: (px: number, py: number) => number) => number;
      copyRegion: (x: number, y: number, width: number, height: number) => void;
      pixels: Uint8Array;
      field: Uint16Array;
      half: Uint16Array;
      cols: number;
      rows: number;
      renderer: THREE.WebGLRenderer;
      heightsTex: THREE.DataTexture;
    };
  }
}

const results: SpikeResults = { started: new Date().toISOString(), done: false, errors: [] };
window.__spike = results;
const hud = document.getElementById('hud')!;
const renderHud = () => {
  hud.textContent = `U8 spike ${results.done ? '(done)' : '(running)'}\n`
    + JSON.stringify({ ...results, started: undefined, done: undefined, errors: undefined }, null, 1)
    + (results.errors?.length ? `\nerrors: ${results.errors.join(' | ')}` : '');
};
const fail = (message: string) => { results.errors!.push(message); console.error(message); };

// --- Scenario: pocket + V-bit flower on a 400x300x12 sheet -----------------
const WIDTH = 400;
const HEIGHT = 300;
const THICKNESS = 12;
const sheet = { x0: 0, y0: 0, x1: WIDTH, y1: HEIGHT, thicknessMm: THICKNESS };
const resolution: Resolution = { cellMm: WIDTH / 4096, cappedByTexels: false, cappedByBudget: false };
const endmill = normalizeTool({ kind: 'endmill', diameterMm: 6 });
const vbit = normalizeTool({ kind: 'vbit', includedAngleDeg: 60, tipDiameterMm: 0.2, maxCuttingDiameterMm: 3.175, cuttingHeightMm: 2.6 });
const field = createField(sheet, [endmill, vbit], resolution);
const cols = field.cols;
const rows = field.rows;

const ENDMILL_FEED = 1800;
const VBIT_FEED = 1000;
const motions: SimMotion[] = [];
const cut = (tool: number, x0: number, y0: number, z0: number, x1: number, y1: number, z1: number, kind: SimMotion['kind'] = 'cut') =>
  motions.push({ kind, tool, x0, y0, z0, x1, y1, z1 });

// Endmill pocket 120x80 centered, two stepdowns, 2.4 mm stepover.
for (let layer = 0; layer < 2; layer++) {
  const z = -1 - layer;
  for (let pass = 0; pass * 2.4 <= 76; pass++) {
    const y = 112 + pass * 2.4;
    const forward = (pass + layer) % 2 === 0;
    cut(0, forward ? 142 : 258, y, z, forward ? 258 : 142, y, z);
  }
}
// V-bit flower: polar roses plus an outer ring, depth following the radius.
const flower = (petals: number, base: number, wobble: number, depthBase: number, depthWobble: number) => {
  const steps = 360;
  let previous: { x: number; y: number; z: number } | null = null;
  for (let index = 0; index <= steps; index++) {
    const theta = (index / steps) * Math.PI * 2;
    const radius = base + wobble * Math.cos(petals * theta);
    const x = 200 + radius * Math.cos(theta);
    const y = 150 + radius * Math.sin(theta);
    const z = -(depthBase + depthWobble * (0.5 + 0.5 * Math.cos(petals * theta)));
    if (previous) cut(1, previous.x, previous.y, previous.z, x, y, z, Math.abs(previous.z - z) > 0.2 ? 'ramp' : 'cut');
    previous = { x, y, z };
  }
};
flower(5, 46, 16, 1.0, 1.2);
flower(8, 24, 6, 0.7, 0.5);
flower(1, 88, 0, 1.3, 0);
const modelSeconds = motions.reduce((sum, m) =>
  sum + Math.hypot(m.x1 - m.x0, m.y1 - m.y0) / (m.tool === 0 ? ENDMILL_FEED : VBIT_FEED) * 60, 0);
results.scenario = {
  motions: motions.length,
  modelSeconds: +modelSeconds.toFixed(1),
  grid: '2561x1921 vertices',
  texture: `${cols}x${rows} R16+R8`,
};

// --- Engine field mirrored into full CPU arrays -----------------------------
// The engine stores quantized Uint16 levels; the GPU height texture uses
// half-float bits because vertex-stage sampling of normalized R16 returned
// zero on the tested ANGLE/D3D11 stack (see results.vertexFetch history).
const heightsField = new Uint16Array(cols * rows);
const heightsHalf = new Uint16Array(cols * rows);
const ownersField = new Uint8Array(cols * rows);
const blitTile = (tile: number) => {
  const heights = field.heights[tile];
  const owners = field.cellOwner[tile];
  if (heights === undefined || owners === undefined) return;
  const tileCol = (tile % field.tilesX) * 256;
  const tileRow = Math.floor(tile / field.tilesX) * 256;
  const width = Math.min(256, cols - tileCol);
  const height = Math.min(256, rows - tileRow);
  for (let row = 0; row < height; row++) {
    const source = row << 8;
    const destination = (tileRow + row) * cols + tileCol;
    heightsField.set(heights.subarray(source, source + width), destination);
    for (let index = 0; index < width; index++) {
      heightsHalf[destination + index] = THREE.DataUtils.toHalfFloat(heights[source + index] / 65535);
    }
    ownersField.set(owners.subarray(source, source + width), destination);
  }
};

// --- three.js setup ----------------------------------------------------------
const canvas = document.createElement('canvas');
document.body.appendChild(canvas);
const renderer = new THREE.WebGLRenderer({ canvas, antialias: true });
renderer.setPixelRatio(Math.min(window.devicePixelRatio, 2));
renderer.setSize(window.innerWidth, window.innerHeight);
const gl = renderer.getContext();
results.webgl2 = renderer.capabilities.isWebGL2;
results.maxVertexTextureImageUnits = gl.getParameter(gl.MAX_VERTEX_TEXTURE_IMAGE_UNITS);
const debugInfo = gl.getExtension('WEBGL_debug_renderer_info');
results.renderer = debugInfo ? String(gl.getParameter(debugInfo.UNMASKED_RENDERER_WEBGL)) : 'unavailable';

const Z_SCALE = 3;
const scene = new THREE.Scene();
scene.background = new THREE.Color(0x14161a);
const camera = new THREE.PerspectiveCamera(35, window.innerWidth / window.innerHeight, 1, 4000);
camera.position.set(270, 230, 330);
const controls = new OrbitControls(camera, canvas);
controls.enableDamping = true;
controls.autoRotateSpeed = 1.2;
scene.add(new THREE.HemisphereLight(0xdfe6ee, 0x2c3038, 1.1));
const sun = new THREE.DirectionalLight(0xffffff, 1.6);
sun.position.set(180, 320, 140);
scene.add(sun);

// CPU source textures wrap the live arrays; the GPU twins receive full and
// region uploads. The CPU twins are never rendered with, so three keeps them
// client-side and copyTextureToTexture takes its texSubImage2D path.
const heightsCpu = new THREE.DataTexture(heightsHalf, cols, rows, THREE.RedFormat, THREE.HalfFloatType);
heightsCpu.flipY = false;
const ownersCpu = new THREE.DataTexture(ownersField, cols, rows, THREE.RedFormat, THREE.UnsignedByteType);
ownersCpu.flipY = false;
const heightsTex = heightsCpu.clone();
heightsTex.minFilter = THREE.LinearFilter;
heightsTex.magFilter = THREE.LinearFilter;
heightsTex.flipY = false;
const ownersTex = ownersCpu.clone();
ownersTex.minFilter = THREE.NearestFilter;
ownersTex.magFilter = THREE.NearestFilter;
ownersTex.flipY = false;

const surfaceUniforms = {
  uHeights: { value: heightsTex },
  uOwners: { value: ownersTex },
  uThickness: { value: THICKNESS },
  uZScale: { value: Z_SCALE },
  uTexel: { value: new THREE.Vector2(1 / cols, 1 / rows) },
  uCellMm: { value: new THREE.Vector2(field.cellMm, field.cellMm) },
  uHalf: { value: new THREE.Vector2(WIDTH / 2, HEIGHT / 2) },
};
const surfaceMaterial = new THREE.ShaderMaterial({
  uniforms: surfaceUniforms,
  vertexShader: `
    uniform sampler2D uHeights;
    uniform float uThickness;
    uniform float uZScale;
    uniform vec2 uHalf;
    varying vec2 vUv;
    void main() {
      vUv = (position.xy + uHalf) / (2.0 * uHalf);
      float depth = texture2D(uHeights, vUv).r * uThickness * uZScale;
      vec3 displaced = vec3(position.xy, -depth);
      gl_Position = projectionMatrix * modelViewMatrix * vec4(displaced, 1.0);
    }
  `,
  fragmentShader: `
    uniform sampler2D uHeights;
    uniform sampler2D uOwners;
    uniform float uThickness;
    uniform float uZScale;
    uniform vec2 uTexel;
    uniform vec2 uCellMm;
    varying vec2 vUv;
    void main() {
      float hl = texture2D(uHeights, vUv - vec2(uTexel.x, 0.0)).r;
      float hr = texture2D(uHeights, vUv + vec2(uTexel.x, 0.0)).r;
      float hd = texture2D(uHeights, vUv - vec2(0.0, uTexel.y)).r;
      float hu = texture2D(uHeights, vUv + vec2(0.0, uTexel.y)).r;
      vec3 normal = normalize(vec3(
        (hl - hr) * uThickness * uZScale / uCellMm.x,
        1.0,
        (hd - hu) * uThickness * uZScale / uCellMm.y));
      float owner = floor(texture2D(uOwners, vUv).r * 255.0 + 0.5);
      vec3 base = owner > 1.5 ? vec3(0.25, 0.72, 0.69) : owner > 0.5 ? vec3(0.29, 0.50, 0.83) : vec3(0.60, 0.62, 0.66);
      float light = 0.55 + 0.45 * max(dot(normal, normalize(vec3(0.4, 0.35, 0.85))), 0.0);
      gl_FragColor = vec4(base * light, 1.0);
    }
  `,
});

// Worst-case mesh density from the plan: 2560 on the long side.
const grid = new THREE.PlaneGeometry(WIDTH, HEIGHT, 2560, 1920);
grid.deleteAttribute('normal');
grid.deleteAttribute('uv');
const surface = new THREE.Mesh(grid, surfaceMaterial);
surface.rotation.x = -Math.PI / 2;
scene.add(surface);

// --- Skirts: stock border and interior cliffs -------------------------------
const quantum = THICKNESS / 65535;
const levelToZ = (level: number) => -level * quantum * Z_SCALE;
const positions: number[] = [];
const indices: number[] = [];
const quad = (ax: number, ay: number, az: number, bx: number, by: number, bz: number,
  cx: number, cy: number, cz: number, dx: number, dy: number, dz: number) => {
  const base = positions.length / 3;
  positions.push(ax, ay, az, bx, by, bz, cx, cy, cz, dx, dy, dz);
  indices.push(base, base + 1, base + 2, base, base + 2, base + 3);
};
const levelAt = (col: number, row: number) => heightsField[row * cols + col];
const cellX = (col: number) => (col + 0.5) * field.cellMm - WIDTH / 2;
const cellY = (row: number) => (row + 0.5) * field.cellMm - HEIGHT / 2;
const bottom = -THICKNESS * Z_SCALE;

const buildBorderSkirt = () => {
  for (let col = 0; col < cols - 1; col++) {
    const x0 = cellX(col) - field.cellMm / 2;
    const x1 = cellX(col + 1) - field.cellMm / 2;
    quad(x0, -HEIGHT / 2, levelToZ(levelAt(col, 0)), x1, -HEIGHT / 2, levelToZ(levelAt(col + 1, 0)), x1, -HEIGHT / 2, bottom, x0, -HEIGHT / 2, bottom);
    quad(x0, HEIGHT / 2, levelToZ(levelAt(col, rows - 1)), x1, HEIGHT / 2, levelToZ(levelAt(col + 1, rows - 1)), x1, HEIGHT / 2, bottom, x0, HEIGHT / 2, bottom);
  }
  for (let row = 0; row < rows - 1; row++) {
    const y0 = cellY(row) - field.cellMm / 2;
    const y1 = cellY(row + 1) - field.cellMm / 2;
    quad(-WIDTH / 2, y0, levelToZ(levelAt(0, row)), -WIDTH / 2, y1, levelToZ(levelAt(0, row + 1)), -WIDTH / 2, y1, bottom, -WIDTH / 2, y0, bottom);
    quad(WIDTH / 2, y0, levelToZ(levelAt(cols - 1, row)), WIDTH / 2, y1, levelToZ(levelAt(cols - 1, row + 1)), WIDTH / 2, y1, bottom, WIDTH / 2, y0, bottom);
  }
};

let cliffThresholdMm = 0.5;
let cliffQuads = 0;
const CLIFF_BUDGET = 400_000;
const buildCliffs = () => {
  cliffQuads = 0;
  for (let row = 0; row < rows; row++) {
    for (let col = 0; col < cols; col++) {
      const level = levelAt(col, row);
      if (col + 1 < cols) {
        const other = levelAt(col + 1, row);
        if (Math.abs(other - level) * quantum > cliffThresholdMm) {
          const x = (cellX(col) + cellX(col + 1)) / 2;
          quad(x, cellY(row) - field.cellMm / 2, levelToZ(level), x, cellY(row) + field.cellMm / 2, levelToZ(level),
            x, cellY(row) + field.cellMm / 2, levelToZ(other), x, cellY(row) - field.cellMm / 2, levelToZ(other));
          cliffQuads++;
        }
      }
      if (row + 1 < rows) {
        const other = levelAt(col, row + 1);
        if (Math.abs(other - level) * quantum > cliffThresholdMm) {
          const y = (cellY(row) + cellY(row + 1)) / 2;
          quad(cellX(col) - field.cellMm / 2, y, levelToZ(level), cellX(col) + field.cellMm / 2, y, levelToZ(level),
            cellX(col) + field.cellMm / 2, y, levelToZ(other), cellX(col) - field.cellMm / 2, y, levelToZ(other));
          cliffQuads++;
        }
      }
    }
  }
};
const skirtGeometry = new THREE.BufferGeometry();
const skirt = new THREE.Mesh(skirtGeometry, new THREE.MeshStandardMaterial({ color: 0x4c5158, side: THREE.DoubleSide, roughness: 0.9, metalness: 0.05 }));
skirt.rotation.x = -Math.PI / 2;
scene.add(skirt);
const buildSkirts = () => {
  positions.length = 0;
  indices.length = 0;
  buildBorderSkirt();
  buildCliffs();
  while (cliffQuads > CLIFF_BUDGET && cliffThresholdMm < 4) {
    positions.length = 0;
    indices.length = 0;
    cliffThresholdMm *= 1.6;
    buildBorderSkirt();
    buildCliffs();
  }
  results.cliffSkirts = { quads: cliffQuads, thresholdMm: cliffThresholdMm };
  skirtGeometry.dispose();
  skirtGeometry.setAttribute('position', new THREE.Float32BufferAttribute(positions, 3));
  skirtGeometry.setIndex(indices);
};

// --- Tool meshes -------------------------------------------------------------
const toolMaterial = new THREE.MeshStandardMaterial({ color: 0xb8bec6, roughness: 0.35, metalness: 0.8 });
const endmillMesh = new THREE.Mesh(new THREE.CylinderGeometry(3, 3, 26, 24), toolMaterial);
const vbitMesh = new THREE.Mesh(new THREE.LatheGeometry([
  new THREE.Vector2(0, 0), new THREE.Vector2(0.1, 0),
  new THREE.Vector2(1.5875, 2.576), new THREE.Vector2(1.5875, 4.2),
  new THREE.Vector2(4.5, 4.2), new THREE.Vector2(4.5, 30),
], 28), toolMaterial);
const tools: THREE.Group[] = [];
for (const mesh of [endmillMesh, vbitMesh]) {
  const group = new THREE.Group();
  mesh.position.y = 15;
  group.add(mesh);
  tools.push(group);
  scene.add(group);
}
const placeTool = (index: number, x: number, y: number, z: number) => {
  tools[1 - index].visible = false;
  const group = tools[index];
  group.visible = true;
  group.position.set(x - WIDTH / 2, -(-z) * Z_SCALE, HEIGHT / 2 - y);
};

// --- GPU verification helpers -------------------------------------------------
// The probe renders a 256x256 texel region through linear sampling; fragment
// centers land on texel centers when the rect starts exactly at the region
// origin (fragment centers are at half-pixel offsets already).
const probeTarget = new THREE.WebGLRenderTarget(256, 256, { depthBuffer: false });
const probeScene = new THREE.Scene();
const probeCamera = new THREE.OrthographicCamera(-0.5, 0.5, 0.5, -0.5, 0, 1);
const probeUniforms = {
  uHeights: { value: heightsTex },
  uRect: { value: new THREE.Vector4(0, 0, 1, 1) },
};
probeScene.add(new THREE.Mesh(new THREE.PlaneGeometry(1, 1), new THREE.ShaderMaterial({
  uniforms: probeUniforms,
  vertexShader: 'varying vec2 vUv; void main() { vUv = position.xy + 0.5; gl_Position = vec4(position.xy, 0.0, 1.0); }',
  fragmentShader: `
    uniform sampler2D uHeights;
    uniform vec4 uRect; // x, y, w, h in uv space
    varying vec2 vUv;
    void main() { gl_FragColor = vec4(texture2D(uHeights, uRect.xy + vUv * uRect.zw).r, 0.0, 0.0, 1.0); }
  `,
})));
const probePixels = new Uint8Array(256 * 256 * 4);
const debugBox = new THREE.Box2();
const debugDst = new THREE.Vector2();
const probeRegion = (x: number, y: number, expect: (px: number, py: number) => number) => {
  probeUniforms.uRect.value.set(x / cols, y / rows, 256 / cols, 256 / rows);
  renderer.setRenderTarget(probeTarget);
  renderer.render(probeScene, probeCamera);
  renderer.readRenderTargetPixels(probeTarget, 0, 0, 256, 256, probePixels);
  renderer.setRenderTarget(null);
  let maxError = 0;
  for (let py = 0; py < 256; py++) {
    for (let px = 0; px < 256; px++) {
      const value = probePixels[(py * 256 + px) * 4] / 255;
      maxError = Math.max(maxError, Math.abs(value - expect(px, py)));
    }
  }
  return maxError;
};

// --- Phase 1: carved-state upload + probe + region uploads -------------------
try {
  for (const motion of motions) applyMotion(field, motion);
  for (let tile = 0; tile < field.heights.length; tile++) blitTile(tile);
  const initialUploadStart = performance.now();
  heightsTex.needsUpdate = true;
  ownersTex.needsUpdate = true;
  renderer.initTexture(heightsTex);
  renderer.initTexture(ownersTex);
  renderer.compile(scene, camera);
  renderer.render(scene, camera);
  results.initialUpload = { texels: cols * rows, ms: +(performance.now() - initialUploadStart).toFixed(1) };

  interface WorstCell { px: number; py: number; gpu: number; cpu: number }
  const worstCells = (x: number, y: number): WorstCell[] => {
    const worst: WorstCell[] = [];
    for (let py = 0; py < 256 && worst.length < 8; py++) {
      for (let px = 0; px < 256; px++) {
        const gpu = probePixels[(py * 256 + px) * 4];
        const cpu = Math.round(heightsField[(y + py) * cols + x + px] / 257);
        if (Math.abs(gpu - cpu) > 3) worst.push({ px, py, gpu, cpu });
      }
    }
    return worst;
  };
  const regionReports: Array<{ x: number; y: number; error: number; worst: WorstCell[] }> = [];
  let probeError = 0;
  for (const [x, y] of [[1000, 700], [2000, 1800], [300, 2200]] as const) {
    probeError = Math.max(probeError, probeRegion(x, y, (px, py) => heightsField[(y + py) * cols + x + px] / 65535));
    regionReports.push({ x, y, error: 0, worst: worstCells(x, y) });
  }
  results.probe = { samples: 3 * 256 * 256, maxByteError: +(probeError * 255).toFixed(2), pass: probeError * 255 <= 1.01 };
  (results.probe as unknown as { regions?: typeof regionReports }).regions = regionReports;
  window.__probeDebug = {
    probe: probeRegion,
    copyRegion: (x: number, y: number, width: number, height: number) => {
      debugBox.min.set(x, y);
      debugBox.max.set(x + width, y + height);
      debugDst.set(x, y);
      renderer.copyTextureToTexture(heightsCpu, heightsTex, debugBox, debugDst);
    },
    pixels: probePixels,
    field: heightsField,
    half: heightsHalf,
    cols,
    rows,
    renderer,
    heightsTex,
  };

  // Isolate vertex-stage sampling of the R16 height texture from the
  // fragment stage: a quad whose vertex shader samples uHeights at a uniform
  // uv inside the pocket floor and forwards the value as a varying.
  {
    const vx = 2050;
    const vy = 1850;
    const vtfScene = new THREE.Scene();
    const vtfCamera = new THREE.OrthographicCamera(-0.5, 0.5, 0.5, -0.5, 0, 1);
    const vtfTarget = new THREE.WebGLRenderTarget(4, 4, { depthBuffer: false });
    vtfScene.add(new THREE.Mesh(new THREE.PlaneGeometry(1, 1), new THREE.ShaderMaterial({
      uniforms: {
        uHeights: { value: heightsTex },
        uUv: { value: new THREE.Vector2((vx + 0.5) / cols, (vy + 0.5) / rows) },
      },
      vertexShader: `
        uniform sampler2D uHeights;
        uniform vec2 uUv;
        varying float vSample;
        void main() {
          vSample = texture2D(uHeights, uUv).r;
          gl_Position = vec4(position.xy, 0.0, 1.0);
        }
      `,
      fragmentShader: `
        varying float vSample;
        void main() { gl_FragColor = vec4(vSample, 0.0, 0.0, 1.0); }
      `,
    })));
    renderer.setRenderTarget(vtfTarget);
    renderer.render(vtfScene, vtfCamera);
    const vtfPixels = new Uint8Array(4 * 4 * 4);
    renderer.readRenderTargetPixels(vtfTarget, 0, 0, 4, 4, vtfPixels);
    renderer.setRenderTarget(null);
    const cpuByte = Math.round(heightsField[vy * cols + vx] / 257);
    const vertexByte = vtfPixels[(2 * 4 + 2) * 4];
    results.vertexFetch = { cpu: cpuByte, vertexByte, fragmentByte: -1, pass: Math.abs(vertexByte - cpuByte) <= 2 };
    vtfTarget.dispose();
  }
  buildSkirts();

  // Region uploads: verify a recognizable pattern round-trips, then time 200.
  const targets: Array<[number, number]> = [];
  for (let index = 0; index < 6; index++) {
    targets.push([(index * 7919) % (cols - 256), (index * 104729) % (rows - 256)]);
  }
  for (const [x, y] of targets) {
    for (let py = 0; py < 256; py++) {
      for (let px = 0; px < 256; px++) {
        const level = ((px ^ py) & 255) * 257;
        const destination = (y + py) * cols + x + px;
        heightsField[destination] = level;
        heightsHalf[destination] = THREE.DataUtils.toHalfFloat(level / 65535);
      }
    }
  }
  let regionError = 0;
  const region = new THREE.Box2();
  const position = new THREE.Vector2();
  let worstRegion: { x: number; y: number; worst: Array<{ px: number; py: number; gpu: number; cpu: number }> } | null = null;
  for (const [x, y] of targets) {
    region.min.set(x, y);
    region.max.set(x + 256, y + 256);
    position.set(x, y);
    renderer.copyTextureToTexture(heightsCpu, heightsTex, region, position);
    regionError = Math.max(regionError, probeRegion(x, y, (px, py) => (((px ^ py) & 255) * 257) / 65535));
    const worst: Array<{ px: number; py: number; gpu: number; cpu: number }> = [];
    for (let py = 0; py < 256 && worst.length < 8; py++) {
      for (let px = 0; px < 256; px++) {
        const gpu = probePixels[(py * 256 + px) * 4];
        const cpu = Math.round((((px ^ py) & 255) * 257) / 257);
        if (Math.abs(gpu - cpu) > 3) worst.push({ px, py, gpu, cpu });
      }
    }
    if (worst.length && !worstRegion) worstRegion = { x, y, worst };
  }
  const regionStart = performance.now();
  for (let index = 0; index < 200; index++) {
    const x = (index * 613) % (cols - 256);
    const y = (index * 1543) % (rows - 256);
    region.min.set(x, y);
    region.max.set(x + 256, y + 256);
    position.set(x, y);
    renderer.copyTextureToTexture(heightsCpu, heightsTex, region, position);
  }
  renderer.render(scene, camera);
  const regionMs = performance.now() - regionStart;
  results.regionUpload = {
    uploads: 200,
    msPerUpload: +(regionMs / 200).toFixed(3),
    verifyByteError: +(regionError * 255).toFixed(2),
    pass: regionError * 255 <= 1.01,
  };
  if (worstRegion) (results.regionUpload as unknown as { worst?: typeof worstRegion }).worst = worstRegion;
} catch (error) {
  fail(`upload/probe: ${error}`);
}

// --- Phase 2: reset to pristine stock and play the carve live ----------------
const prefixSeconds = new Array<number>(motions.length + 1);
prefixSeconds[0] = 0;
for (let index = 0; index < motions.length; index++) {
  const m = motions[index];
  prefixSeconds[index + 1] = prefixSeconds[index]
    + Math.hypot(m.x1 - m.x0, m.y1 - m.y0, m.z1 - m.z0) / (m.tool === 0 ? ENDMILL_FEED : VBIT_FEED) * 60;
}
const indexForTime = (time: number) => {
  let lo = 0;
  let hi = motions.length;
  while (lo + 1 < hi) {
    const mid = (lo + hi) >> 1;
    if (prefixSeconds[mid] <= time) lo = mid; else hi = mid;
  }
  const span = prefixSeconds[lo + 1] - prefixSeconds[lo];
  return { index: lo, fraction: span > 0 ? Math.min(1, (time - prefixSeconds[lo]) / span) : 1 };
};

for (let tile = 0; tile < field.heights.length; tile++) {
  field.heights[tile] = undefined;
  field.cellOwner[tile] = undefined;
  field.tileVersions[tile] = 0;
}
field.stats.dirtyCells = 0;
field.stats.removedVolumeMm3 = 0;
field.stats.stageRemovedMm3 = [0, 0];
heightsField.fill(0);
heightsHalf.fill(0);
ownersField.fill(0);
heightsTex.needsUpdate = true;
ownersTex.needsUpdate = true;
renderer.initTexture(heightsTex);
renderer.initTexture(ownersTex);
buildSkirts();
beginEpoch(field);

const emitted = new Uint32Array(field.tileVersions.length);
let applied = 0;
let partialIndex = -1;
let partialFraction = 0;
const PLAY_SPEED = 30;
const PLAY_WALL_BUDGET = 12_000;
const playbackStats = { frames: 0, changedTiles: 0, uploadMs: 0 };
const regionBox = new THREE.Box2();
const regionDst = new THREE.Vector2();
const syncChangedTiles = () => {
  const uploadStart = performance.now();
  let changed = 0;
  for (let tile = 0; tile < emitted.length; tile++) {
    if (field.tileVersions[tile] === emitted[tile]) continue;
    emitted[tile] = field.tileVersions[tile];
    blitTile(tile);
    const x = (tile % field.tilesX) * 256;
    const y = Math.floor(tile / field.tilesX) * 256;
    regionBox.min.set(x, y);
    regionBox.max.set(x + Math.min(256, cols - x), y + Math.min(256, rows - y));
    regionDst.set(x, y);
    renderer.copyTextureToTexture(heightsCpu, heightsTex, regionBox, regionDst);
    renderer.copyTextureToTexture(ownersCpu, ownersTex, regionBox, regionDst);
    changed++;
  }
  playbackStats.changedTiles += changed;
  playbackStats.uploadMs += performance.now() - uploadStart;
};

const playbackResult: NonNullable<SpikeResults['playback']> = {
  motions: motions.length, modelSeconds: +modelSeconds.toFixed(1), wallSeconds: 0, speed: PLAY_SPEED,
  sustained: 0, frames: 0, avgChangedTiles: 0, avgUploadMs: 0, pass: false,
};
let playing = true;
let modelTime = 0;
const playbackStart = performance.now();
let lastNow = performance.now();

renderer.setAnimationLoop(() => {
  const now = performance.now();
  const dt = (now - lastNow) / 1000;
  lastNow = now;
  controls.update();
  if (playing) {
    modelTime = Math.min(modelSeconds, modelTime + dt * PLAY_SPEED);
    const target = indexForTime(modelTime);
    if (partialIndex >= 0 && partialIndex < target.index) {
      applyMotion(field, motions[partialIndex], partialFraction, 1);
      partialIndex = -1;
    }
    while (applied < target.index) applyMotion(field, motions[applied++]);
    if (target.index < motions.length) {
      if (partialIndex === target.index) {
        applyMotion(field, motions[target.index], partialFraction, Math.max(target.fraction, partialFraction));
      } else {
        applyMotion(field, motions[target.index], 0, target.fraction);
      }
      partialIndex = target.index;
      partialFraction = target.fraction;
      const m = motions[target.index];
      placeTool(m.tool, m.x0 + (m.x1 - m.x0) * target.fraction, m.y0 + (m.y1 - m.y0) * target.fraction,
        m.z0 + (m.z1 - m.z0) * target.fraction);
    } else {
      applied = motions.length;
      tools[0].visible = false;
      tools[1].visible = false;
    }
    syncChangedTiles();
    playbackStats.frames++;
    if (modelTime >= modelSeconds || now - playbackStart > PLAY_WALL_BUDGET) {
      while (applied < motions.length) applyMotion(field, motions[applied++]);
      if (partialIndex >= 0) applyMotion(field, motions[partialIndex], partialFraction, 1);
      for (let tile = 0; tile < field.heights.length; tile++) blitTile(tile);
      syncChangedTiles();
      buildSkirts();
      playing = false;
      const wall = (performance.now() - playbackStart) / 1000;
      playbackResult.wallSeconds = +wall.toFixed(2);
      playbackResult.frames = playbackStats.frames;
      playbackResult.avgChangedTiles = +(playbackStats.changedTiles / Math.max(1, playbackStats.frames)).toFixed(1);
      playbackResult.avgUploadMs = +(playbackStats.uploadMs / Math.max(1, playbackStats.frames)).toFixed(2);
      playbackResult.sustained = +(modelTime / wall).toFixed(1);
      playbackResult.pass = playbackResult.sustained >= 15;
      results.playback = playbackResult;
      startFpsPhase();
    }
  }
  renderer.render(scene, camera);
  if (playbackStats.frames % 30 === 0) renderHud();
});

// --- Phase 3: orbit fps on the worst-case mesh -------------------------------
const fpsPhase: NonNullable<SpikeResults['fps']> = { frames: 0, seconds: 0, fps: 0, triangles: 0, pass: false };
function startFpsPhase() {
  controls.autoRotate = true;
  const started = performance.now();
  const frames = { count: 0 };
  const sample = () => {
    frames.count++;
    const elapsed = (performance.now() - started) / 1000;
    if (elapsed >= 4) {
      fpsPhase.frames = frames.count;
      fpsPhase.seconds = +elapsed.toFixed(2);
      fpsPhase.fps = +(frames.count / elapsed).toFixed(1);
      fpsPhase.triangles = renderer.info.render.triangles;
      fpsPhase.pass = fpsPhase.fps >= 25;
      results.fps = fpsPhase;
      controls.autoRotate = false;
      results.done = true;
      renderHud();
      console.log('U8 spike results', results);
      return;
    }
    requestAnimationFrame(sample);
  };
  requestAnimationFrame(sample);
}

renderHud();
