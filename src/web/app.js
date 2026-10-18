import { sha256 } from './sha256.js';

const $ = id => document.getElementById(id);
let config = null;
let busy = false;
const fragment = new URLSearchParams(location.hash.slice(1));
$('access').value = fragment.get('token') || sessionStorage.getItem('quote0-token') || '';
if (location.hash) history.replaceState(null, '', location.pathname);

function message(text) {
  $('message').textContent = text;
  $('message').style.display = 'block';
}
async function api(path, method = 'GET', body, extra = {}) {
  const response = await fetch('/api/' + path, {
    method, headers: { 'X-Device-Token': $('access').value, ...extra }, body,
  });
  const result = await response.json();
  if (!response.ok) throw Error(result.error || 'Request failed');
  return result;
}
function show(status) {
  $('summary').textContent = `${status.device} · ${status.network.mode} · `
    + `${status.clock.time} (UTC+8) · Battery ${status.battery.percent ?? '?'}%`;
  $('status').textContent = JSON.stringify(status, null, 2);
}
async function refresh() { show(await api('status')); }
async function load() {
  sessionStorage.setItem('quote0-token', $('access').value);
  await refresh();
  config = (await api('config')).config;
  for (const [id, value] of Object.entries({
    ssid: config.wifi.ssid, password: config.wifi.password,
    url: config.pull.url, interval: config.pull.interval_seconds,
    timeout: config.pull.timeout_seconds, bearer: config.pull.bearer_token,
  })) $(id).value = value;
  $('enabled').checked = config.pull.enabled;
  $('insecure').checked = config.pull.allow_insecure_http;
  $('bar').checked = config.display.status_bar;
  message('Connected');
}
async function run(action) {
  if (busy) return;
  busy = true;
  document.querySelectorAll('button').forEach(button => button.disabled = true);
  try { await action(); }
  catch (error) { message(error.message); }
  finally {
    busy = false;
    document.querySelectorAll('button').forEach(button => button.disabled = false);
  }
}
$('connect').onclick = () => run(load);
$('refresh').onclick = () => run(refresh);
$('pull').onclick = () => run(async () => {
  await api('pull', 'POST');
  message('Pull scheduled');
});
$('reboot').onclick = () => run(async () => {
  await api('reboot', 'POST');
  message('Restarting. Reconnect in a few seconds.');
});
$('settings').onsubmit = event => {
  event.preventDefault();
  run(async () => {
    if (!config) throw Error('Connect first');
    const next = JSON.parse(JSON.stringify(config));
    next.wifi = { ssid: $('ssid').value, password: $('password').value };
    next.pull = {
      ...next.pull, enabled: $('enabled').checked, url: $('url').value,
      interval_seconds: Number($('interval').value),
      timeout_seconds: Number($('timeout').value),
      bearer_token: $('bearer').value, allow_insecure_http: $('insecure').checked,
    };
    next.display.status_bar = $('bar').checked;
    await api('config', 'POST', JSON.stringify(next), { 'Content-Type': 'application/json' });
    config = next;
    message('Settings saved. Reconnect if the Wi-Fi address changes.');
  });
};
async function upload(id, path) {
  const file = $(id).files[0];
  if (!file) throw Error('Choose a .bin file');
  if (id === 'frame' && file.size !== 5624) throw Error('Frame must be exactly 5,624 bytes');
  if (id === 'firmware' && (file.size === 0 || file.size > 1966080)) {
    throw Error('Firmware image is too large or empty');
  }
  message('Uploading…');
  const data = await file.arrayBuffer();
  await api(path, 'PUT', data, {
    'Content-Type': 'application/octet-stream', 'X-Content-SHA256': sha256(data),
  });
  message(id === 'firmware' ? 'Update verified. Restarting…' : 'Frame accepted');
  if (id === 'frame') await refresh();
}
$('sendFrame').onclick = () => run(() => upload('frame', 'frame'));
$('sendFirmware').onclick = () => run(() => upload('firmware', 'firmware'));
if ($('access').value) run(load);
