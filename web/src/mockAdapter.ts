//! Dev-only browser fixture replay. Every command returns a fixed result from
//! mockFixtures and every long-running command replays a canned progress-event script.
//! It owns no stage, invalidation or consent state: the same command always answers the
//! same way, so browser preview can never become a second domain implementation.
import type { ProgressEvent } from "./types";
import {
  MOCK_GENERATE_CARD_EVENTS,
  MOCK_GENERATE_EVENTS,
  MOCK_LAYOUT,
  MOCK_MATTE_EVENTS,
  MOCK_PROJECT,
  MOCK_PROJECT_ID,
  MOCK_REF_AUDIO,
  MOCK_SCRIPT_PAGES,
  MOCK_SLIDES,
  mockMattePreview,
  mockWavDataUrl,
  type MockProgressStep,
} from "./mockFixtures";

const mockProgressListeners = new Set<(ev: ProgressEvent) => void>();

export function subscribeMockProgress(cb: (ev: ProgressEvent) => void): () => void {
  mockProgressListeners.add(cb);
  return () => mockProgressListeners.delete(cb);
}

const sleep = (ms: number) => new Promise((r) => setTimeout(r, ms));

function emit(projectId: string, step: Omit<MockProgressStep, "delay_ms">) {
  const ev: ProgressEvent = {
    project_id: projectId,
    segment: step.segment,
    state: step.state,
    done: step.done ?? null,
    total: step.total ?? null,
    message: step.message ?? null,
  };
  mockProgressListeners.forEach((cb) => cb(ev));
}

async function replay(projectId: string, steps: MockProgressStep[]) {
  for (const step of steps) {
    await sleep(step.delay_ms);
    emit(projectId, step);
  }
}

function fail(code: string, message: string): never {
  throw { ok: false, error: { code, message } };
}

export async function mockInvoke<T>(cmd: string, args?: Record<string, any>): Promise<T> {
  await sleep(150);
  const id = () => String(args?.projectId || MOCK_PROJECT_ID);
  switch (cmd) {
    case "create_project":
      return {
        ok: true,
        project_id: MOCK_PROJECT_ID,
        name: String(args?.name || MOCK_PROJECT.name),
        resolution: String(args?.resolution || MOCK_PROJECT.resolution),
        stage: "created",
      } as T;
    case "list_projects":
      return { ok: true, projects: [MOCK_PROJECT] } as T;
    case "delete_project":
      if (!args?.confirm) {
        fail("invalid_argument", "project_delete 是不可逆操作：需要显式 confirm:true 才会执行");
      }
      return {
        ok: true,
        project_id: id(),
        deleted: true,
        dir: "mock://" + id(),
        next: "项目已删除，不可恢复",
      } as T;
    case "slide_render": {
      const src = String(args?.sourcePath || "");
      if (!/\.(pdf|pptx)$/i.test(src)) {
        fail("source_unsupported", "不支持的课件类型：请拖入 .pdf 或 .pptx 文件");
      }
      if (/\.pptx$/i.test(src) && /broken/i.test(src)) {
        fail(
          "source_unsupported",
          "pptx 本机转换失败（fixture 注入）。请先在 PowerPoint 中另存为 PDF，再拖入该 PDF 文件"
        );
      }
      return {
        ok: true,
        page_count: MOCK_SLIDES.length,
        slides: MOCK_SLIDES,
        slides_dir: "mock://",
        stage: "parsed",
      } as T;
    }
    case "slides_list":
      return { ok: true, count: MOCK_SLIDES.length, files: MOCK_SLIDES } as T;
    case "script_get":
      return { ok: true, exists: true, pages: MOCK_SCRIPT_PAGES } as T;
    case "script_set":
      return {
        ok: true,
        page_count: MOCK_SCRIPT_PAGES.length,
        script: "script.md",
      } as T;
    case "script_import_notes":
      return {
        ok: true,
        imported_pages: MOCK_SCRIPT_PAGES.length,
        pages: MOCK_SCRIPT_PAGES,
        script: "script.md",
      } as T;
    case "script_split":
      return { ok: true, segments: MOCK_SCRIPT_PAGES } as T;
    case "script_balance":
      return { ok: true, pages: MOCK_SCRIPT_PAGES } as T;
    case "project_status":
      return { ok: true, project_id: id(), stage: MOCK_PROJECT.stage, missing: [] } as T;

    // ---- Step 4 配置声音 ----------------------------------------------------
    case "voice_configure": {
      const hasKey = !!args?.apiKey;
      const hasAudio = !!args?.audioPath;
      const hasPreference = !!args?.engine || !!args?.edgeVoice;
      if (!hasKey && !hasAudio && !hasPreference) {
        fail("invalid_argument", "voice_configure 需要 API Key、参考音频或配音方式至少其一");
      }
      return {
        ok: true,
        api_key_stored: hasKey,
        audio: hasAudio ? MOCK_REF_AUDIO : null,
        consent: true,
        tts: {
          engine: args?.engine === "edge" ? "edge" : "mimo",
          edge_voice: args?.edgeVoice === "yunxi" ? "yunxi" : "xiaoxiao",
        },
      } as T;
    }
    case "voice_status":
      return {
        ok: true,
        has_key: false,
        ref_audio: MOCK_REF_AUDIO,
        consent: true,
        tts: { engine: "mimo", edge_voice: "xiaoxiao" },
      } as T;
    case "voice_preview":
      await sleep(900);
      return {
        ok: true,
        text: "大家好，欢迎来到我的课程，这就是我克隆出来的声音。",
        path_abs: mockWavDataUrl(),
        duration_sec: 2.0,
        sample_rate: 24000,
        channels: 1,
      } as T;
    case "voice_consent_get":
      return { ok: true, granted: true } as T;
    case "voice_consent_set":
      return { ok: true, granted: !!args?.granted } as T;

    // ---- Step 5 出镜视频 ----------------------------------------------------
    case "person_video_set": {
      const src = String(args?.sourcePath || "");
      if (!/\.(mp4|mov)$/i.test(src)) {
        fail("source_unsupported", "不支持的出镜视频类型：请拖入 .mp4 或 .mov 文件（建议 30–60 秒）");
      }
      if (/broken/i.test(src)) {
        fail("engine_error", "出镜视频损坏（fixture 注入）。请换一段 .mp4 / .mov 后重试");
      }
      return { ok: true, path: "source/person.mp4", path_abs: "mock://person.mp4" } as T;
    }
    case "person_matte_skip":
      return {
        ok: true,
        project_id: id(),
        matte_done: false,
        next: "生成时用纯色圆角卡片包住原视频（照样能出片）",
      } as T;
    case "person_status":
      return {
        ok: true,
        stage: MOCK_PROJECT.stage,
        has_video: true,
        video_abs: "mock://person.mp4",
        matte_done: true,
        preview_abs: mockMattePreview(),
        alpha_abs: "mock://alpha.mov",
      } as T;
    case "person_matte":
      await replay(id(), MOCK_MATTE_EVENTS);
      return {
        ok: true,
        skipped: false,
        frames: 24,
        width: 320,
        height: 240,
        fps: 30,
        elapsed_sec: 2.2,
        preview_abs: mockMattePreview(),
      } as T;

    // ---- Step 6 版式 + 生成 --------------------------------------------------
    case "layout_get":
      return {
        ok: true,
        layout: {
          mode: "free",
          x_pct: 85,
          y_pct: 75,
          width_pct: MOCK_LAYOUT.width_pct,
          shape: MOCK_LAYOUT.shape,
          transition: MOCK_LAYOUT.transition,
        },
      } as T;
    case "layout_set": {
      const layout = args?.position
        ? {
            mode: "preset",
            position: args.position,
            width_pct: Number(args?.widthPct ?? MOCK_LAYOUT.width_pct),
            shape: args?.shape ?? MOCK_LAYOUT.shape,
            transition: args?.transition ?? MOCK_LAYOUT.transition,
          }
        : {
            mode: "free",
            x_pct: Number(args?.xPct ?? 85),
            y_pct: Number(args?.yPct ?? 75),
            width_pct: Number(args?.widthPct ?? MOCK_LAYOUT.width_pct),
            shape: args?.shape ?? MOCK_LAYOUT.shape,
            transition: args?.transition ?? MOCK_LAYOUT.transition,
          };
      return {
        ok: true,
        layout,
        next: "版式已更新：运行 render 只重渲成片",
      } as T;
    }
    case "generate": {
      const useMatte = !!args?.useMatte;
      await replay(id(), useMatte ? MOCK_GENERATE_EVENTS : MOCK_GENERATE_CARD_EVENTS);
      return {
        ok: true,
        stage: "done",
        segments: {
          tts: { state: "done" },
          timeline: { state: "done", total_sec: 18.2 },
          matte: useMatte
            ? { state: "done" }
            : { state: "skipped", reason: "card_fallback" },
          render: { state: "done", pip: useMatte ? "alpha" : "card", total_sec: 18.2 },
        },
        fallbacks: [
          {
            from: "mimo",
            to: "sapi",
            reason: "fixture 演示：未配置 MiMo API Key，已自动降级到系统兜底音色（mock）",
          },
        ],
        output_abs: "/mock-sample.mp4",
        total_sec: 18.2,
      } as T;
    }
    case "final_video":
      return { ok: true, done: true, path_abs: "/mock-sample.mp4" } as T;
    case "download_video": {
      const name = `课影-${MOCK_PROJECT.name}.mp4`;
      const a = document.createElement("a");
      a.href = "/mock-sample.mp4";
      a.download = name;
      document.body.appendChild(a);
      a.click();
      a.remove();
      return { ok: true, saved: `下载文件夹/${name}（fixture 演示下载）` } as T;
    }
    default:
      fail("unknown_command", `mock 未实现 ${cmd}`);
  }
}
