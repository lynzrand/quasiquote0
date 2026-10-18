import test from 'node:test';
import assert from 'node:assert/strict';
import { WIDTH, HEIGHT, ROW_BYTES, FRAME_BYTES, imagePlacement, rgbaToFrame, frameToRgba } from '../src/web/frame-converter.js';
const solid = (gray, alpha = 255) => {
  const data = new Uint8ClampedArray(WIDTH * HEIGHT * 4);
  for (let i = 0; i < data.length; i += 4) data.set([gray, gray, gray, alpha], i);
  return data;
};

test('wire bytes match firmware: landscape, MSB first, white is one', () => {
  const data = solid(255);
  for (const [x, y] of [[0, 0], [7, 0], [8, 0], [295, 151]]) {
    data.set([0, 0, 0, 255], (y * WIDTH + x) * 4);
  }
  const frame = rgbaToFrame(data, { statusBar: false });
  assert.equal(frame.length, 5624);
  assert.equal(frame[0], 0x7e);
  assert.equal(frame[1], 0x7f);
  assert.equal(frame[ROW_BYTES], 255);
  assert.equal(frame[FRAME_BYTES - 1], 0xfe);
  assert.deepEqual(frameToRgba(frame), data);
});

test('reserved status columns stay white in threshold and diffusion modes', () => {
  for (const dither of [false, true]) {
    const frame = rgbaToFrame(solid(0), { statusBar: true, dither });
    for (let row = 0; row < HEIGHT; row++) {
      assert(frame.slice(row * ROW_BYTES, row * ROW_BYTES + 35).every(value => value === 0));
      assert.deepEqual(frame.slice(row * ROW_BYTES + 35, (row + 1) * ROW_BYTES), new Uint8Array([255, 255]));
    }
    assert(rgbaToFrame(solid(0), { statusBar: false, dither }).every(value => value === 0));
  }
});

test('transparent pixels composite onto white before thresholding', () => {
  assert(rgbaToFrame(solid(0, 0), { statusBar: false }).every(value => value === 255));
  assert(rgbaToFrame(solid(0, 128), { statusBar: false, threshold: 127 }).every(value => value === 255));
  assert(rgbaToFrame(solid(0, 128), { statusBar: false, threshold: 128 }).every(value => value === 0));
});

test('threshold endpoints and equality are deterministic', () => {
  assert(rgbaToFrame(solid(0), { statusBar: false, threshold: 0 }).every(value => value === 255));
  assert(rgbaToFrame(solid(255), { statusBar: false, threshold: 255 }).every(value => value === 255));
  assert(rgbaToFrame(solid(128), { statusBar: false, threshold: 128 }).every(value => value === 255));
});

test('diffusion retains gray coverage and does not write into status area', () => {
  const frame = rgbaToFrame(solid(128), { dither: true, statusBar: true });
  const pixels = frameToRgba(frame);
  let white = 0;
  for (let y = 0; y < HEIGHT; y++) {
    for (let x = 0; x < 280; x++) white += pixels[(y * WIDTH + x) * 4] === 255;
    assert.equal(frame[y * ROW_BYTES + 35], 255);
    assert.equal(frame[y * ROW_BYTES + 36], 255);
  }
  assert(Math.abs(white / (280 * HEIGHT) - 128 / 255) < 0.02);
});

test('fit adds margins and crop centers the overflow using content dimensions', () => {
  assert.deepEqual(imagePlacement(560, 152, 280, 'fit'), { x: 0, y: 38, width: 280, height: 76 });
  assert.deepEqual(imagePlacement(560, 152, 280, 'crop'), { x: -140, y: 0, width: 560, height: 152 });
  const portrait = imagePlacement(100, 200, 296, 'crop');
  assert.equal(portrait.width, 296);
  assert.equal(portrait.y, -220);
});

test('invalid raster/frame lengths and threshold are rejected', () => {
  assert.throws(() => rgbaToFrame(new Uint8Array(4)), /296/);
  assert.throws(() => rgbaToFrame(solid(0), { threshold: NaN }), /Threshold/);
  assert.throws(() => frameToRgba(new Uint8Array(5623)), /5,624/);
  assert.throws(() => imagePlacement(0, 152, 280), /dimensions/);
});
