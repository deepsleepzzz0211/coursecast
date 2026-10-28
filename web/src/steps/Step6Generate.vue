<script setup lang="ts">
import { computed, onMounted, ref } from "vue";
import { ElMessage } from "element-plus";
import { assetUrl, call } from "../api";
import { useProgress } from "../progress";
import { wizard } from "../wizard";
import LayoutPreview from "./LayoutPreview.vue";
import type { Layout, LayoutAnchor, ProjectEntry } from "../types";

const props = defineProps<{ project: ProjectEntry }>();

const layout = ref<Layout>({
  mode: "preset",
  position: "bottom_right",
  width_pct: 20,
  shape: "rounded_rect",
  transition: "hard",
});
const layoutLoaded = ref(false);
const savingLayout = ref(false);
const layoutHint = ref("");
const xPct = ref(95);
const yPct = ref(90);
const widthPct = ref(20);
const previewDirty = ref(false);
const slideUrl = ref<string | null>(null);
const personUrl = ref<string | null>(null);
const personKind = ref<"alpha" | "card" | "none">("none");

const canvasWidth = computed(() => {
  const width = Number(props.project.resolution.split("x")[0]);
  return Number.isFinite(width) && width > 0 ? width : 1920;
});
const canvasHeight = computed(() => {
  const height = Number(props.project.resolution.split("x")[1]);
  return Number.isFinite(height) && height > 0 ? height : 1080;
});
const previewLayout = computed<Layout>(() => {
  if (!previewDirty.value) return layout.value;
  return {
    ...layout.value,
    mode: "free",
    x_pct: xPct.value,
    y_pct: yPct.value,
    width_pct: widthPct.value,
  };
});

const { segments, start: startProgress, reset: resetProgress } = useProgress(
  () => props.project.project_id,
);
const segmentMeta: Record<string, { title: string; desc: string }> = {
  tts: { title: "合成语音", desc: "按逐字稿克隆/兜底音色逐页配音" },
  timeline: { title: "拼接音轨", desc: "页间 0.6s 静音 + 整轨淡出" },
  matte: { title: "处理人像", desc: "抠像画中画（跳过则卡片兜底）" },
  render: { title: "渲染成片", desc: "ffmpeg 一次性合成 MP4" },
};
const segmentOrder = ["tts", "timeline", "matte", "render"];

const generating = ref(false);
const generateError = ref<string | null>(null);
const failedSegment = ref<string | null>(null);
const fallbackBanner = ref<string | null>(null);
const filmSrc = ref<string | null>(null);
const filmTotal = ref<number | null>(null);
const pipSource = ref<string>("");
const downloading = ref(false);
const savedTo = ref<string | null>(null);

const canGenerate = computed(() => wizard.canGenerate().ok && !generating.value);

onMounted(async () => {
  // restore first: progress events are a nicety, a denied listener must not block
  // the gate/film from loading (Tauri 2 capabilities can vary by build).
  await Promise.all([
    restoreLayout(),
    restorePreviewAssets(),
    wizard.refresh(props.project.project_id),
    restoreFilm(),
  ]);
  await startProgress();
});

async function restoreLayout() {
  const v = await call<any>("layout_get", { projectId: props.project.project_id });
  if (v.ok) {
    layout.value = v.layout;
    widthPct.value = Number(v.layout.width_pct ?? 20);
    if (v.layout.mode === "free") {
      xPct.value = Number(v.layout.x_pct ?? 0);
      yPct.value = Number(v.layout.y_pct ?? 0);
    }
  }
  layoutLoaded.value = true;
}

async function restorePreviewAssets() {
  const [slides, person] = await Promise.all([
    call<any>("slides_list", { projectId: props.project.project_id }),
    call<any>("person_status", { projectId: props.project.project_id }),
  ]);
  if (slides.ok && Array.isArray(slides.files) && slides.files.length) {
    slideUrl.value = assetUrl(slides.files[0]);
  }
  if (person.ok) {
    if (person.matte_done && person.preview_abs) {
      personKind.value = "alpha";
      personUrl.value = assetUrl(person.preview_abs);
    } else if (person.has_video && person.video_abs) {
      personKind.value = "card";
      personUrl.value = assetUrl(person.video_abs);
    }
  }
}

function onPreviewReady(value: LayoutAnchor) {
  if (!layoutLoaded.value || previewDirty.value) return;
  xPct.value = value.x_pct;
  yPct.value = value.y_pct;
}

async function restoreFilm() {
  const v = await call<any>("final_video", { projectId: props.project.project_id });
  if (v.ok && v.done) {
    filmSrc.value = assetUrl(v.path_abs);
  }
}

function markPreviewDirty() {
  if (layoutLoaded.value) {
    previewDirty.value = true;
    layoutHint.value = "";
  }
}

async function saveLayout(free: boolean) {
  if (!layoutLoaded.value) return;
  savingLayout.value = true;
  try {
    const payload: Record<string, unknown> = {
      projectId: props.project.project_id,
      shape: layout.value.shape,
      transition: layout.value.transition,
    };
    if (free || previewDirty.value || layout.value.mode === "free") {
      payload.xPct = xPct.value;
      payload.yPct = yPct.value;
      payload.widthPct = widthPct.value;
    }
    const v = await call<any>("layout_set", payload);
    if (v.ok) {
      layout.value = v.layout;
      widthPct.value = Number(v.layout.width_pct ?? widthPct.value);
      if (v.layout.mode === "free") {
        xPct.value = Number(v.layout.x_pct ?? xPct.value);
        yPct.value = Number(v.layout.y_pct ?? yPct.value);
      }
      previewDirty.value = false;
      layoutHint.value = String(v.next || "版式已保存");
    } else {
      ElMessage.error(v.error?.message || "版式保存失败");
    }
  } catch (e: any) {
    ElMessage.error(e?.error?.message || "版式保存失败");
  } finally {
    savingLayout.value = false;
  }
}

function changeShapeTransition() {
  if (layoutLoaded.value) saveLayout(false);
}

function changeFreeLayout() {
  if (layoutLoaded.value) saveLayout(true);
}

function applyAnchor(value: LayoutAnchor) {
  previewDirty.value = true;
  xPct.value = value.x_pct;
  yPct.value = value.y_pct;
  widthPct.value = value.width_pct;
  layout.value = {
    ...layout.value,
    mode: "free",
    x_pct: value.x_pct,
    y_pct: value.y_pct,
    width_pct: value.width_pct,
  };
}

function onPreviewUpdate(value: LayoutAnchor) {
  applyAnchor(value);
}

async function onPreviewCommit(value: LayoutAnchor) {
  applyAnchor(value);
  await saveLayout(true);
}

async function applyQuickPosition(x: number, y: number, width = widthPct.value) {
  if (!layoutLoaded.value) return;
  applyAnchor({ x_pct: x, y_pct: y, width_pct: width });
  await saveLayout(true);
}

async function resetLayout() {
  if (!layoutLoaded.value) return;
  savingLayout.value = true;
  try {
    const v = await call<any>("layout_set", {
      projectId: props.project.project_id,
      position: "bottom_right",
      widthPct: 20,
      shape: "rounded_rect",
      transition: "hard",
    });
    if (v.ok) {
      layout.value = v.layout;
      widthPct.value = Number(v.layout.width_pct ?? 20);
      previewDirty.value = false;
      layoutHint.value = String(v.next || "已恢复默认版式");
    } else {
      ElMessage.error(v.error?.message || "恢复默认版式失败");
    }
  } catch (e: any) {
    ElMessage.error(e?.error?.message || "恢复默认版式失败");
  } finally {
    savingLayout.value = false;
  }
}

async function generate() {
  const gate = wizard.canGenerate();
  if (!gate.ok) {
    ElMessage.warning(gate.reason || "请先完成前面的步骤");
    return;
  }
  generating.value = true;
  generateError.value = null;
  failedSegment.value = null;
  fallbackBanner.value = null;
  savedTo.value = null;
  resetProgress();
  // 抠像选择：第 5 步已抠像（有成功结果）→ 走 alpha；跳过/未抠 → 卡片兜底
  let useMatte = false;
  const person = await call<any>("person_status", { projectId: props.project.project_id });
  if (person.ok) useMatte = !!person.matte_done;

  const v = await call<any>("generate", {
    projectId: props.project.project_id,
    useMatte,
  });
  if (v.segments) {
    for (const key of segmentOrder) {
      if (v.segments[key]) segments[key].state = v.segments[key].state;
    }
  }
  const fbs = v?.fallbacks || v?.segments?.tts?.fallbacks || [];
  if (Array.isArray(fbs) && fbs.length) {
    fallbackBanner.value = fbs
      .map((f: any) => `语音引擎 ${f.from} → ${f.to}：${f.reason}`)
      .join("；");
  }
  if (v.ok) {
    filmSrc.value = assetUrl(v.output_abs);
    filmTotal.value = v.total_sec ?? null;
    pipSource.value = v.segments?.render?.pip ?? "";
    ElMessage.success("成片已生成");
  } else {
    failedSegment.value = v.error?.segment || null;
    generateError.value = v.error?.message || "生成失败";
  }
  generating.value = false;
}

async function download() {
  downloading.value = true;
  const v = await call<any>("download_video", { projectId: props.project.project_id });
  if (v.ok) {
    savedTo.value = v.saved;
    ElMessage.success(`已下载：${v.saved}`);
  } else {
    ElMessage.error(v.error?.message || "下载失败");
  }
  downloading.value = false;
}

function segIcon(state: string): string {
  switch (state) {
    case "done":
      return "✓";
    case "skipped":
      return "∘";
    case "failed":
      return "✕";
    case "running":
      return "…";
    default:
      return "";
  }
}
</script>

<template>
  <div class="step6">
    <el-alert
      v-if="wizard.engine === 'mimo' && !wizard.consent"
      type="warning"
      show-icon
      :closable="false"
      title="MiMo 克隆尚未勾选《声音授权确认》：请回第 4 步勾选，或改用 Edge 朗读"
      class="banner"
    />
    <el-alert
      v-if="!wizard.hasPerson"
      type="error"
      show-icon
      :closable="false"
      title="缺少出镜视频：请先在第 5 步上传出镜视频"
      class="banner"
    />
    <el-alert v-if="generateError" :title="generateError" type="error" show-icon :closable="true" class="banner" />
    <el-alert
      v-if="fallbackBanner"
      :title="`已自动降级：${fallbackBanner}（成片仍会生成）`"
      type="warning"
      show-icon
      :closable="true"
      class="banner"
    />

    <section class="card">
      <h3>版式预览</h3>
      <LayoutPreview
        :layout="previewLayout"
        :slide-url="slideUrl"
        :person-url="personUrl"
        :person-kind="personKind"
        :canvas-width="canvasWidth"
        :canvas-height="canvasHeight"
        editable
        @ready="onPreviewReady"
        @update="onPreviewUpdate"
        @commit="onPreviewCommit"
      />
      <el-alert
        v-if="layout.mode !== 'free'"
        type="info"
        show-icon
        :closable="false"
        title="当前是旧版式；调整位置或宽度后转为自由版式，旧项目仍可正常重渲"
        class="legacy-layout-alert"
      />
      <div class="layout-controls">
        <div class="layout-item">
          <div class="layout-label">
            水平位置 <span class="layout-value">{{ Math.round(xPct) }}%</span>
          </div>
          <div class="layout-control-row">
            <el-slider
              v-model="xPct"
              :min="0"
              :max="100"
              :disabled="!layoutLoaded"
              @input="markPreviewDirty"
              @change="changeFreeLayout"
            />
            <el-input-number
              v-model="xPct"
              :min="0"
              :max="100"
              :step="1"
              :precision="0"
              size="small"
              controls-position="right"
              class="layout-number"
              @input="markPreviewDirty"
              @change="changeFreeLayout"
            />
          </div>
        </div>
        <div class="layout-item">
          <div class="layout-label">
            垂直位置 <span class="layout-value">{{ Math.round(yPct) }}%</span>
          </div>
          <div class="layout-control-row">
            <el-slider
              v-model="yPct"
              :min="0"
              :max="100"
              :disabled="!layoutLoaded"
              @input="markPreviewDirty"
              @change="changeFreeLayout"
            />
            <el-input-number
              v-model="yPct"
              :min="0"
              :max="100"
              :step="1"
              :precision="0"
              size="small"
              controls-position="right"
              class="layout-number"
              @input="markPreviewDirty"
              @change="changeFreeLayout"
            />
          </div>
        </div>
        <div class="layout-item">
          <div class="layout-label">
            人像宽度 <span class="layout-value">{{ Math.round(widthPct) }}%</span>
          </div>
          <div class="layout-control-row">
            <el-slider
              v-model="widthPct"
              :min="5"
              :max="40"
              :disabled="!layoutLoaded"
              @input="markPreviewDirty"
              @change="changeFreeLayout"
            />
            <el-input-number
              v-model="widthPct"
              :min="5"
              :max="40"
              :step="1"
              :precision="0"
              size="small"
              controls-position="right"
              class="layout-number"
              @input="markPreviewDirty"
              @change="changeFreeLayout"
            />
          </div>
        </div>
      </div>
      <div class="layout-presets">
        <span class="preset-label">快捷位置</span>
        <el-button size="small" @click="applyQuickPosition(0, 0)">左上</el-button>
        <el-button size="small" @click="applyQuickPosition(100, 0)">右上</el-button>
        <el-button size="small" @click="applyQuickPosition(0, 100)">左下</el-button>
        <el-button size="small" @click="applyQuickPosition(100, 100)">右下</el-button>
        <el-button size="small" @click="applyQuickPosition(50, 50)">居中</el-button>
        <el-button size="small" @click="resetLayout">恢复默认</el-button>
      </div>
      <p class="hint layout-hint">
        拖动人像或右下角蓝色圆点缩放；位置为剩余画布空间的 0–100%（100 = 贴右/下边），
        宽度 5–40%，超高素材会自动缩小以保持在画布内。
        <template v-if="personKind === 'card'">卡片兜底固定为圆角形状。</template>
      </p>
      <div class="layout-grid">
        <div class="layout-item">
          <div class="layout-label">形状</div>
          <el-radio-group v-model="layout.shape" @change="changeShapeTransition">
            <el-radio-button value="rounded_rect">圆角矩形</el-radio-button>
            <el-radio-button value="circle">圆形</el-radio-button>
          </el-radio-group>
        </div>
        <div class="layout-item">
          <div class="layout-label">翻页动效</div>
          <el-radio-group v-model="layout.transition" @change="changeShapeTransition">
            <el-radio-button value="hard">硬切</el-radio-button>
            <el-radio-button value="fade">淡入淡出 0.3s</el-radio-button>
          </el-radio-group>
        </div>
      </div>
      <div v-if="savingLayout" class="layout-saving">保存版式…</div>
      <el-alert
        v-if="layoutHint"
        :title="layoutHint"
        type="success"
        show-icon
        :closable="true"
        class="layout-saved-hint"
        @close="layoutHint = ''"
      />
    </section>

    <section class="card">
      <div class="gen-head">
        <h3>生成</h3>
        <el-button type="primary" size="large" :loading="generating" :disabled="!canGenerate" @click="generate">
          {{ generating ? "生成中…" : segments.render.state === "done" ? "重新生成" : "开始生成" }}
        </el-button>
      </div>
      <ol class="segments">
        <li
          v-for="key in segmentOrder"
          :key="key"
          class="segment"
          :class="`s-${segments[key].state}`"
        >
          <span class="seg-icon">{{ segIcon(segments[key].state) }}</span>
          <div class="seg-body">
            <div class="seg-title">
              {{ segmentMeta[key].title }}
              <span class="seg-state">{{ segments[key].state }}</span>
            </div>
            <div class="seg-desc">{{ segmentMeta[key].desc }}</div>
            <div v-if="segments[key].note" class="seg-note">{{ segments[key].note }}</div>
            <el-progress
              v-if="segments[key].state === 'running' && segments[key].total"
              :percentage="Math.min(100, Math.round(((segments[key].done || 0) / (segments[key].total || 1)) * 100))"
              class="seg-progress"
            />
            <el-button
              v-if="key === failedSegment && !generating"
              size="small"
              type="primary"
              @click="generate"
            >
              重试此段
            </el-button>
          </div>
        </li>
      </ol>
    </section>

    <section v-if="filmSrc" class="card film-card">
      <h3>成片预览<template v-if="filmTotal">（{{ filmTotal.toFixed(1) }} 秒 · {{ pipSource === "card" ? "卡片兜底" : "抠像画中画" }}）</template></h3>
      <video class="film" :src="filmSrc" controls />
      <div class="film-actions">
        <el-button type="primary" :loading="downloading" @click="download">下载 MP4</el-button>
        <span v-if="savedTo" class="saved">已保存到：{{ savedTo }}</span>
      </div>
    </section>
  </div>
</template>

<style scoped>
.step6 {
  display: flex;
  flex-direction: column;
  gap: 14px;
  max-width: 860px;
}
.banner {
  margin-bottom: 0;
}
.card {
  background: #fff;
  border: 1px solid #e4e7ed;
  border-radius: 8px;
  padding: 14px 18px;
}
.card h3 {
  margin: 0 0 10px;
  font-size: 15px;
}
.layout-controls {
  display: grid;
  grid-template-columns: repeat(auto-fit, minmax(260px, 1fr));
  gap: 4px 20px;
  margin-top: 14px;
}
.layout-grid {
  display: grid;
  grid-template-columns: repeat(auto-fit, minmax(300px, 1fr));
  gap: 14px 20px;
  margin-top: 14px;
}
.layout-value {
  color: #409eff;
  font-variant-numeric: tabular-nums;
}
.layout-control-row {
  display: flex;
  align-items: center;
  gap: 12px;
}
.layout-control-row :deep(.el-slider) {
  flex: 1;
}
.layout-number {
  width: 104px;
  flex-shrink: 0;
}
.layout-saved-hint {
  margin-top: 12px;
}
.layout-hint {
  margin: 6px 0 0;
}
.layout-presets {
  display: flex;
  align-items: center;
  gap: 8px;
  flex-wrap: wrap;
  margin-top: 12px;
}
.preset-label {
  color: #606266;
  font-size: 13px;
}
.legacy-layout-alert {
  margin-top: 12px;
}
.layout-label {
  font-size: 13px;
  color: #606266;
  margin-bottom: 6px;
}
.layout-saving {
  color: #909399;
  font-size: 12px;
  margin-top: 8px;
}
.gen-head {
  display: flex;
  justify-content: space-between;
  align-items: center;
  margin-bottom: 8px;
}
.gen-head h3 {
  margin: 0;
}
.segments {
  list-style: none;
  margin: 0;
  padding: 0;
}
.segment {
  display: flex;
  gap: 12px;
  padding: 10px 6px;
  border-bottom: 1px dashed #ebeef5;
}
.segment:last-child {
  border-bottom: none;
}
.seg-icon {
  width: 22px;
  height: 22px;
  border-radius: 50%;
  display: inline-flex;
  align-items: center;
  justify-content: center;
  font-size: 13px;
  font-weight: 700;
  background: #f0f2f5;
  color: #909399;
  flex-shrink: 0;
}
.segment.s-done .seg-icon {
  background: #f0f9eb;
  color: #67c23a;
}
.segment.s-running .seg-icon {
  background: #ecf5ff;
  color: #409eff;
}
.segment.s-failed .seg-icon {
  background: #fef0f0;
  color: #f56c6c;
}
.segment.s-skipped .seg-icon {
  background: #fdf6ec;
  color: #e6a23c;
}
.seg-title {
  font-weight: 600;
  font-size: 14px;
}
.seg-state {
  font-weight: 400;
  font-size: 12px;
  color: #909399;
  margin-left: 8px;
}
.seg-desc {
  font-size: 12px;
  color: #909399;
}
.seg-note {
  font-size: 13px;
  color: #606266;
  margin-top: 4px;
}
.seg-progress {
  margin-top: 6px;
  max-width: 420px;
}
.segment .el-button {
  margin-top: 8px;
}
.film {
  width: 100%;
  max-width: 720px;
  border-radius: 8px;
  background: #000;
}
.film-actions {
  display: flex;
  align-items: center;
  gap: 12px;
  margin-top: 10px;
}
.saved {
  color: #67c23a;
  font-size: 13px;
}
</style>
