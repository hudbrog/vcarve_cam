// Writes fixtures/gui10/tight-holder.job.json from the gui4 lettering carving:
// a 3 mm carve, a 2.5 mm endmill that sticks 0.8 mm out of its holder, and a
// machine configuration whose holder is an ER20 collet chuck. The applied
// machine snapshot is produced by `cam collection apply-machine`, so the fixture
// is written by the same code path the GUI uses rather than by hand.
//
// Run from the workspace root after building the CLI:
//   cargo build --locked -p cam-app
//   node fixtures/gui10/create-fixture.mjs
import {execFileSync} from 'node:child_process';
import {readFileSync, rmSync, writeFileSync} from 'node:fs';
import path from 'node:path';

const root = process.cwd();
const source = JSON.parse(
  readFileSync(path.join(root, 'fixtures/gui4/lettering.job.json'), 'utf8'),
);
// The carve reaches 1.5 mm deep in this artwork; 0.8 mm of stickout puts the
// holder's nut 0.7 mm inside the material beside the cut.
source.operations[0].settings.settings.max_depth_mm = 3.0;
const endmill = source.tools.find(tool => tool.id === 'endmill');
endmill.assembly = {shaft_diameter_mm: 2.5, stickout_mm: 0.8};

const machine = JSON.parse(
  readFileSync(path.join(root, 'fixtures/gui2/machine.json'), 'utf8'),
);
machine.holder = {id: 'er20'};
const profile = path.join(root, 'fixtures/gui10/er20-machine.json');
writeFileSync(profile, `${JSON.stringify(machine, null, 2)}\n`);

const bare = path.join(root, 'fixtures/gui10/.tight-holder-bare.job.json');
writeFileSync(bare, `${JSON.stringify(source, null, 2)}\n`);
const output = path.join(root, 'fixtures/gui10/tight-holder.job.json');
execFileSync(
  path.join(root, 'target/debug/cam.exe'),
  ['collection', 'apply-machine', bare, '--profile', profile, '--name', 'GUI10 fixture', '--output', output],
  {stdio: 'inherit'},
);
rmSync(bare);
console.log(`wrote ${path.relative(root, output)}`);
