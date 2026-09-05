// Run with: node --test apps/hub-server/tests/ui/member-flows.test.cjs
// Execute the shipped inline scripts; fake only DOM/API boundaries. Browser checks
// cover native focus, disclosure and layout behavior separately.
const { test } = require('node:test');
const assert = require('node:assert/strict');
const { readFileSync } = require('node:fs');
const { resolve } = require('node:path');
const vm = require('node:vm');

function element() {
  const classes = new Set(['hidden']);
  const queries = new Map();
  return {
    value: '', textContent: '', dataset: {}, style: {}, children: [], open: false,
    classList: { add: x => classes.add(x), remove: x => classes.delete(x),
      contains: x => classes.has(x), toggle: (x, yes) => yes ? classes.add(x) : classes.delete(x) },
    set innerHTML(value) { this.html = value; this.children = []; queries.clear(); },
    get innerHTML() { return this.html || ''; },
    querySelector: key => { if (!queries.has(key)) queries.set(key, element()); return queries.get(key); },
    querySelectorAll: () => [],
    appendChild(child) { this.children.push(child); },
    closest() { return this; },
    insertBefore(child, before) { this.children = this.children.filter(c => c !== child); const i = before ? this.children.indexOf(before) : this.children.length; this.children.splice(i, 0, child); },
    addEventListener() {}, focus() {}, removeAttribute(key) { delete this[key]; },
  };
}
function page(name) {
  const nodes = new Map();
  const document = { getElementById(id) { if (!nodes.has(id)) nodes.set(id, element()); return nodes.get(id); },
    createElement: element, querySelectorAll: () => [], activeElement: null };
  const context = vm.createContext({ document, console, URLSearchParams,
    location: { search: '', pathname: '/' }, history: { replaceState() {} },
    localStorage: { getItem: () => '' }, navigator: {},
    I18N: { t: key => key, apply() {}, switcher() {} },
    setTimeout() {}, setInterval() { return 1; }, clearInterval() {},
    fetch: async () => ({ ok: false, status: 404, json: async () => ({ error: 'UNKNOWN_ENTRY' }) }),
  });
  const html = readFileSync(resolve(__dirname, '../../static', name + '.html'), 'utf8');
  const script = html.match(/<script>\s*([\s\S]*?)<\/script>/)[1];
  vm.runInContext(script.split('I18N.apply();')[0], context);
  return { context, document, run: code => vm.runInContext(code, context) };
}

test('unknown entry never exposes a success pass and leaves lookup available', async () => {
  const p = page('signup');
  p.context.location.search = '?code=ZZZZZZ';
  // Same bootstrap path as reopening a saved URL.
  const html = readFileSync(resolve(__dirname, '../../static/signup.html'), 'utf8');
  await p.run(html.match(/const asked =[\s\S]*?(?=<\/script>)/)[0]);
  await new Promise(resolve => setImmediate(resolve));
  assert.equal(p.document.getElementById('view-pass').classList.contains('hidden'), true);
  assert.equal(p.document.getElementById('view-form').classList.contains('hidden'), false);
});

test('ready and active entrants are not labelled DNF; completed times stay visible', () => {
  const p = page('result');
  const row = { athlete_id: 'A', name: 'Alice', bib: 1, splits: [], stations_completed: 0,
    elapsed_ms: 60000, finished_at: null, status: 'READY' };
  p.context.row = row;
  assert.match(p.run('card(row).innerHTML'), /result.notStarted/);
  row.status = 'ACTIVE';
  assert.match(p.run('card(row).innerHTML'), /result.inProgress/);
  row.status = 'FINISHED'; row.finished_at = 100000;
  assert.match(p.run('card(row).innerHTML'), /01:00.0/);
});

test('a selected entrant survives new bands and repeated pending-list refreshes', () => {
  const p = page('checkin');
  p.run('VIEW = { athletes: [{athlete_id:"ABC123", name:"Alice", bib:1}], pending: [] }; findByCode("ABC123")');
  p.run('VIEW.pending = ["band1"]; renderPending()');
  const selected = () => p.document.getElementById('pending').children[0].querySelector('.who').value;
  assert.equal(selected(), 'ABC123');
  p.run('renderPending()');
  assert.equal(selected(), 'ABC123');
});

test('an invalid lookup clears the previous entrant selection', () => {
  const p = page('checkin');
  p.run('VIEW = { athletes: [{athlete_id:"ABC123", name:"Alice", bib:1}], pending: [] }; findByCode("ABC123"); findByCode("ZZZZZZ"); VIEW.pending = ["band1"]; renderPending()');
  assert.equal(p.document.getElementById('pending').children[0].querySelector('.who').value, '');
});

test('result polling retains the expanded element and focused control', async () => {
  const p = page('result');
  const row = { athlete_id: 'A', name: 'Alice', bib: 1, splits: [], stations_completed: 0,
    elapsed_ms: 60000, finished_at: null, status: 'ACTIVE' };
  p.context.fetch = async () => ({ ok: true, json: async () => ({ results: {
    status: 'RUNNING', session_name: 'Test', ordering: 'BIB', rows: [row],
  } }) });
  await p.run('load()');
  const card = p.document.getElementById('rows').children[0];
  card.open = true;
  const focused = card.querySelector('.btn-share');
  p.document.activeElement = focused;
  row.stations_completed = 1;
  await p.run('load()');
  const updated = p.document.getElementById('rows').children[0];
  assert.equal(updated, card);
  assert.equal(updated.open, true);
  assert.equal(updated.querySelector('.btn-share'), focused);
});

test('manual band choice survives polling and cannot reuse a bound athlete', () => {
  const p = page('checkin');
  p.run('VIEW = { athletes: [{athlete_id:"ABC123", name:"Alice", bib:1}, {athlete_id:"DEF456", name:"Bob", bib:2}], pending: ["band1"] }; findByCode("ABC123"); renderPending()');
  let select = p.document.getElementById('pending').children[0].querySelector('.who');
  select.value = 'DEF456'; select.onchange();
  p.run('renderPending()');
  select = p.document.getElementById('pending').children[0].querySelector('.who');
  assert.equal(select.value, 'DEF456');
  p.run('VIEW.athletes[1].tag_id = "another-band"; renderPending()');
  assert.equal(p.document.getElementById('pending').children[0].querySelector('.who').value, '');
});

test('a verified pass survives a network failure with a stale-data notice', async () => {
  const p = page('signup');
  p.context.fetch = async () => ({ ok: true, json: async () => ({ code: 'ABC123', row: {
    name: 'Alice', bib: 1, status: 'READY', stations_completed: 0,
  }, course_length: 16 }) });
  await p.run('openPass("ABC123")');
  assert.equal(p.document.getElementById('view-pass').classList.contains('hidden'), false);
  p.context.fetch = async () => { throw new Error('offline'); };
  await p.run('refresh()');
  assert.equal(p.document.getElementById('view-pass').classList.contains('hidden'), false);
  assert.equal(p.document.getElementById('lookup-status').textContent, 'signup.stale');
});

test('completed and cancelled sessions have explicit non-finisher labels', () => {
  const p = page('result');
  assert.equal(p.run('RESULT_STATUS = "COMPLETED"; resultTime({status:"READY"})'), 'result.didNotStart');
  assert.equal(p.run('resultTime({status:"ACTIVE"})'), 'result.dnf');
  assert.equal(p.run('RESULT_STATUS = "CANCELLED"; resultTime({status:"ACTIVE"})'), 'result.cancelled');
});
