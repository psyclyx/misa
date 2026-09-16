// Real browser/daemon gate. Pass packaged web and daemon binaries to verify artifacts.
import { spawn } from 'node:child_process';
import { mkdtempSync, mkdirSync, readFileSync, writeFileSync, openSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join, resolve } from 'node:path';
import assert from 'node:assert/strict';
const work = mkdtempSync(join(tmpdir(), 'misa-web-lifecycle-'));
const env = { ...process.env, XDG_RUNTIME_DIR: join(work, 'runtime'), XDG_STATE_HOME: join(work, 'state'), RUST_LOG: 'info' };
mkdirSync(env.XDG_RUNTIME_DIR, { mode: 0o700 });
const children = [];
function start(name, bin, args) {
  const log = join(work, name + '.log');
  const fd = openSync(log, 'w');
  const child = spawn(bin, args, { env, stdio: ['ignore', fd, fd] });
  children.push(child);
  child.on('error', error => writeFileSync(join(work, name + '.error'), String(error)));
  return log;
}
async function until(check, label, timeout = 30000) {
  const end = Date.now() + timeout;
  while (Date.now() < end) {
    try { const value = await check(); if (value) return value; } catch (_) {}
    await new Promise(resolve => setTimeout(resolve, 100));
  }
  throw new Error('Timed out: ' + label);
}
let socket, diagnostic;
try {
  const web = resolve(process.argv[2] || 'next/target/debug/misa-web');
  const daemon = resolve(process.argv[3] || 'next/target/debug/misa-daemon');
  for (const id of ['one', 'two']) {
    const log = start(id, daemon, ['--no-relay', '--session', 'same-label', '--data-dir', join(work, id)]);
    await until(() => readFileSync(log, 'utf8').includes('misa:'), 'daemon ' + id);
  }
  const log = start('web', web, ['--listen', '127.0.0.1:0']);
  const origin = await until(() => readFileSync(log, 'utf8').match(/http:\/\/127\.0\.0\.1:\d+/)?.[0], 'web listener');
  const profile = join(work, 'chromium');
  start('browser', process.env.CHROMIUM || 'chromium', ['--headless', '--no-sandbox', '--disable-gpu', '--disable-dev-shm-usage', '--remote-debugging-port=0', '--user-data-dir=' + profile, 'about:blank']);
  const port = await until(() => readFileSync(join(profile, 'DevToolsActivePort'), 'utf8').split('\n')[0], 'browser debugger');
  const target = await (await fetch('http://127.0.0.1:' + port + '/json/new?' + encodeURIComponent(origin + '/daemons'), { method: 'PUT' })).json();
  socket = new WebSocket(target.webSocketDebuggerUrl);
  await new Promise((resolve, reject) => { socket.onopen = resolve; socket.onerror = reject; });
  const pending = new Map(); let sequence = 0;
  socket.onmessage = event => { const message = JSON.parse(event.data); const item = pending.get(message.id); if (item) { pending.delete(message.id); message.error ? item.reject(new Error(JSON.stringify(message.error))) : item.resolve(message.result); } };
  function call(method, params = {}) { return new Promise((resolve, reject) => { const id = ++sequence; pending.set(id, { resolve, reject }); socket.send(JSON.stringify({ id, method, params })); }); }
  async function evaluate(expression) {
    const result = await call('Runtime.evaluate', { expression, returnByValue: true, awaitPromise: true });
    if (result.exceptionDetails) throw new Error(JSON.stringify(result.exceptionDetails));
    return result.result.value;
  }
  diagnostic = async () => {
    writeFileSync(join(work, 'failure.html'), await evaluate('document.documentElement.outerHTML'));
    await shot('failure');
  };
  const wait = (expression, label) => until(() => evaluate(expression), label);
  async function navigate(path) { await call('Page.navigate', { url: origin + path }); }
  async function shot(name) { const { data } = await call('Page.captureScreenshot'); writeFileSync(join(work, name + '.png'), Buffer.from(data, 'base64')); }
  await call('Emulation.setDeviceMetricsOverride', { width: 1000, height: 900, deviceScaleFactor: 1, mobile: false });
  await wait('document.querySelectorAll("#overview article").length === 2', 'two locally paired daemons');
  assert.equal(await evaluate('[...document.querySelectorAll("#overview article > form:first-child button")].map(b => b.textContent).join(",")'), 'same-label (scripted),same-label (scripted)');
  await shot('overview');
  const identity = await evaluate('document.querySelector("#overview input[name=daemon]").value');
  await navigate('/daemon/commands?daemon=' + encodeURIComponent(identity));
  await wait('[...document.querySelectorAll("input[name=command]")].some(input => input.value === "daemon.work.forget")', 'installed daemon commands');
  await evaluate('[...document.querySelectorAll("input[name=command]")].find(input => input.value === "operation.cancel").form.requestSubmit(); true');
  await wait('document.querySelector("input[name=\'field.generation\']")', 'generic daemon command form');
  await shot('daemon-command');
  await navigate('/daemons');
  await wait('document.querySelectorAll("#overview article").length === 2', 'unchanged directory after local preparation');
  await evaluate('document.querySelector("form[action=\'/lifecycle\'] select[name=command]").value = "daemon.session.create"; document.querySelector("select[name=command]").form.requestSubmit(); true');
  await wait('document.querySelector("input[name=\'field.id\']")', 'creation form');
  await evaluate('document.querySelector("input[name=\'field.id\']").value="browser-created"; document.querySelector("form[action=\'/lifecycle\']").requestSubmit(); true');
  await wait('document.body.dataset.claimed === "true" && document.querySelector("#composer textarea")', 'created session composer');
  await evaluate('var text=document.querySelector("#composer textarea"); text.value="browser lifecycle history"; text.dispatchEvent(new Event("input",{bubbles:true})); text.form.requestSubmit(); true');
  await wait('document.querySelector("#main").textContent.includes("that is all I have.")', 'streamed reply');
  await evaluate('var text=document.querySelector("#composer textarea"); text.value="retained local draft"; text.dispatchEvent(new Event("input",{bubbles:true})); document.querySelector("#theme").value="dark"; document.querySelector("#theme").dispatchEvent(new Event("change")); true');
  assert.equal(await evaluate('document.documentElement.dataset.theme'), 'dark');
  assert.equal(await evaluate('document.querySelectorAll("form[action=\'./close\']").length'), 1);
  assert.equal(await evaluate('document.querySelectorAll("[data-session-loading]").length'), 0);
  await shot('session');
  const firstView = await evaluate('location.pathname');
  await call('Page.reload');
  await wait('document.body.dataset.claimed === "true" && location.pathname !== ' + JSON.stringify(firstView), 'independent reloaded instance');
  assert.equal(await evaluate('document.querySelector("#composer textarea").value'), 'retained local draft');
  assert.equal(await evaluate('document.documentElement.dataset.theme'), 'dark');
  await navigate('/daemons');
  await wait('[...document.querySelectorAll("#overview article")].some(a=>a.textContent.includes("browser-created"))', 'created directory row');
  await evaluate('[...document.querySelectorAll("#overview article")].find(a=>a.textContent.includes("browser-created")).querySelector("form[action=\'/lifecycle\']").requestSubmit(); true');
  await wait('document.querySelector("input[name=action_id]")?.value === "daemon.session.close"', 'close preparation');
  await evaluate('document.querySelector("form[action=\'/lifecycle\']").requestSubmit(); true');
  await wait('location.pathname === "/daemons" && ![...document.querySelectorAll("#overview article")].some(a=>a.textContent.includes("browser-created"))', 'authoritative close');
  await navigate('/archive?daemon=' + encodeURIComponent(identity));
  await wait('document.querySelector("input[name=conversation]")', 'durable archive');
  await shot('archive');
  await evaluate('document.querySelector("input[name=conversation]").form.requestSubmit(); true');
  await wait('document.querySelector("input[name=action_id]")?.value === "daemon.session.resume"', 'resume form');
  await evaluate('document.querySelector("input[name=\'field.id\']").value="browser-resumed"; document.querySelector("form[action=\'/lifecycle\']").requestSubmit(); true');
  await wait('document.querySelector("#main")?.textContent.includes("browser lifecycle history")', 'resumed history');
  await shot('resumed');
  await call('Emulation.setDeviceMetricsOverride', { width: 375, height: 800, deviceScaleFactor: 1, mobile: true });
  assert.equal(await evaluate('document.documentElement.scrollWidth <= innerWidth'), true);
  await shot('mobile');
  await navigate('/daemons');
  await wait('document.querySelectorAll("#overview article").length === 3', 'mobile multi-daemon overview');
  assert.equal(await evaluate('document.documentElement.scrollWidth <= innerWidth'), true);
  await shot('mobile-overview');
  console.log('Browser lifecycle gate passed; artifacts: ' + work);
} catch (error) {
  try { await diagnostic?.(); } catch (_) {}
  console.error('Browser lifecycle gate failed; artifacts: ' + work);
  throw error;
} finally {
  socket?.close();
  for (const child of children.reverse()) child.kill('SIGTERM');
}
