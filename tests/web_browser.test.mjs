// QUOTE0_CDP_URL=http://127.0.0.1:9222 pnpm test:browser
import test from 'node:test';
import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';
import { createHash } from 'node:crypto';
import { chromium } from 'playwright-core';

const cdp = process.env.QUOTE0_CDP_URL;
test('image conversion, preview, download and canonical upload in a real browser',
  { skip: !cdp && 'Set QUOTE0_CDP_URL to an isolated browser debugging endpoint' }, async () => {
    const browser = await chromium.connectOverCDP(cdp);
    const context = await browser.newContext({ acceptDownloads: true });
    const page = await context.newPage();
    if (process.env.QUOTE0_BROWSER_DOWNLOAD_DIR) {
      const session = await context.newCDPSession(page);
      const { targetInfo } = await session.send('Target.getTargetInfo');
      await session.send('Browser.setDownloadBehavior', {
        behavior: 'allowAndName', downloadPath: process.env.QUOTE0_BROWSER_DOWNLOAD_DIR,
        browserContextId: targetInfo.browserContextId, eventsEnabled: true,
      });
    }
    await page.addInitScript(() => {
      const create = URL.createObjectURL.bind(URL);
      URL.createObjectURL = blob => {
        if (blob.type === 'application/octet-stream') window.downloadedFrame = blob;
        return create(blob);
      };
    });
    const html = await readFile(new URL('../src/web/embedded.html', import.meta.url), 'utf8');
    let config = {
      schema: 1, wifi: { ssid: 'test-network', password: '***' },
      pull: { enabled: false, url: '', interval_seconds: 900, timeout_seconds: 15, bearer_token: '', allow_insecure_http: false },
      display: { status_bar: true },
    };
    const uploads = [], errors = [];
    page.on('pageerror', error => errors.push(error.message));
    await page.route('**/*', async route => {
      const request = route.request(), url = new URL(request.url());
      assert.equal(url.hostname, 'quote0.test', 'UI must not require external requests');
      if (url.pathname === '/') return route.fulfill({ contentType: 'text/html', body: html });
      assert.equal(request.headers()['x-device-token'], 'browser-test-token');
      let value;
      if (url.pathname === '/api/status') {
        value = { device: 'Quote0-test', network: { mode: 'station' }, clock: { time: '12:34' }, battery: { percent: 70 } };
      } else if (url.pathname === '/api/config') {
        if (request.method() === 'POST') config = request.postDataJSON();
        value = { config };
      } else if (url.pathname === '/api/frame') {
        const bytes = request.postDataBuffer();
        assert.equal(bytes.length, 5624);
        assert.equal(request.headers()['x-content-sha256'], createHash('sha256').update(bytes).digest('hex'));
        uploads.push(bytes);
        value = { accepted: true };
      } else throw Error('Unexpected request ' + url.pathname);
      return route.fulfill({ contentType: 'application/json', body: JSON.stringify(value) });
    });
    try {
      await page.goto('http://quote0.test/#token=browser-test-token');
      await page.waitForFunction(() => document.getElementById('message').textContent === 'Connected');
      assert.equal(await page.locator('#downloadFrame').isDisabled(), true);
      const fixture = async (format = 'image/png', transparent = false) => {
        const encoded = await page.evaluate(({ format, transparent }) => {
          const canvas = document.createElement('canvas'); canvas.width = 280; canvas.height = 152;
          const ctx = canvas.getContext('2d');
          if (!transparent) { ctx.fillStyle = '#fff'; ctx.fillRect(0, 0, 280, 152); }
          ctx.fillStyle = '#000'; ctx.fillRect(0, 0, transparent ? 8 : 140, transparent ? 1 : 152);
          return canvas.toDataURL(format).split(',')[1];
        }, { format, transparent });
        return Buffer.from(encoded, 'base64');
      };
      const choose = async (name, mimeType, buffer) => {
        await page.locator('#frame').setInputFiles({ name, mimeType, buffer });
        await page.waitForFunction(() => !document.getElementById('downloadFrame').disabled);
      };
      await choose('half.png', 'image/png', await fixture());
      assert.equal(await page.locator('#statusZone').isVisible(), true);
      const downloadEvent = page.waitForEvent('download');
      await page.locator('#downloadFrame').click();
      const download = await downloadEvent;
      assert.equal(await download.failure(), null);
      const frame = Buffer.from(await page.evaluate(async () => Array.from(new Uint8Array(await window.downloadedFrame.arrayBuffer()))));
      assert.equal(download.suggestedFilename(), 'half-296x152.bin');
      assert.equal(frame.length, 5624);
      for (let y = 0; y < 152; y++) {
        assert(frame.subarray(y * 37, y * 37 + 17).every(byte => byte === 0));
        assert.equal(frame[y * 37 + 17], 0x0f);
        assert(frame.subarray(y * 37 + 18, (y + 1) * 37).every(byte => byte === 255));
      }
      const preview = await page.locator('#framePreview').evaluate(canvas => {
        const data = canvas.getContext('2d').getImageData(0, 0, 296, 152).data;
        return [data[0], data[140 * 4], data[280 * 4]];
      });
      assert.deepEqual(preview, [0, 255, 255]);
      await page.locator('#sendFrame').click();
      await page.waitForFunction(() => document.getElementById('message').textContent.startsWith('Frame accepted'));
      assert.deepEqual(uploads.at(-1), frame);

      await choose('transparent.png', 'image/png', await fixture('image/png', true));
      await page.locator('#sendFrame').click();
      await page.waitForFunction(() => document.getElementById('message').textContent.startsWith('Frame accepted'));
      assert.equal(uploads.at(-1)[0], 0);
      assert(uploads.at(-1).subarray(1).every(byte => byte === 255));

      for (const [name, format] of [['photo.jpg', 'image/jpeg'], ['photo.webp', 'image/webp']]) {
        await choose(name, format, await fixture(format));
        assert.equal(await page.locator('#framePreview').isVisible(), true);
      }
      // Saved configuration determines the actual firmware overlay, not unsaved controls.
      await page.locator('#bar').uncheck();
      await page.locator('#settings button').click();
      await page.waitForFunction(() => document.getElementById('message').textContent.startsWith('Settings saved'));
      assert.equal(await page.locator('#statusZone').isVisible(), false);
      await page.locator('#imageFit').selectOption('crop');
      await page.locator('#dither').check();
      await page.locator('#threshold').evaluate(input => { input.value = '180'; input.dispatchEvent(new Event('input', { bubbles: true })); });
      assert.equal(await page.locator('#thresholdValue').textContent(), '180');
      await page.locator('#sendFrame').click();
      await page.waitForFunction(() => document.getElementById('message').textContent.startsWith('Frame accepted'));
      assert.equal(uploads.at(-1).length, 5624);

      const raw = Buffer.alloc(5624, 0xa5);
      await choose('existing.bin', 'application/octet-stream', raw);
      assert.equal(await page.locator('#imageFit').isDisabled(), true);
      await page.locator('#sendFrame').click();
      await page.waitForFunction(() => document.getElementById('message').textContent.startsWith('Frame accepted'));
      assert.deepEqual(uploads.at(-1), raw);
      await page.locator('#frame').setInputFiles({ name: 'bad.bin', mimeType: 'application/octet-stream', buffer: Buffer.alloc(8) });
      await page.waitForFunction(() => document.getElementById('message').textContent.includes('exactly 5,624'));
      assert.equal(await page.locator('#sendFrame').isDisabled(), true);
      assert.equal(await page.locator('#previewArea').isVisible(), false);
      await page.locator('#frame').setInputFiles({ name: 'broken.png', mimeType: 'image/png', buffer: Buffer.from('not an image') });
      await page.waitForFunction(() => document.getElementById('frameInfo').textContent.includes('No usable frame'));
      assert.equal(await page.locator('#downloadFrame').isDisabled(), true);
      assert.deepEqual(errors, []);
      console.log('PNG/JPEG/WebP, transparency, canonical preview/download/upload, saved layout, raw BIN and decode errors passed.');
    } finally {
      await context.close();
      await browser.close();
    }
  });
