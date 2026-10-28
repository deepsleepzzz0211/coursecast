import { onUnmounted, reactive } from "vue";
import { subscribeProgress } from "./api";
import type { ProgressEvent } from "./types";

export type SegmentState = {
  state: string;
  note: string;
  done?: number | null;
  total?: number | null;
};

const emptySegments = (): Record<string, SegmentState> => ({
  tts: { state: "pending", note: "" },
  timeline: { state: "pending", note: "" },
  matte: { state: "pending", note: "" },
  render: { state: "pending", note: "" },
});

/**
 * One progress store for the wizard. Tauri events and the browser mock both feed the
 * same `cc-progress` shape; Step5 and Step6 read the same segment states.
 */
export function useProgress(projectId: () => string) {
  const segments = reactive<Record<string, SegmentState>>(emptySegments());
  let unsubscribe: (() => void) | null = null;

  function onEvent(ev: ProgressEvent) {
    if (ev.project_id !== projectId()) return;
    const seg = segments[ev.segment];
    if (!seg) return;
    if (ev.state === "start") {
      seg.state = "running";
      seg.note = "";
      seg.total = ev.total ?? null;
    } else if (ev.state === "progress") {
      seg.state = "running";
      seg.done = ev.done ?? null;
      seg.total = ev.total ?? seg.total ?? null;
      seg.note =
        ev.message || (seg.total ? `${seg.done}/${seg.total}` : `${seg.done ?? ""}`);
    } else if (ev.state === "done" || ev.state === "skipped") {
      seg.state = ev.state;
      seg.note = ev.message || "";
    } else if (ev.state === "failed") {
      seg.state = "failed";
      seg.note = ev.message || "";
    }
  }

  async function start() {
    try {
      unsubscribe = await subscribeProgress(onEvent);
    } catch {
      /* progress unavailable; the commands still work without live ticks */
    }
  }

  function reset() {
    for (const key of Object.keys(segments)) {
      segments[key].state = "pending";
      segments[key].note = "";
      segments[key].done = null;
      segments[key].total = null;
    }
  }

  onUnmounted(() => unsubscribe?.());

  return { segments, start, reset };
}
