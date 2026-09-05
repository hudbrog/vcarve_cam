import { readFileSync, writeFileSync } from 'node:fs';
import assert from 'node:assert/strict';

// Lossless projection of captured engine geometry into the proposed display DTO.
// Dividing stored grid coordinates by ticks/mm changes units, not geometry.
const root = new URL('../src/fixtures/', import.meta.url);
const capture = JSON.parse(readFileSync(new URL('inkscape.inspection.json', root), 'utf8'));
const job = JSON.parse(readFileSync(new URL('inkscape.job.json', root), 'utf8'));
const inspection = capture.inspection;
const display = {
  coordinateSpace: 'source-page-mm-y-up',
  widthMm: inspection.geometry.page_width_mm,
  heightMm: inspection.geometry.page_height_mm,
  engineVersion: inspection.engine_version,
  geometryToleranceMm: job.import.geometry_tolerance_mm,
  description: 'Captured Rust inspection · synthetic Inkscape artwork',
  components: inspection.geometry.sources.map(source => ({
    id: source.id, label: source.label || source.source_id,
    rings: source.geometry.rings.map(ring => ({
      hole: ring.is_hole,
      points: ring.points.map(point => ({
        x: point.x / source.geometry.grid.ticks_per_mm,
        y: point.y / source.geometry.grid.ticks_per_mm,
      })),
    })),
  })),
};
const destination = new URL('inkscape.display.json', root);
if (process.argv.includes('--check')) {
  assert.deepEqual(JSON.parse(readFileSync(destination, 'utf8')), display, 'Display projection differs from captured Rust inspection');
  console.log('Display coordinates, IDs, and hole flags match captured Rust geometry.');
} else {
  writeFileSync(destination, JSON.stringify(display) + '\n');
  console.log('Wrote display projection. No machining calculations were performed.');
}
