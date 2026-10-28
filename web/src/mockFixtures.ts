/**
 * Static fixtures for the browser-only mock adapter. They mirror the real artifacts
 * (slide PNGs, the matte preview frame, the sample-sentence wav) and the canonical
 * command results/progress events, so the wizard can be reviewed without the Tauri
 * shell. Nothing here is authoritative; the adapter only replays these values.
 */

import type { Layout, ProjectEntry, ProgressEvent } from "./types";

export function mockThumb(i: number, n: number): string {
  const hue = Math.round((i / Math.max(n, 1)) * 360);
  const svg = `<svg xmlns="http://www.w3.org/2000/svg" width="960" height="540"><rect width="960" height="540" fill="hsl(${hue},45%,80%)"/><text x="480" y="290" font-size="120" text-anchor="middle" fill="hsl(${hue},45%,30%)">${i + 1}</text></svg>`;
  return `data:image/svg+xml;base64,${btoa(unescape(encodeURIComponent(svg)))}`;
}

/** Canvas checkerboard + a person silhouette — the mock of work/person/preview.png. */
export function mockMattePreview(): string {
  const c = document.createElement("canvas");
  c.width = 320;
  c.height = 440;
  const g = c.getContext("2d")!;
  for (let y = 0; y < c.height; y += 16)
    for (let x = 0; x < c.width; x += 16) {
      g.fillStyle = (x / 16 + y / 16) % 2 === 0 ? "#ffffff" : "#cccccc";
      g.fillRect(x, y, 16, 16);
    }
  g.fillStyle = "#3a7bd5";
  g.beginPath();
  g.arc(c.width / 2, c.height / 2 + 50, 90, 0, Math.PI * 2);
  g.fill();
  g.fillStyle = "#f2c094";
  g.beginPath();
  g.arc(c.width / 2, c.height / 2 - 60, 42, 0, Math.PI * 2);
  g.fill();
  return c.toDataURL("image/png");
}

/** JS-synthesized 2 s / 24 kHz sine wav — the mock of work/voice_preview.wav. */
export function mockWavDataUrl(): string {
  const rate = 24000;
  const n = rate * 2;
  const buf = new ArrayBuffer(44 + n * 2);
  const v = new DataView(buf);
  const w = (o: number, s: string) => {
    for (let i = 0; i < s.length; i++) v.setUint8(o + i, s.charCodeAt(i));
  };
  w(0, "RIFF");
  v.setUint32(4, 36 + n * 2, true);
  w(8, "WAVE");
  w(12, "fmt ");
  v.setUint32(16, 16, true);
  v.setUint16(20, 1, true);
  v.setUint16(22, 1, true);
  v.setUint32(24, rate, true);
  v.setUint32(28, rate * 2, true);
  v.setUint16(32, 2, true);
  v.setUint16(34, 16, true);
  w(36, "data");
  v.setUint32(40, n * 2, true);
  for (let i = 0; i < n; i++) {
    const t = i / rate;
    const s = 0.3 * Math.sin(2 * Math.PI * 440 * t) * Math.exp(-t * 0.2);
    v.setInt16(44 + i * 2, Math.max(-32768, Math.min(32767, Math.round(s * 32767))), true);
  }
  let bin = "";
  const bytes = new Uint8Array(buf);
  for (let i = 0; i < bytes.length; i++) bin += String.fromCharCode(bytes[i]);
  return "data:audio/wav;base64," + btoa(bin);
}

export const MOCK_PROJECT_ID = "mock-demo";

/** One project entry for the home list; opening it restores at the finished step. */
export const MOCK_PROJECT: ProjectEntry = {
  project_id: MOCK_PROJECT_ID,
  name: "示例课程（fixture）",
  resolution: "1920x1080",
  stage: "done",
  created_at: "2026-10-26T00:00:00.000Z",
  progress: { slides: true, script: true, person: true, film: true },
};

export const MOCK_SLIDES = [mockThumb(0, 3), mockThumb(1, 3), mockThumb(2, 3)];

export const MOCK_SCRIPT_PAGES = [
  "大家好，欢迎来到本节课。今天我们学习三个要点。",
  "第一个要点是输入与输出的关系，注意观察示例。",
  "最后回顾一下：先理解概念，再动手练习。",
];

export const MOCK_LAYOUT: Layout = {
  position: "bottom_right",
  width_pct: 20,
  shape: "rounded_rect",
  transition: "hard",
};

export const MOCK_REF_AUDIO = {
  path: "source/voice_ref.wav",
  path_abs: "mock://voice_ref.wav",
  duration_sec: 12.5,
  sample_rate: 24000,
  channels: 1,
};

/** One replayed progress event; the fixture owns the delay before it fires. */
export interface MockProgressStep {
  delay_ms: number;
  segment: ProgressEvent["segment"];
  state: ProgressEvent["state"];
  done?: number | null;
  total?: number | null;
  message?: string | null;
}

export const MOCK_MATTE_EVENTS: MockProgressStep[] = [
  { delay_ms: 0, segment: "matte", state: "start" },
  ...Array.from({ length: 6 }, (_, i) => ({
    delay_ms: 90,
    segment: "matte" as const,
    state: "progress" as const,
    done: (i + 1) * 4,
    message: `${(i + 1) * 4} 帧`,
  })),
  { delay_ms: 90, segment: "matte", state: "done", done: 24, message: "24 帧" },
];

const generateHead: MockProgressStep[] = [
  { delay_ms: 0, segment: "tts", state: "start", total: 3 },
  { delay_ms: 350, segment: "tts", state: "progress", done: 1, total: 3 },
  { delay_ms: 350, segment: "tts", state: "progress", done: 2, total: 3 },
  { delay_ms: 350, segment: "tts", state: "progress", done: 3, total: 3 },
  { delay_ms: 0, segment: "tts", state: "done", done: 3, total: 3 },
  { delay_ms: 300, segment: "timeline", state: "start" },
  { delay_ms: 600, segment: "timeline", state: "done", message: "整轨 18.2 秒" },
];

const generateTail: MockProgressStep[] = [
  { delay_ms: 0, segment: "render", state: "start" },
  { delay_ms: 900, segment: "render", state: "done", message: "18.2 秒成片" },
];

export const MOCK_GENERATE_EVENTS: MockProgressStep[] = [
  ...generateHead,
  { delay_ms: 0, segment: "matte", state: "start" },
  { delay_ms: 200, segment: "matte", state: "progress", done: 8, message: "8 帧" },
  { delay_ms: 200, segment: "matte", state: "progress", done: 16, message: "16 帧" },
  { delay_ms: 200, segment: "matte", state: "done", done: 24, message: "24 帧" },
  ...generateTail,
];

export const MOCK_GENERATE_CARD_EVENTS: MockProgressStep[] = [
  ...generateHead,
  { delay_ms: 0, segment: "matte", state: "skipped", message: "卡片兜底" },
  ...generateTail,
];
