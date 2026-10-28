<script setup lang="ts">
import { computed, onMounted, onUnmounted, ref } from "vue";
import { ElMessage } from "element-plus";
import { assetUrl, call, inTauri } from "../api";
import { useProgress } from "../progress";
import { wizard } from "../wizard";
import type { ProjectEntry } from "../types";

const props = defineProps<{ project: ProjectEntry }>();
const emit = defineEmits<{ (e: "next"): void }>();

const hasVideo = ref(false);
const matteDone = ref(false);
const previewSrc = ref<string | null>(null);
const dragOver = ref(false);
const uploading = ref(false);
const matting = ref(false);
const info = ref<string>("");
const error = ref<string | null>(null);

const progress = useProgress(() => props.project.project_id);
const seg = progress.segments.matte;
const frames = computed(() => Number(seg.done || 0));

let unlistenDrag: (() => void) | null = null;

onMounted(async () => {
  await restore();
  await progress.start();
  if (inTauri) {
    const { getCurrentWebview } = await import("@tauri-apps/api/webview");
    unlistenDrag = await getCurrentWebview().onDragDropEvent((ev: any) => {
      if (ev.payload.type === "over") dragOver.value = true;
      else if (ev.payload.type === "leave") dragOver.value = false;
      else if (ev.payload.type === "drop") {
        dragOver.value = false;
        const path: string | undefined = ev.payload.paths?.[0];
        if (path) upload(path);
      }
    });
  }
});
onUnmounted(() => unlistenDrag?.());

async function restore() {
  const v = await call<any>("person_status", { projectId: props.project.project_id });
  if (v.ok) {
    hasVideo.value = v.has_video;
    matteDone.value = v.matte_done;
    wizard.hasPerson = v.has_video;
    if (v.matte_done && v.preview_abs) {
      previewSrc.value = assetUrl(v.preview_abs);
    }
  }
}

async function upload(path: string) {
  if (!/\.(mp4|mov)$/i.test(path)) {
    error.value = "出镜视频支持 .mp4 / .mov（建议 30–60 秒、光线均匀的半身出镜）";
    return;
  }
  uploading.value = true;
  error.value = null;
  const v = await call<any>("person_video_set", { projectId: props.project.project_id, sourcePath: path });
  if (v.ok) {
    hasVideo.value = true;
    matteDone.value = false;
    previewSrc.value = null;
    wizard.hasPerson = true;
    ElMessage.success("出镜视频已上传，开始自动抠像");
    await runMatte();
  } else {
    error.value = v.error?.message || "上传失败";
  }
  uploading.value = false;
}

async function onDomDrop(ev: DragEvent) {
  dragOver.value = false;
  const file = ev.dataTransfer?.files?.[0];
  if (file) await upload(file.name);
}

async function runMatte() {
  matting.value = true;
  progress.reset();
  seg.state = "running";
  seg.note = "启动引擎…";
  error.value = null;
  const v = await call<any>("person_matte", { projectId: props.project.project_id });
  if (v.ok) {
    matteDone.value = true;
    previewSrc.value = assetUrl(v.preview_abs);
    info.value = `${v.width}×${v.height} · ${v.frames} 帧 · ${v.elapsed_sec?.toFixed?.(1) ?? v.elapsed_sec}s${v.skipped ? "（输入未变，复用结果）" : ""}`;
    seg.state = v.skipped ? "skipped" : "done";
    seg.note = v.skipped ? "输入未变化，复用已抠好的结果" : "抠像完成";
  } else {
    seg.state = "failed";
    seg.note = "";
    error.value =
      v.error?.message ||
      "抠像失败：可重试，或跳过抠像——生成时将用纯色圆角卡片包住原视频照样出片";
  }
  matting.value = false;
}

async function skipMatte() {
  error.value = null;
  // 服务端持久化：清掉旧 alpha，避免生成/重进项目时又用回上一段视频的抠像
  const v = await call<any>("person_matte_skip", { projectId: props.project.project_id });
  if (!v.ok) {
    error.value = v.error?.message || "跳过抠像失败，请重试";
    return;
  }
  matteDone.value = false;
  previewSrc.value = null;
  seg.state = "skipped";
  seg.note = "已选择跳过抠像：生成时用纯色圆角卡片包住原视频";
  ElMessage.info("已选择卡片兜底：生成时将用纯色圆角卡片包住原视频出片");
}
</script>

<template>
  <div class="step5">
    <el-alert v-if="error" :title="error" type="error" show-icon :closable="true" class="banner" />

    <div
      class="dropzone"
      :class="{ over: dragOver, busy: uploading || matting }"
      @dragover.prevent="dragOver = true"
      @dragleave.prevent="dragOver = false"
      @drop.prevent="onDomDrop"
      v-loading="uploading"
      element-loading-text="正在保存出镜视频…"
    >
      <template v-if="!hasVideo">
        <div class="dropzone-icon">🎬</div>
        <div class="dropzone-title">把出镜视频拖到这里</div>
        <div class="dropzone-sub">支持 .mp4 / .mov，建议 30–60 秒半身出镜；上传后自动抠像</div>
      </template>
      <template v-else>
        <div class="dropzone-title">已上传出镜视频{{ matteDone ? "（抠像完成）" : "" }}</div>
        <div class="dropzone-sub">重新拖入可替换（抠像结果会自动失效重做）</div>
      </template>
    </div>

    <el-progress
      v-if="matting"
      :percentage="100"
      status="success"
      :indeterminate="true"
      :duration="0.5"
      :show-text="false"
      class="frame-progress"
    />
    <div v-if="matting" class="frame-note">正在逐帧抠像：已处理 {{ frames }} 帧…</div>
    <div v-else-if="seg.state === 'done' || seg.state === 'skipped'" class="frame-note ok">
      {{ seg.note }} {{ info }}
    </div>
    <div v-else-if="seg.state === 'failed'" class="frame-note bad">抠像失败：{{ seg.note }}</div>

    <div v-if="previewSrc" class="preview-box">
      <div class="preview-title">抠像预览（棋盘格 = 透明区域，请检查发丝与手指边缘）</div>
      <img class="preview-img" :src="previewSrc" alt="抠像棋盘格预览" />
    </div>

    <div class="actions">
      <el-button
        v-if="hasVideo && !matteDone && seg.state !== 'running'"
        type="primary"
        @click="runMatte"
      >
        {{ seg.state === "failed" ? "重试抠像" : "开始抠像" }}
      </el-button>
      <el-button
        v-if="hasVideo && !matting && !uploading"
        @click="skipMatte"
      >
        跳过抠像（卡片兜底出片）
      </el-button>
      <span class="spacer" />
      <el-button type="primary" :disabled="!hasVideo" @click="emit('next')">
        下一步：版式与生成
      </el-button>
    </div>
    <p v-if="!hasVideo" class="hint">出镜视频为必选项：抠像做画中画，跳过抠像则用纯色圆角卡片包住原视频，同样能出片。</p>
  </div>
</template>

<style scoped>
.step5 {
  display: flex;
  flex-direction: column;
  gap: 14px;
  max-width: 860px;
}
.banner {
  margin-bottom: 0;
}
.dropzone {
  border: 2px dashed #c0c4cc;
  border-radius: 10px;
  padding: 36px 20px;
  text-align: center;
  background: #fff;
  transition: border-color 0.15s, background 0.15s;
}
.dropzone.over {
  border-color: #409eff;
  background: #ecf5ff;
}
.dropzone-icon {
  font-size: 38px;
}
.dropzone-title {
  font-size: 16px;
  font-weight: 600;
  margin-top: 6px;
}
.dropzone-sub {
  color: #909399;
  font-size: 13px;
  margin-top: 6px;
}
.frame-progress {
  margin-top: -4px;
}
.frame-note {
  color: #606266;
  font-size: 13px;
}
.frame-note.ok {
  color: #67c23a;
}
.frame-note.bad {
  color: #f56c6c;
}
.preview-box {
  background: #fff;
  border: 1px solid #e4e7ed;
  border-radius: 8px;
  padding: 12px 14px;
  display: inline-flex;
  flex-direction: column;
  gap: 8px;
  align-self: flex-start;
}
.preview-title {
  font-size: 13px;
  color: #606266;
  font-weight: 600;
}
.preview-img {
  max-width: 420px;
  border-radius: 6px;
  border: 1px solid #e4e7ed;
  image-rendering: auto;
}
.actions {
  display: flex;
  align-items: center;
  gap: 10px;
}
.spacer {
  flex: 1;
}
.hint {
  color: #909399;
  font-size: 13px;
  margin: 0;
}
</style>
