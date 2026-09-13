// Builds the GUI9 larger-job fixture: the real flower_box.svg artwork placed
// as a batch of identical carvings on one stock, one Flat V-carve operation
// per copy. Same bytes, same cutter and same cutting values as the accepted
// GUI2 fixture; only the placement and the number of copies change.
//
//   node fixtures/gui9/create-batch-fixture.mjs [columns] [rows]
//
// Default 3 x 2 = six copies (about 137,000 motions), which is deliberately
// above the previous 100,000-motion display limit. The spacing and the stock
// rectangle are derived from the artwork extent so the fixture stays
// reproducible from the committed source job.
import { readFileSync, writeFileSync } from 'node:fs';
import { fileURLToPath } from 'node:url';
import { dirname, join } from 'node:path';

const here = dirname(fileURLToPath(import.meta.url));
const source = JSON.parse(
  readFileSync(join(here, '..', 'gui2', 'flower.job.json'), 'utf8'),
);
const columns = Number(process.argv[2] ?? 3);
const rows = Number(process.argv[3] ?? 2);
if (!Number.isInteger(columns) || !Number.isInteger(rows) || columns < 1 || rows < 1) {
  throw new Error('columns and rows must be positive integers');
}

const template = source.artwork[0];
const templateOperation = source.operations[0];
const pitchX = 210;
const pitchY = 110;
const gap = 10;
const stock = {
  thickness_mm: source.setup.stock.thickness_mm,
  xy: {
    min_x_mm: 0,
    min_y_mm: 0,
    width_mm: columns * 200 + (columns - 1) * gap,
    length_mm: rows * 100 + (rows - 1) * gap,
  },
};

const artwork = [];
const operations = [];
for (let row = 0; row < rows; row += 1) {
  for (let column = 0; column < columns; column += 1) {
    const index = row * columns + column + 1;
    const artworkId = `gui9-artwork-${index}`;
    const operationId = `gui9-flat-v-carve-${index}`;
    artwork.push({
      ...structuredClone(template),
      id: artworkId,
      placement: {
        origin_mm: { x: column * pitchX, y: row * pitchY },
        scale: template.placement.scale,
        rotation_deg: template.placement.rotation_deg,
      },
    });
    const operation = structuredClone(templateOperation);
    operation.id = operationId;
    operation.name = `Flat V-carve ${index}`;
    operation.settings.settings.components = templateOperation.settings.settings.components.map(
      (component) => ({ ...component, artwork_item_id: artworkId }),
    );
    operations.push(operation);
  }
}

const job = {
  ...source,
  name: `flower_box_batch_${columns}x${rows}`,
  setup: { ...source.setup, stock },
  artwork,
  operations,
};
writeFileSync(
  join(here, 'flower-box-batch.job.json'),
  `${JSON.stringify(job)}\n`,
  'utf8',
);
console.log(
  `${job.name}: ${artwork.length} artwork items, ${operations.length} operations, stock ${stock.xy.width_mm} x ${stock.xy.length_mm} mm`,
);
