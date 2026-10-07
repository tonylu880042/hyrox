// node --test apps/hub-server/tests/ui/course.test.cjs
// Runs the shipped page's own derivations; only the DOM and the socket are left out.
const { test } = require('node:test');
const assert = require('node:assert/strict');
const fs = require('node:fs');
const vm = require('node:vm');

function page() {
  const html = fs.readFileSync(require('node:path').resolve(__dirname, '../../static/course.html'), 'utf8');
  const script = html.match(/<script>\s*([\s\S]*?)<\/script>/)[1];
  const c = vm.createContext({ I18N: { t: (key, ...args) => [key, ...args].join('|') } });
  vm.runInContext(script.split('// ── Rendering')[0], c);
  return (code) => vm.runInContext(code, c);
}

const course = [{ name: 'SKIERG' }, { name: 'SLED_PUSH' }, { name: 'WALL_BALLS' }];
const athlete = (bib, extra) => ({ bib, name: 'A' + bib, status: 'ACTIVE', station_state: 'OUTSIDE',
  station_index: null, completed: 0, last_event_age_ms: 1000, splits: [], ...extra });

test('athletes are drawn where the snapshot says they are, runs numbered by the station they lead to', () => {
  const p = page();
  p(`snap = { course: ${JSON.stringify(course)}, athletes: [
    ${JSON.stringify(athlete(1, { status: 'READY' }))},
    ${JSON.stringify(athlete(2))},
    ${JSON.stringify(athlete(3, { station_state: 'INSIDE', station_index: 1, completed: 1 }))},
    ${JSON.stringify(athlete(4, { completed: 2 }))},
    ${JSON.stringify(athlete(5, { status: 'FINISHED', completed: 3 }))},
    ${JSON.stringify(athlete(6, { completed: 3 }))} ] }; placed = place(snap, 300000);`);
  assert.deepEqual(JSON.parse(p('JSON.stringify(placed.waiting.map(a => a.bib))')), [1]);
  assert.deepEqual(JSON.parse(p('JSON.stringify(placed.runs.map(r => r.map(a => a.bib)))')), [[2], [], [4]]);
  assert.deepEqual(JSON.parse(p('JSON.stringify(placed.inside.map(r => r.map(a => a.bib)))')), [[], [3], []]);
  assert.deepEqual(JSON.parse(p('JSON.stringify(placed.finished.map(a => a.bib))')), [5]);
  assert.deepEqual(JSON.parse(p('JSON.stringify(placed.toFinish.map(a => a.bib))')), [6]);
});

test('only an active athlete past the threshold is stale; never-read is not stale', () => {
  const p = page();
  p(`snap = { course: ${JSON.stringify(course)}, athletes: [
    ${JSON.stringify(athlete(1, { last_event_age_ms: 300000 }))},
    ${JSON.stringify(athlete(2, { last_event_age_ms: 299999 }))},
    ${JSON.stringify(athlete(3, { status: 'FINISHED', completed: 3, last_event_age_ms: 900000 }))},
    ${JSON.stringify(athlete(4, { last_event_age_ms: null }))} ] }; placed = place(snap, 300000);`);
  assert.deepEqual(JSON.parse(p('JSON.stringify([...placed.runs.flat(), ...placed.finished].filter(a => a.stale).map(a => a.bib))')), [1]);
});

test('gates come from the reader map, and the last N gate can be the start/finish mat', () => {
  const p = page();
  const readers = [
    { station: 'SKIERG', mode: 'ENTRY' }, { station: 'skierg', mode: 'EXIT' },
    { station: 'SLED_PUSH', mode: 'TOGGLE' },
    { station: 'WALL_BALLS', mode: 'ENTRY' }, { station: 'WALL_BALLS', mode: 'START_FINISH' },
  ];
  p(`g = gates(${JSON.stringify(course)}, ${JSON.stringify(readers)})`);
  assert.equal(p('g.mat.length'), 1);
  assert.equal(p('g.stations[0].s.length + "/" + g.stations[0].n.length'), '1/1');
  assert.equal(p('g.stations[1].s.length + "/" + g.stations[1].n.length'), '1/1', 'a toggle serves both');
  assert.equal(p('g.stations[2].nIsMat'), true);
  assert.equal(p('g.stations[0].nIsMat'), false);
});

test('a gate light is the worst of its readers, and unheard is never green', () => {
  const p = page();
  assert.equal(p('health([])'), 'none');
  assert.equal(p('health([{ last_seen_age_ms: 1000 }])'), 'ok');
  assert.equal(p('health([{ last_seen_age_ms: 1000, warning: "JOURNAL_NEAR_FULL" }])'), 'warn');
  assert.equal(p('health([{ last_seen_age_ms: 1000 }, { last_seen_age_ms: null }])'), 'bad');
  assert.equal(p('health([{ last_seen_age_ms: 60001 }])'), 'bad');
});

test('missed exits are counted against the gate that missed them', () => {
  const p = page();
  p(`snap = { course: ${JSON.stringify(course)}, athletes: [
    ${JSON.stringify(athlete(1, { splits: [{ exit_missed: true }, { exit_missed: false }] }))},
    ${JSON.stringify(athlete(2, { splits: [{ exit_missed: true }] }))} ] }`);
  assert.deepEqual(JSON.parse(p('JSON.stringify(missedAt(snap))')), [2, 0, 0]);
});

test('the layout snakes two stations per row like the venue drawing, with every run placed once', () => {
  const p = page();
  const cells = JSON.parse(p('JSON.stringify(layout(8))'));
  const stations = cells.filter((c) => c.kind === 'station');
  const runs = cells.filter((c) => c.kind === 'run').map((c) => c.i).sort((a, b) => a - b);
  assert.deepEqual(runs, [0, 1, 2, 3, 4, 5, 6, 7]);
  const at = (i) => stations.find((c) => c.i === i);
  assert.deepEqual([at(0).col, at(1).col, at(2).col, at(3).col], [3, 5, 5, 3]);
  assert.equal(at(2).ltr, false, 'the second row runs back');
  const down = cells.find((c) => c.kind === 'run' && c.i === 2);
  assert.equal(down.vertical, true);
  assert.equal(down.col, 5, 'run 3 drops on the side station 2 ends');
  assert.equal(JSON.parse(p('JSON.stringify(layout(3))')).filter((c) => c.kind === 'run').length, 3);
});

test('alerts name silent readers, stale athletes and missed exits', () => {
  const p = page();
  const readers = [
    { station: 'SKIERG', mode: 'ENTRY', last_seen_age_ms: null },
    { station: 'SKIERG', mode: 'EXIT', last_seen_age_ms: 120000 },
    { station: 'WALL_BALLS', mode: 'START_FINISH', last_seen_age_ms: 1000 },
  ];
  p(`snap = { course: ${JSON.stringify(course)}, athletes: [
    ${JSON.stringify(athlete(7, { last_event_age_ms: 360000, splits: [{ station: 'SKIERG', exit_missed: true }], completed: 1 }))} ] };
    list = alerts(snap, ${JSON.stringify(readers)}, place(snap, 300000), 300000);`);
  const texts = JSON.parse(p('JSON.stringify(list.map(a => a.level + " " + a.text))'));
  assert.ok(texts.includes('bad course.readerUnheard|2S'), texts.join('\n'));
  assert.ok(texts.includes('bad course.readerDown|2N'), texts.join('\n'));
  assert.ok(texts.includes('warn course.stale|7|A7|6'), texts.join('\n'));
  assert.ok(texts.includes('warn course.exitMissed|7|A7|SKIERG'), texts.join('\n'));
  assert.equal(texts.length, 4, 'the healthy mat raises nothing');
});

test('a whole venue of silent readers is one line, not one per gate', () => {
  const p = page();
  const readers = ['SKIERG', 'SLED_PUSH', 'WALL_BALLS'].flatMap((station) => [
    { station, mode: 'ENTRY', last_seen_age_ms: 120000 }, { station, mode: 'EXIT', last_seen_age_ms: 120000 }]);
  p(`snap = { course: ${JSON.stringify(course)}, athletes: [] };
    list = alerts(snap, ${JSON.stringify(readers)}, place(snap, 300000), 300000);`);
  assert.deepEqual(JSON.parse(p('JSON.stringify(list.map(a => a.text))')), ['course.readerDown|2S 2N 3S 3N 4S 4N']);
});
