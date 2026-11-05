// Runs wrangler with wrangler.local.jsonc (your domain and D1 id, gitignored) when it exists, else
// wrangler.jsonc. The npm scripts for deploy, db:migrate and dev:api go through this.
// Usage: node tools/wr.mjs <wrangler arguments>   e.g. node tools/wr.mjs deploy
import { existsSync } from 'node:fs';
import { spawnSync } from 'node:child_process';
import { dirname, join } from 'node:path';
import { fileURLToPath } from 'node:url';

const root = join(dirname(fileURLToPath(import.meta.url)), '..');
const local = join(root, 'wrangler.local.jsonc');
const args = process.argv.slice(2);
if (existsSync(local)) args.push('-c', local);
const r = spawnSync('npx', ['wrangler', ...args], { cwd: root, stdio: 'inherit', shell: process.platform === 'win32' });
process.exit(r.status ?? 1);
