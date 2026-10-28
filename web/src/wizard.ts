import { reactive } from "vue";
import { call } from "./api";
import type { VoiceEngineKind } from "./types";

function eligibility(
  consent: boolean,
  hasPerson: boolean,
  engine: VoiceEngineKind
): { ok: boolean; reason?: string } {
  if (engine === "mimo" && !consent) {
    return {
      ok: false,
      reason: "MiMo 克隆需要先勾选《声音授权确认》；也可以改用 Edge 朗读（免授权）",
    };
  }
  if (!hasPerson) {
    return { ok: false, reason: "缺少出镜视频：请先在第 5 步上传" };
  }
  return { ok: true };
}

/**
 * One wizard gate for navigation and the generate button. All eligibility facts come
 * from the same command results (slides / voice / person / status), so the header nav
 * and Step6 cannot disagree.
 */
export const wizard = reactive({
  projectId: "",
  pageCount: 0,
  consent: false,
  engine: "mimo" as VoiceEngineKind,
  hasPerson: false,
  missing: [] as string[],
  loaded: false,

  async refresh(projectId: string) {
    this.projectId = projectId;
    const [slides, voice, person, status] = await Promise.all([
      call<any>("slides_list", { projectId }),
      call<any>("voice_status", { projectId }),
      call<any>("person_status", { projectId }),
      call<any>("project_status", { projectId }),
    ]);
    this.pageCount = slides.ok ? Number(slides.count || 0) : 0;
    this.consent = voice.ok ? !!voice.consent : false;
    this.engine = voice.ok && voice.tts?.engine === "edge" ? "edge" : "mimo";
    this.hasPerson = person.ok ? !!person.has_video : false;
    this.missing = status.ok ? status.missing || [] : [];
    this.loaded = true;
  },

  setPageCount(count: number) {
    this.pageCount = count;
  },

  canEnter(step: number): { ok: boolean; reason?: string } {
    if (step <= 2) return { ok: true };
    if (!this.pageCount) return { ok: false, reason: "请先上传课件并完成渲染" };
    if (step >= 6) return eligibility(this.consent, this.hasPerson, this.engine);
    return { ok: true };
  },

  canGenerate(): { ok: boolean; reason?: string } {
    if (!this.pageCount) return { ok: false, reason: "请先上传课件并完成渲染" };
    return eligibility(this.consent, this.hasPerson, this.engine);
  },
});
