// Run with node tests/web_sha256.cjs; verifies the code served by the device.
const assert = require('node:assert/strict');
const crypto = require('node:crypto');
const fs = require('node:fs');
const vm = require('node:vm');
const path = require('node:path');
const page = fs.readFileSync(path.join(__dirname, '../src/web/index.html'), 'utf8');
const source = page.match(/function sha256\(input\)\{[\s\S]*?\}\nasync function upload/);
assert(source, 'Cannot locate the browser upload hash');
const context = {};
vm.createContext(context);
vm.runInContext(source[0].replace(/\nasync function upload$/, ''), context);
const vectors = [Buffer.alloc(0), Buffer.from('abc')];
for (const length of [55, 56, 63, 64, 65, 5624, 1966080]) {
  vectors.push(crypto.randomBytes(length));
}
for (const bytes of vectors) {
  assert.equal(context.sha256(bytes), crypto.createHash('sha256').update(bytes).digest('hex'));
}
console.log('Browser SHA-256 verified: known vectors, padding boundaries, frame and maximum OTA sizes.');
