// Canonical wire format: landscape, row-major, MSB first, 1 = white.
export const WIDTH = 296;
export const HEIGHT = 152;
export const ROW_BYTES = WIDTH / 8;
export const FRAME_BYTES = ROW_BYTES * HEIGHT;
export const STATUS_WIDTH = 16;

export function imagePlacement(sourceWidth, sourceHeight, contentWidth, mode = 'fit') {
  if (!(sourceWidth > 0 && sourceHeight > 0) || !['fit', 'crop'].includes(mode)) {
    throw Error('Invalid image dimensions or fit mode');
  }
  const scale = (mode === 'crop' ? Math.max : Math.min)(
    contentWidth / sourceWidth, HEIGHT / sourceHeight,
  );
  const width = sourceWidth * scale, height = sourceHeight * scale;
  return { x: (contentWidth - width) / 2, y: (HEIGHT - height) / 2, width, height };
}

export function rgbaToFrame(rgba, { statusBar = true, threshold = 128, dither = false } = {}) {
  if (rgba.length !== WIDTH * HEIGHT * 4) throw Error('Expected a 296 × 152 image');
  if (!Number.isFinite(threshold) || threshold < 0 || threshold > 255) {
    throw Error('Threshold must be between 0 and 255');
  }
  const contentWidth = WIDTH - (statusBar ? STATUS_WIDTH : 0);
  const levels = new Float32Array(contentWidth * HEIGHT);
  for (let y = 0; y < HEIGHT; y++) {
    for (let x = 0; x < contentWidth; x++) {
      const index = (y * WIDTH + x) * 4;
      const alpha = rgba[index + 3] / 255;
      const gray = (299 * rgba[index] + 587 * rgba[index + 1] + 114 * rgba[index + 2]) / 1000;
      levels[y * contentWidth + x] = gray * alpha + 255 * (1 - alpha);
    }
  }
  const frame = new Uint8Array(FRAME_BYTES).fill(255);
  for (let y = 0; y < HEIGHT; y++) {
    for (let x = 0; x < contentWidth; x++) {
      const index = y * contentWidth + x;
      const value = Math.max(0, Math.min(255, levels[index]));
      const white = value >= threshold;
      if (!white) frame[y * ROW_BYTES + (x >> 3)] &= ~(0x80 >> (x & 7));
      if (!dither) continue;
      // Floyd–Steinberg diffusion is limited to content: no row wrapping or
      // spill into the status area that the firmware will replace.
      const error = value - (white ? 255 : 0);
      if (x + 1 < contentWidth) levels[index + 1] += error * 7 / 16;
      if (y + 1 < HEIGHT) {
        if (x > 0) levels[index + contentWidth - 1] += error * 3 / 16;
        levels[index + contentWidth] += error * 5 / 16;
        if (x + 1 < contentWidth) levels[index + contentWidth + 1] += error / 16;
      }
    }
  }
  return frame;
}

export function frameToRgba(frame) {
  if (frame.length !== FRAME_BYTES) throw Error('Frame must be exactly 5,624 bytes');
  const rgba = new Uint8ClampedArray(WIDTH * HEIGHT * 4);
  for (let y = 0; y < HEIGHT; y++) {
    for (let x = 0; x < WIDTH; x++) {
      const value = frame[y * ROW_BYTES + (x >> 3)] & (0x80 >> (x & 7)) ? 255 : 0;
      const index = (y * WIDTH + x) * 4;
      rgba[index] = rgba[index + 1] = rgba[index + 2] = value;
      rgba[index + 3] = 255;
    }
  }
  return rgba;
}
