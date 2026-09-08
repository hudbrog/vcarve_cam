// Build the in-browser engine into web/src/wasm/gen for Vite to bundle.
import { execSync } from 'node:child_process';
import { fileURLToPath } from 'node:url';

const crate = fileURLToPath(new URL('../../crates/cam-wasm', import.meta.url));
const outDir = fileURLToPath(new URL('../src/wasm/gen', import.meta.url));

try {
  execSync('wasm-pack --version', { stdio: 'pipe' });
} catch {
  console.error('wasm-pack is not installed. Install it from https://rustwasm.github.io/wasm-pack/installer/ and retry.');
  process.exit(1);
}
execSync(
  `wasm-pack build "${crate}" --target web --release --out-dir "${outDir}" --out-name cam_wasm`,
  { stdio: 'inherit' },
);
console.log(`Engine module written to ${outDir}`);
