// Local HTTP pages cannot depend on Web Crypto being available. Only uploads
// are hashed here; the firmware independently verifies their complete bodies.
export function sha256(input) {
  const bytes = new Uint8Array(input);
  const padded = new Uint8Array(Math.ceil((bytes.length + 9) / 64) * 64);
  padded.set(bytes);
  padded[bytes.length] = 0x80;
  new DataView(padded.buffer).setUint32(padded.length - 4, bytes.length * 8);

  const initial = [];
  const constants = [];
  for (let n = 2; constants.length < 64; n++) {
    let prime = true;
    for (let divisor = 2; divisor * divisor <= n; divisor++) {
      if (n % divisor === 0) { prime = false; break; }
    }
    if (prime) {
      if (initial.length < 8) initial.push((Math.sqrt(n) % 1 * 2 ** 32) | 0);
      constants.push((Math.cbrt(n) % 1 * 2 ** 32) | 0);
    }
  }

  const hash = initial.slice();
  const words = new Int32Array(64);
  const rotate = (value, bits) => (value >>> bits) | (value << (32 - bits));
  for (let offset = 0; offset < padded.length; offset += 64) {
    const block = new DataView(padded.buffer, offset, 64);
    for (let i = 0; i < 64; i++) {
      if (i < 16) words[i] = block.getInt32(i * 4);
      else {
        const x = words[i - 15], y = words[i - 2];
        const s0 = rotate(x, 7) ^ rotate(x, 18) ^ (x >>> 3);
        const s1 = rotate(y, 17) ^ rotate(y, 19) ^ (y >>> 10);
        words[i] = (words[i - 16] + s0 + words[i - 7] + s1) | 0;
      }
    }
    let [a, b, c, d, e, f, g, h] = hash;
    for (let i = 0; i < 64; i++) {
      const t1 = (h + (rotate(e, 6) ^ rotate(e, 11) ^ rotate(e, 25))
        + ((e & f) ^ (~e & g)) + constants[i] + words[i]) | 0;
      const t2 = ((rotate(a, 2) ^ rotate(a, 13) ^ rotate(a, 22))
        + ((a & b) ^ (a & c) ^ (b & c))) | 0;
      h = g; g = f; f = e; e = (d + t1) | 0;
      d = c; c = b; b = a; a = (t1 + t2) | 0;
    }
    for (const [i, value] of [a, b, c, d, e, f, g, h].entries()) {
      hash[i] = (hash[i] + value) | 0;
    }
  }
  return hash.map(value => (value >>> 0).toString(16).padStart(8, '0')).join('');
}
