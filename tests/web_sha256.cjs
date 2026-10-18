// Run with node tests/web_sha256.cjs; pnpm test also checks the built asset.
const assert = require('node:assert/strict');
const crypto = require('node:crypto');
(async () => {
  const { sha256 } = await import('../src/web/sha256.js');
  const vectors = [Buffer.alloc(0), Buffer.from('abc')];
  for (const length of [55, 56, 63, 64, 65, 5624, 1966080]) {
    vectors.push(crypto.randomBytes(length));
  }
  for (const bytes of vectors) {
    assert.equal(sha256(bytes), crypto.createHash('sha256').update(bytes).digest('hex'));
  }
  console.log('Browser SHA-256 verified: known vectors, padding boundaries, frame and maximum OTA sizes.');

})().catch(error => { console.error(error); process.exitCode = 1; });
