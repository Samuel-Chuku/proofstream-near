// Wraps @hot-labs/near-connect's CommonJS build into one ES module for a static page, with
// no bundler: each file becomes a function in a small module table. Reproducible:
//   npm pack @hot-labs/near-connect@0.12.0 && tar -xzf hot-labs-near-connect-0.12.0.tgz
//   node web/tools/bundle-near-connect.mjs package/build > web/vendor/near-connect-0.12.0.mjs
// Tarball sha256: b5cd265f70e76c1fa3aeba5fca5640939fb577f7a385ca96eb44ead8f8898e4e
import { readFileSync, readdirSync, statSync } from 'node:fs';
import { join, relative, dirname, posix } from 'node:path';

const root = process.argv[2];
const files = [];
const walk = (d) => readdirSync(d).sort().forEach((f) => {
  const p = join(d, f);
  if (statSync(p).isDirectory()) walk(p);
  else if (p.endsWith('.js')) files.push(relative(root, p).split('\\').join('/'));
});
walk(root);

const out = ['// @hot-labs/near-connect 0.12.0, wrapped by web/tools/bundle-near-connect.mjs. Do not edit.',
  'const defs = {};', 'const cache = {};',
  'function load(id) {',
  '  if (cache[id]) return cache[id].exports;',
  '  const module = (cache[id] = { exports: {} });',
  '  defs[id](module, module.exports, (spec) => load(resolve(id, spec)));',
  '  return module.exports;',
  '}',
  'function resolve(from, spec) {',
  '  const parts = (from.split("/").slice(0, -1).join("/") + "/" + spec).split("/");',
  '  const stack = [];',
  '  for (const p of parts) { if (p === "..") stack.pop(); else if (p && p !== ".") stack.push(p); }',
  '  const base = stack.join("/");',
  '  if (defs[base + ".js"]) return base + ".js";',
  '  if (defs[base + "/index.js"]) return base + "/index.js";',
  '  throw new Error("near-connect bundle: cannot resolve " + spec + " from " + from);',
  '}'];
for (const f of files) {
  const src = readFileSync(join(root, f), 'utf8').replace(/\n\/\/# sourceMappingURL=.*$/m, '');
  out.push(`defs[${JSON.stringify(f)}] = function (module, exports, require) {\n${src}\n};`);
}
out.push('const api = load("index.js");', 'export const NearConnector = api.NearConnector;', 'export default api;');
process.stdout.write(out.join('\n') + '\n');
