import {writeFile} from 'node:fs/promises';
import {fixtures} from './fixtures.mjs';
await writeFile(new URL('./fixtures.json',import.meta.url),JSON.stringify(fixtures(),null,2)+'\n');
