import { WIDTH, HEIGHT, FRAME_BYTES, STATUS_WIDTH, imagePlacement, rgbaToFrame, frameToRgba } from './frame-converter.js';
const $ = id => document.getElementById(id);

export class FrameEditor {
  constructor({ message, statusBar }) {
    this.message = message;
    this.statusBar = statusBar;
    this.version = 0;
    this.busy = false;
    this.loading = false;
    this.image = null;
    this.raw = null;
    this.bytes = null;
    $('frame').onchange = () => this.load($('frame').files[0]);
    for (const id of ['imageFit', 'threshold', 'dither']) {
      $(id).addEventListener('input', () => this.render());
    }
    $('downloadFrame').onclick = () => this.download();
    this.updateControls();
  }
  setBusy(busy) { this.busy = busy; this.updateControls(); }
  updateControls() {
    for (const id of ['imageFit', 'threshold', 'dither']) {
      $(id).disabled = this.busy || this.loading || !this.image;
    }
    for (const id of ['sendFrame', 'downloadFrame']) {
      $(id).disabled = this.busy || this.loading || !this.bytes;
    }
  }
  async load(file) {
    const version = ++this.version;
    this.image = this.raw = this.bytes = null;
    this.loading = Boolean(file);
    $('previewArea').hidden = true;
    $('frameInfo').textContent = file ? 'Preparing preview…' : 'No image selected.';
    this.updateControls();
    if (!file) return;
    this.filename = file.name;
    try {
      if (/\.bin$/i.test(file.name)) {
        if (file.size !== FRAME_BYTES) throw Error('Frame must be exactly 5,624 bytes');
        const data = new Uint8Array(await file.arrayBuffer());
        if (version !== this.version) return;
        this.raw = data;
      } else {
        if (!/\.(png|jpe?g|webp)$/i.test(file.name)
          && !['image/png', 'image/jpeg', 'image/webp'].includes(file.type)) {
          throw Error('Choose a PNG, JPEG, WebP, or frame .bin file');
        }
        if (file.size > 32 * 1024 * 1024) throw Error('Choose an image smaller than 32 MiB');
        const url = URL.createObjectURL(file);
        try {
          const image = new Image();
          image.src = url;
          await image.decode();
          if (version !== this.version) return;
          if (image.naturalWidth * image.naturalHeight > 64_000_000) {
            throw Error('Choose an image smaller than 64 megapixels');
          }
          this.image = image;
        } finally { URL.revokeObjectURL(url); }
      }
      this.loading = false;
      this.render();
    } catch (error) {
      if (version !== this.version) return;
      this.image = this.raw = this.bytes = null;
      $('frameInfo').textContent = 'No usable frame. Choose another file.';
      this.message(error.message || 'This browser could not decode the image');
    } finally {
      if (version === this.version) { this.loading = false; this.updateControls(); }
    }
  }
  render() {
    $('thresholdValue').textContent = $('threshold').value;
    if (this.loading || (!this.image && !this.raw)) return;
    const statusBar = this.statusBar();
    let bytes = this.raw;
    if (this.image) {
      const canvas = document.createElement('canvas');
      canvas.width = WIDTH; canvas.height = HEIGHT;
      const context = canvas.getContext('2d');
      context.fillStyle = '#fff';
      context.fillRect(0, 0, WIDTH, HEIGHT);
      const contentWidth = WIDTH - (statusBar ? STATUS_WIDTH : 0);
      const placement = imagePlacement(this.image.naturalWidth, this.image.naturalHeight,
        contentWidth, $('imageFit').value);
      context.save();
      context.beginPath(); context.rect(0, 0, contentWidth, HEIGHT); context.clip();
      context.imageSmoothingEnabled = true;
      context.imageSmoothingQuality = 'high';
      context.drawImage(this.image, placement.x, placement.y, placement.width, placement.height);
      context.restore();
      bytes = rgbaToFrame(context.getImageData(0, 0, WIDTH, HEIGHT).data, {
        statusBar, threshold: Number($('threshold').value), dither: $('dither').checked,
      });
    }
    const preview = $('framePreview').getContext('2d');
    preview.putImageData(new ImageData(frameToRgba(bytes), WIDTH, HEIGHT), 0, 0);
    this.bytes = bytes;
    $('statusZone').hidden = !statusBar;
    $('layoutNote').textContent = statusBar
      ? 'The shaded 16-column strip is reserved for the device status bar. Image content uses 280 × 152 pixels.'
      : 'The saved status bar is off. Image content uses the full 296 × 152 pixels.';
    const source = this.image ? `${this.image.naturalWidth} × ${this.image.naturalHeight} → ` : '';
    $('frameInfo').textContent = `${this.filename} · ${source}296 × 152 · 5,624 bytes`;
    $('previewArea').hidden = false;
    this.updateControls();
  }
  download() {
    if (!this.bytes || this.busy || this.loading) return;
    const url = URL.createObjectURL(new Blob([this.bytes], { type: 'application/octet-stream' }));
    const link = document.createElement('a');
    link.href = url;
    link.download = this.filename.replace(/\.[^.]*$/, '') + '-296x152.bin';
    link.click();
    setTimeout(() => URL.revokeObjectURL(url), 1000);
  }
}
