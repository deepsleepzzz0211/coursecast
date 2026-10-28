/**
 * Command bridge: in Tauri every call goes straight to the Rust command layer
 * (same cc-core path as cc-cli). In a plain browser (no `__TAURI_INTERNALS__`)
 * a fixture-replay adapter answers instead — that mode exists only so the
 * wizard UI can be exercised/reviewed without building the shell; fixture data
 * is never authoritative.
 */
import { invoke as tauriInvoke, convertFileSrc } from "@tauri-apps/api/core";
import type { ProgressEvent } from "./types";
import { mockInvoke, subscribeMockProgress } from "./mockAdapter";

export const inTauri = "__TAURI_INTERNALS__" in window;

/**
 * One command seam for Tauri and the browser mock. Business failures always resolve
 * as `{ok:false, error:{code,message}}` — no caller needs a second throw path.
 */
export async function call<T = any>(cmd: string, args?: Record<string, unknown>): Promise<T> {
  try {
    return await (inTauri ? tauriInvoke<T>(cmd, args) : mockInvoke<T>(cmd, args));
  } catch (e: any) {
    if (e && e.ok === false && e.error) return e as T;
    return {
      ok: false,
      error: {
        code: e?.code || "command_failed",
        message: e?.message || String(e),
      },
    } as T;
  }
}

/** Absolute path -> webview asset url (Tauri assetProtocol scope: $APPDATA/**). */
export function assetUrl(absolutePath: string): string {
  return inTauri ? convertFileSrc(absolutePath) : absolutePath;
}

// ---------------------------------------------------------------------------
// progress events (票 13): Tauri "cc-progress" listen / browser mock emitter
// ---------------------------------------------------------------------------

/** Subscribe to generate/matte progress. Returns an unsubscribe function. */
export async function subscribeProgress(
  cb: (ev: ProgressEvent) => void,
): Promise<() => void> {
  if (inTauri) {
    const { listen } = await import("@tauri-apps/api/event");
    const un = await listen<ProgressEvent>("cc-progress", (e) => cb(e.payload));
    return un;
  }
  return subscribeMockProgress(cb);
}
