import assert from 'node:assert/strict';
import { test } from 'node:test';
import { appendPoint, rateBetween, linePath, formatBytes } from '../src/traffic.ts';
const baseline = { attachment: 'first', sampled_ms: 1000, elapsed_ms: 1000, sent_bytes: '9007199254740993000', received_bytes: '0' };
test('rates preserve small byte deltas above JS integer precision and use real elapsed time', () => {
  const next = { ...baseline, sampled_ms: 2500, sent_bytes: '9007199254740993003', received_bytes: '6000' };
  assert.deepEqual(rateBetween(baseline, next), { up: 2, down: 4000 });
  assert.equal(formatBytes(0.2), '0.2 B');
});
test('missing, stale, reattached, backwards clock and reset counters produce gaps instead of spikes', () => {
  for (const next of [
    { ...baseline, attachment: 'new' },
    { ...baseline, sampled_ms: 1000 },
    { ...baseline, sampled_ms: 0 },
    { ...baseline, sampled_ms: 6001 },
    { ...baseline, sampled_ms: 2000, sent_bytes: '0' },
  ]) assert.equal(rateBetween(baseline, next), null);
  assert.equal(rateBetween(null, baseline), null);
});
test('history is bounded by age and count; paths never bridge missing measurements', () => {
  let points = [];
  for (let i = 0; i < 1500; i++) points = appendPoint(points, { at: i * 1000, time: '', rate: { up: 1, down: 2 } });
  assert.equal(points.length, 300);
  assert.equal(points[0].at, 1200000);
  const gap = { ...points[1], rate: null };
  const path = linePath([points[0], gap, points[2], points[3]], 'up', points[0].at, 60000, 2);
  assert.equal((path.match(/M/g) ?? []).length, 2);
  assert.equal((path.match(/L/g) ?? []).length, 1);
  assert(!path.includes('NaN'));
  for (let i = 0; i < 1000; i++) points = appendPoint(points, { at: 1500000 + i, time: '', rate: null });
  assert.equal(points.length, 301);
});
