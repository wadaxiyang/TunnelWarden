import type { TrafficSample } from './types';

export interface Rate { up: number; down: number }
export interface TrafficPoint { at: number; time: string; rate: Rate | null }
// Subtract exact integer counters before conversion to display units.
export function rateBetween(previous: TrafficSample | null, next: TrafficSample): Rate | null {
  if (!previous || previous.attachment !== next.attachment) return null;
  const seconds = (next.sampled_ms - previous.sampled_ms) / 1000;
  if (seconds <= 0 || seconds > 5) return null;
  const sent = BigInt(next.sent_bytes) - BigInt(previous.sent_bytes);
  const received = BigInt(next.received_bytes) - BigInt(previous.received_bytes);
  if (sent < 0n || received < 0n) return null;
  return { up: Number(sent) / seconds, down: Number(received) / seconds };
}
export function appendPoint(points: TrafficPoint[], point: TrafficPoint): TrafficPoint[] {
  return [...points.filter(p => p.at > point.at - 300_000), point].slice(-301);
}
export function formatBytes(bytes: number | string): string {
  const n = Number(bytes);
  const scale = n <= 0 ? 0 : Math.max(0, Math.min(4, Math.floor(Math.log(n) / Math.log(1024))));
  return `${(n / 1024 ** scale).toLocaleString(undefined, { maximumFractionDigits: 2 })} ${['B', 'KiB', 'MiB', 'GiB', 'TiB'][scale]}`;
}
export const formatRate = (rate: number | undefined) => rate === undefined ? '—' : `${formatBytes(rate)}/s`;
export function linePath(points: TrafficPoint[], direction: keyof Rate, start: number, span: number, max: number): string {
  let connected = false;
  return points.map(p => {
    if (!p.rate) { connected = false; return ''; }
    const command = connected ? 'L' : 'M'; connected = true;
    return `${command}${(60 + (p.at - start) / span * 640).toFixed(2)},${(174 - p.rate[direction] / max * 150).toFixed(2)}`;
  }).join(' ');
}
