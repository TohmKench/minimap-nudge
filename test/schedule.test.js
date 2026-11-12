'use strict';
// Runs the shared vectors against the JS copy used by the settings page.
const test = require('node:test');
const assert = require('node:assert');
const S = require('../src/schedule.js');
const v = require('../tests/schedule-vectors.json');

for (const c of v.alertTimes) {
  test(`alertTimes: ${c.name}`, () => assert.deepStrictEqual([...S.alertTimes(c.phases, c.until)], c.expected));
}
for (const c of v.nextAlertAfter) {
  test(`nextAlertAfter(${c.t})`, () => assert.strictEqual(S.nextAlertAfter(c.phases, c.t), c.expected));
}
test('formatClock', () => {
  assert.strictEqual(S.formatClock(605.9), '10:05');
  assert.strictEqual(S.formatClock(null), '--:--');
});
