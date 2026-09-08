// three.js stock-surface renderer for the 3D simulator (U8 §6), ported from
// the validated phase-1 spike: R16F heights + R8 owners textures, displaced
// grid with per-pixel normals, border and cliff skirts, animated tool meshes,
// and partial tile uploads through the public copyTextureToTexture path.
// Dynamically imported by SimViewport so three stays out of the main bundle.
import * as THREE from 'three';
import { OrbitControls } from 'three/addons/controls/OrbitControls.js';
import type { StockRect, ToolSpec } from './engine';
import { normalizeTool } from './engine';
import type { TileDelta } from './session';

export interface SimRendererConfig {
  stock: StockRect;
  cols: number;
  rows: number;
  cellMm: number;
  tools: ToolSpec[];
}

export interface SimRenderer {
  applyTileDeltas(tiles: TileDelta[]): void;
  rebuildSkirts(): void;
  setToolPose(toolIndex: number, x: number, y: number, z: number): void;
  hideTool(): void;
  setZScale(zScale: number): void;
  setTrackTool(track: boolean): void;
  setStageColors(enabled: boolean): void;
  fit(): void;
  resize(width: number, height: number): void;
  dispose(): void;
}

const STAGE_STOCK = 0x9a9da3;
const STAGE_ENDMILL = 0x4a80d4;
const STAGE_VBIT = 0x3fb8af;
const SKIRT = 0x4c5158;
const TOOL = 0xb8bec6;

export function createSimRenderer(canvas: HTMLCanvasElement, config: SimRendererConfig): SimRenderer {
  const { stock, cols, rows, cellMm } = config;
  const widthMm = stock.x1 - stock.x0;
  const heightMm = stock.y1 - stock.y0;
  const quantum = stock.thicknessMm / 65535;

  // CPU mirrors of the full field: Uint16 levels, half-float bits for the
  // GPU source, and the owner channel.
  const levels = new Uint16Array(cols * rows);
  const half = new Uint16Array(cols * rows);
  const owners = new Uint8Array(cols * rows);

  const renderer = new THREE.WebGLRenderer({ canvas, antialias: true });
  renderer.setPixelRatio(Math.min(window.devicePixelRatio, 2));
  const scene = new THREE.Scene();
  scene.background = new THREE.Color(0x14161a);
  const camera = new THREE.PerspectiveCamera(35, 1, 1, 40000);
  const controls = new OrbitControls(camera, canvas);
  controls.enableDamping = true;
  scene.add(new THREE.HemisphereLight(0xdfe6ee, 0x2c3038, 1.1));
  const sun = new THREE.DirectionalLight(0xffffff, 1.6);
  sun.position.set(widthMm * 0.45, widthMm * 0.8, heightMm * 0.35);
  scene.add(sun);

  const heightsCpu = new THREE.DataTexture(half, cols, rows, THREE.RedFormat, THREE.HalfFloatType);
  heightsCpu.flipY = false;
  const ownersCpu = new THREE.DataTexture(owners, cols, rows, THREE.RedFormat, THREE.UnsignedByteType);
  ownersCpu.flipY = false;
  const heightsTex = heightsCpu.clone();
  heightsTex.minFilter = THREE.LinearFilter;
  heightsTex.magFilter = THREE.LinearFilter;
  heightsTex.flipY = false;
  const ownersTex = ownersCpu.clone();
  ownersTex.minFilter = THREE.NearestFilter;
  ownersTex.magFilter = THREE.NearestFilter;
  ownersTex.flipY = false;
  heightsTex.needsUpdate = true;
  ownersTex.needsUpdate = true;
  renderer.initTexture(heightsTex);
  renderer.initTexture(ownersTex);

  const uniforms = {
    uHeights: { value: heightsTex },
    uOwners: { value: ownersTex },
    uThickness: { value: stock.thicknessMm },
    uZScale: { value: 3 },
    uTexel: { value: new THREE.Vector2(1 / cols, 1 / rows) },
    uCellMm: { value: new THREE.Vector2(cellMm, cellMm) },
    uHalf: { value: new THREE.Vector2(widthMm / 2, heightMm / 2) },
    uStockColor: { value: new THREE.Color(STAGE_STOCK) },
    uEndmillColor: { value: new THREE.Color(STAGE_ENDMILL) },
    uVbitColor: { value: new THREE.Color(STAGE_VBIT) },
    uStageColors: { value: 1 },
  };
  const surface = new THREE.Mesh(
    new THREE.PlaneGeometry(widthMm, heightMm, Math.min(cols, 2560), Math.min(rows, Math.round(2560 * rows / cols))),
    new THREE.ShaderMaterial({
      uniforms,
      vertexShader: `
        uniform sampler2D uHeights;
        uniform float uThickness;
        uniform float uZScale;
        uniform vec2 uHalf;
        varying vec2 vUv;
        void main() {
          vUv = (position.xy + uHalf) / (2.0 * uHalf);
          float depth = texture2D(uHeights, vUv).r * uThickness * uZScale;
          gl_Position = projectionMatrix * modelViewMatrix * vec4(position.xy, -depth, 1.0);
        }
      `,
      fragmentShader: `
        uniform sampler2D uHeights;
        uniform sampler2D uOwners;
        uniform float uThickness;
        uniform float uZScale;
        uniform vec2 uTexel;
        uniform vec2 uCellMm;
        uniform vec3 uStockColor;
        uniform vec3 uEndmillColor;
        uniform vec3 uVbitColor;
        uniform float uStageColors;
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
          vec3 staged = owner > 1.5 ? uVbitColor : owner > 0.5 ? uEndmillColor : uStockColor;
          vec3 base = mix(uStockColor, staged, uStageColors);
          float shade = 0.55 + 0.45 * max(dot(normal, normalize(vec3(0.4, 0.35, 0.85))), 0.0);
          gl_FragColor = vec4(base * shade, 1.0);
        }
      `,
    }),
  );
  surface.rotation.x = -Math.PI / 2;
  scene.add(surface);

  // Skirts: the stock border always, plus interior cliff walls between
  // steeply differing texels (rebuilt by the caller when it matters).
  const skirtPositions: number[] = [];
  const skirtIndices: number[] = [];
  const skirtGeometry = new THREE.BufferGeometry();
  const skirt = new THREE.Mesh(skirtGeometry, new THREE.MeshStandardMaterial({ color: SKIRT, side: THREE.DoubleSide, roughness: 0.9, metalness: 0.05 }));
  skirt.rotation.x = -Math.PI / 2;
  scene.add(skirt);
  let cliffThresholdMm = 0.5;
  const levelAt = (col: number, row: number) => levels[row * cols + col];
  const cellX = (col: number) => (col + 0.5) * cellMm - widthMm / 2;
  const cellY = (row: number) => (row + 0.5) * cellMm - heightMm / 2;
  const quad = (ax: number, ay: number, az: number, bx: number, by: number, bz: number, cx: number, cy: number, cz: number, dx: number, dy: number, dz: number) => {
    const base = skirtPositions.length / 3;
    skirtPositions.push(ax, ay, az, bx, by, bz, cx, cy, cz, dx, dy, dz);
    skirtIndices.push(base, base + 1, base + 2, base, base + 2, base + 3);
  };
  const zOf = (level: number) => -level * quantum * uniforms.uZScale.value;
  const bottomZ = () => -stock.thicknessMm * uniforms.uZScale.value;
  const buildSkirts = () => {
    skirtPositions.length = 0;
    skirtIndices.length = 0;
    for (let col = 0; col < cols - 1; col++) {
      const x0 = cellX(col) - cellMm / 2;
      const x1 = cellX(col + 1) - cellMm / 2;
      quad(x0, -heightMm / 2, zOf(levelAt(col, 0)), x1, -heightMm / 2, zOf(levelAt(col + 1, 0)), x1, -heightMm / 2, bottomZ(), x0, -heightMm / 2, bottomZ());
      quad(x0, heightMm / 2, zOf(levelAt(col, rows - 1)), x1, heightMm / 2, zOf(levelAt(col + 1, rows - 1)), x1, heightMm / 2, bottomZ(), x0, heightMm / 2, bottomZ());
    }
    for (let row = 0; row < rows - 1; row++) {
      const y0 = cellY(row) - cellMm / 2;
      const y1 = cellY(row + 1) - cellMm / 2;
      quad(-widthMm / 2, y0, zOf(levelAt(0, row)), -widthMm / 2, y1, zOf(levelAt(0, row + 1)), -widthMm / 2, y1, bottomZ(), -widthMm / 2, y0, bottomZ());
      quad(widthMm / 2, y0, zOf(levelAt(cols - 1, row)), widthMm / 2, y1, zOf(levelAt(cols - 1, row + 1)), widthMm / 2, y1, bottomZ(), widthMm / 2, y0, bottomZ());
    }
    let cliffs = 0;
    for (let row = 0; row < rows; row++) {
      for (let col = 0; col < cols; col++) {
        const level = levelAt(col, row);
        if (col + 1 < cols) {
          const other = levelAt(col + 1, row);
          if (Math.abs(other - level) * quantum > cliffThresholdMm) {
            const x = (cellX(col) + cellX(col + 1)) / 2;
            quad(x, cellY(row) - cellMm / 2, zOf(level), x, cellY(row) + cellMm / 2, zOf(level), x, cellY(row) + cellMm / 2, zOf(other), x, cellY(row) - cellMm / 2, zOf(other));
            cliffs++;
          }
        }
        if (row + 1 < rows) {
          const other = levelAt(col, row + 1);
          if (Math.abs(other - level) * quantum > cliffThresholdMm) {
            const y = (cellY(row) + cellY(row + 1)) / 2;
            quad(cellX(col) - cellMm / 2, y, zOf(level), cellX(col) + cellMm / 2, y, zOf(level), cellX(col) + cellMm / 2, y, zOf(other), cellX(col) - cellMm / 2, y, zOf(other));
            cliffs++;
          }
        }
      }
    }
    while (cliffs > 400_000 && cliffThresholdMm < 4) {
      cliffThresholdMm *= 1.6;
      return buildSkirts();
    }
    skirtGeometry.dispose();
    skirtGeometry.setAttribute('position', new THREE.Float32BufferAttribute(skirtPositions, 3));
    skirtGeometry.setIndex(skirtIndices);
  };

  // Tool meshes built from the actual cutter geometry.
  const toolMaterial = new THREE.MeshStandardMaterial({ color: TOOL, roughness: 0.35, metalness: 0.8 });
  const toolGroups: THREE.Group[] = config.tools.map(spec => {
    const group = new THREE.Group();
    let mesh: THREE.Mesh;
    if (spec.kind === 'endmill') {
      mesh = new THREE.Mesh(new THREE.CylinderGeometry(spec.diameterMm / 2, spec.diameterMm / 2, 30, 24), toolMaterial);
      mesh.position.y = 15;
    } else {
      const tool = normalizeTool(spec);
      const tipRadius = tool.kind === 'vbit' ? tool.tipRadiusMm : 0;
      const maxRadius = tool.kind === 'vbit' ? tool.maxRadiusMm : 1;
      const coneHeight = tool.kind === 'vbit' ? tool.maxDepthBelowTipMm : 1;
      mesh = new THREE.Mesh(new THREE.LatheGeometry([
        new THREE.Vector2(0, 0),
        new THREE.Vector2(tipRadius, 0),
        new THREE.Vector2(maxRadius, coneHeight),
        new THREE.Vector2(maxRadius, coneHeight + 1.8),
        new THREE.Vector2(Math.max(maxRadius, 4.5), coneHeight + 1.8),
        new THREE.Vector2(Math.max(maxRadius, 4.5), coneHeight + 28),
      ], 28), toolMaterial);
      mesh.position.y = 0;
    }
    group.add(mesh);
    group.visible = false;
    scene.add(group);
    return group;
  });
  let toolPose = { x: stock.x0, y: stock.y0, z: 0, tool: 0, visible: false };
  const placeTool = () => {
    for (let index = 0; index < toolGroups.length; index++) toolGroups[index].visible = toolPose.visible && index === toolPose.tool;
    if (!toolPose.visible || toolGroups.length === 0) return;
    toolGroups[toolPose.tool % toolGroups.length].position.set(
      toolPose.x - stock.x0 - widthMm / 2,
      -(-toolPose.z) * uniforms.uZScale.value,
      heightMm / 2 - (toolPose.y - stock.y0),
    );
  };

  let trackTool = false;
  const trackedTarget = new THREE.Vector3(0, 0, 0);
  let running = true;
  const frame = () => {
    if (!running) return;
    requestAnimationFrame(frame);
    controls.update();
    if (trackTool && toolPose.visible && toolGroups.length > 0) {
      trackedTarget.lerp(toolGroups[toolPose.tool % toolGroups.length].position, 0.08);
      controls.target.copy(trackedTarget);
    }
    renderer.render(scene, camera);
  };
  requestAnimationFrame(frame);

  const region = new THREE.Box2();
  const destination = new THREE.Vector2();
  const toHalf = THREE.DataUtils.toHalfFloat;

  const api: SimRenderer = {
    applyTileDeltas(tiles) {
      for (const tile of tiles) {
        const width = Math.min(256, cols - tile.x);
        const height = Math.min(256, rows - tile.y);
        if (width <= 0 || height <= 0) continue;
        for (let row = 0; row < height; row++) {
          const source = row * 256;
          const target = (tile.y + row) * cols + tile.x;
          for (let index = 0; index < width; index++) {
            const level = tile.heights[source + index];
            levels[target + index] = level;
            half[target + index] = toHalf(level / 65535);
            owners[target + index] = tile.owners[source + index];
          }
        }
        region.min.set(tile.x, tile.y);
        region.max.set(tile.x + width, tile.y + height);
        destination.set(tile.x, tile.y);
        renderer.copyTextureToTexture(heightsCpu, heightsTex, region, destination);
        renderer.copyTextureToTexture(ownersCpu, ownersTex, region, destination);
      }
    },
    rebuildSkirts: buildSkirts,
    setToolPose(toolIndex, x, y, z) {
      toolPose = { x, y, z, tool: toolIndex, visible: true };
      placeTool();
    },
    hideTool() {
      toolPose = { ...toolPose, visible: false };
      placeTool();
    },
    setZScale(zScale) {
      uniforms.uZScale.value = zScale;
      placeTool();
      buildSkirts();
    },
    setTrackTool(track) {
      trackTool = track;
    },
    setStageColors(enabled) {
      uniforms.uStageColors.value = enabled ? 1 : 0;
    },
    fit() {
      const span = Math.max(widthMm, heightMm);
      controls.target.set(0, 0, 0);
      camera.position.set(span * 0.68, span * 0.55, span * 0.85);
      controls.update();
    },
    resize(width, height) {
      renderer.setSize(width, height, false);
      camera.aspect = width / Math.max(height, 1);
      camera.updateProjectionMatrix();
    },
    dispose() {
      running = false;
      controls.dispose();
      surface.geometry.dispose();
      (surface.material as THREE.ShaderMaterial).dispose();
      skirtGeometry.dispose();
      (skirt.material as THREE.Material).dispose();
      for (const group of toolGroups) group.traverse(child => {
        if (child instanceof THREE.Mesh) child.geometry.dispose();
      });
      toolMaterial.dispose();
      heightsTex.dispose();
      ownersTex.dispose();
      heightsCpu.dispose();
      ownersCpu.dispose();
      renderer.dispose();
    },
  };
  api.fit();
  buildSkirts();
  return api;
}
