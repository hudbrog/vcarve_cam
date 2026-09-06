import { createHash } from 'node:crypto';
import { lstatSync, readdirSync, readFileSync, writeFileSync, existsSync } from 'node:fs';
import { join } from 'node:path';
import { fileURLToPath } from 'node:url';

const web = fileURLToPath(new URL('../', import.meta.url));
function files(root, relative) {
  const path = join(root, relative);
  const stat = lstatSync(path);
  if (stat.isSymbolicLink()) throw new Error(`Bundle inputs must not be symlinks: ${relative}`);
  if (stat.isDirectory()) return readdirSync(path).sort().flatMap(name => files(root, relative ? `${relative}/${name}` : name));
  if (!stat.isFile()) throw new Error(`Not a regular file: ${relative}`);
  return [relative];
}
function hashes(root, paths) {
  return Object.fromEntries(paths.sort().map(path => [path, createHash('sha256').update(readFileSync(join(root, path))).digest('hex')]));
}
const sources = ['index.html', 'package.json', 'pnpm-lock.yaml', 'tsconfig.json', 'vite.config.ts',
  ...['src', 'scripts', 'public'].flatMap(path => existsSync(join(web, path)) ? files(web, path) : [])];
const dist = join(web, 'dist');
const assets = files(dist, '').filter(path => path !== '.bundle-manifest.json');
if (!assets.includes('index.html')) throw new Error('Production build has no index.html');
const engineVersion = readFileSync(join(web, '../Cargo.toml'), 'utf8').match(/^version\s*=\s*"([^"]+)"/m)?.[1];
if (!engineVersion) throw new Error('Cannot read workspace engine version');
writeFileSync(join(dist, '.bundle-manifest.json'), JSON.stringify({ schemaVersion: 1, engineVersion,
  sources: hashes(web, sources), assets: hashes(dist, assets) }, null, 2) + '\n');
console.log(`Recorded ${assets.length} assets and ${sources.length} source hashes for engine ${engineVersion}.`);
