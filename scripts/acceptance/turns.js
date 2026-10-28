#!/usr/bin/env node
// 翻页误差统计（票 16）：把 timeline JSON 的期望翻页点与 ffmpeg scene 检测的实测点对比。
// 用法: node turns.js <timeline.json> <detected-pts.txt> [max-ms=120]
// detected-pts.txt：每行一个 pts_time（秒），由 ffmpeg select=gt(scene,..),showinfo 提取。
const fs = require("fs");

const tl = JSON.parse(fs.readFileSync(process.argv[2], "utf8"));
const detected = fs
  .readFileSync(process.argv[3], "utf8")
  .trim()
  .split(/\s+/)
  .filter(Boolean)
  .map(Number);
const expected = tl.turn_points_sec;
const maxAllowedMs = Number(process.argv[4] || 120);

let maxMs = 0;
let sumMs = 0;
for (const exp of expected) {
  let best = Infinity;
  for (const d of detected) best = Math.min(best, Math.abs(d - exp));
  maxMs = Math.max(maxMs, best * 1000);
  sumMs += best * 1000;
}
const meanMs = expected.length ? sumMs / expected.length : 0;
console.log(
  `翻页点 ${expected.length} 个：最大误差 ${maxMs.toFixed(0)} ms，平均 ${meanMs.toFixed(0)} ms` +
    `（scene 检出 ${detected.length} 个）`
);
if (maxMs > maxAllowedMs) {
  console.error(`✗ 最大翻页误差 ${maxMs.toFixed(0)} ms 超过阈值 ${maxAllowedMs} ms`);
  process.exit(1);
}
