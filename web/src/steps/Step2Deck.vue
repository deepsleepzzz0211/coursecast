<script setup lang="ts">
import { onMounted, onUnmounted, ref } from "vue";
import { ElMessage } from "element-plus";
import { call, assetUrl, inTauri } from "../api";
import { wizard } from "../wizard";
import type { ProjectEntry } from "../types";

const props = defineProps<{ project: ProjectEntry }>();
const emit = defineEmits<{ (e: "rendered", count: number): void }>();

const rendering = ref(false);
const dragOver = ref(false);
const slides = ref<string[]>([]);
const error = ref<string | null>(null);
const lastNote = ref<string>("");

// Tauri 2: file paths arrive through the webview drag-drop event (the DOM drop
// event carries no usable path inside the shell). In a plain browser the DOM
// fallback feeds the mock layer.
let unlisten: (() => void) | null = null;

onMounted(async () => {
  await restore();
  if (inTauri) {
    const { getCurrentWebview } = await import("@tauri-apps/api/webview");
    unlisten = await getCurrentWebview().onDragDropEvent((ev: any) => {
      if (ev.payload.type === "over") {
        dragOver.value = true;
      } else if (ev.payload.type === "leave") {
        dragOver.value = false;
      } else if (ev.payload.type === "drop") {
        dragOver.value = false;
        const path: string | undefined = ev.payload.paths?.[0];
        if (path) render(path);
      }
    });
  }
});
onUnmounted(() => unlisten?.());

async function restore() {
  // 恢复已渲染的页图，只填充本组件状态——切步由用户操作（渲染成功/下一步）驱动
  try {
    const v = await call<any>("slides_list", { projectId: props.project.project_id });
    if (v.ok && v.count > 0) {
      slides.value = v.files;
      wizard.setPageCount(v.count);
    }
  } catch {
    /* nothing rendered yet */
  }
}

async function render(sourcePath: string) {
  if (!/\.(pdf|pptx)$/i.test(sourcePath)) {
    error.value = "仅支持拖入 .pdf 或 .pptx 课件文件";
    return;
  }
  rendering.value = true;
  error.value = null;
  try {
    const v = await call<any>("slide_render", {
      projectId: props.project.project_id,
      sourcePath,
    });
    if (v.ok) {
      const list = await call<any>("slides_list", { projectId: props.project.project_id });
      slides.value = list.files;
      lastNote.value = v.skipped ? "课件未变化，直接复用已渲染的页图" : "";
      ElMessage.success(`已渲染 ${v.page_count} 页课件`);
      emit("rendered", v.page_count);
    } else {
      error.value = v.error?.message || "渲染失败";
    }
  } catch (e: any) {
    error.value = e?.error?.message || "渲染失败";
  } finally {
    rendering.value = false;
  }
}

// browser fallback (mock mode): DOM drop carries the file name only
async function onDomDrop(ev: DragEvent) {
  dragOver.value = false;
  const file = ev.dataTransfer?.files?.[0];
  if (file) await render(file.name);
}
</script>

<template>
  <div class="step2">
    <el-alert
      v-if="error"
      :title="error"
      type="error"
      show-icon
      :closable="true"
      class="error-bar"
    />
    <el-alert
      v-if="lastNote"
      :title="lastNote"
      type="info"
      show-icon
      :closable="true"
      class="error-bar"
    />

    <div
      class="dropzone"
      :class="{ over: dragOver, busy: rendering }"
      @dragover.prevent="dragOver = true"
      @dragleave.prevent="dragOver = false"
      @drop.prevent="onDomDrop"
      v-loading="rendering"
      element-loading-text="正在渲染课件页…"
    >
      <template v-if="!slides.length">
        <div class="dropzone-icon">📄</div>
        <div class="dropzone-title">把课件拖到这里</div>
        <div class="dropzone-sub">
          PDF 直接使用；pptx 将尝试本机 PowerPoint 转换，失败时请先另存为 PDF 再拖入
        </div>
      </template>
      <template v-else>
        <div class="dropzone-title">已渲染 {{ slides.length }} 页（可重新拖入替换课件）</div>
      </template>
    </div>

    <div v-if="slides.length" class="thumbs">
      <figure v-for="(f, i) in slides" :key="f" class="thumb">
        <img :src="assetUrl(f)" :alt="`第 ${i + 1} 页`" loading="lazy" />
        <figcaption>第 {{ i + 1 }} 页</figcaption>
      </figure>
    </div>
  </div>
</template>

<style scoped>
.step2 {
  display: flex;
  flex-direction: column;
  gap: 16px;
}
.error-bar {
  margin-bottom: 0;
}
.dropzone {
  border: 2px dashed #c0c4cc;
  border-radius: 10px;
  padding: 44px 20px;
  text-align: center;
  background: #fff;
  transition: border-color 0.15s, background 0.15s;
  min-height: 160px;
}
.dropzone.over {
  border-color: #409eff;
  background: #ecf5ff;
}
.dropzone-icon {
  font-size: 40px;
}
.dropzone-title {
  font-size: 16px;
  font-weight: 600;
  margin-top: 8px;
}
.dropzone-sub {
  color: #909399;
  font-size: 13px;
  margin-top: 6px;
}
.thumbs {
  display: grid;
  grid-template-columns: repeat(auto-fill, minmax(200px, 1fr));
  gap: 12px;
}
.thumb {
  margin: 0;
  background: #fff;
  border: 1px solid #e4e7ed;
  border-radius: 8px;
  overflow: hidden;
}
.thumb img {
  width: 100%;
  aspect-ratio: 16 / 9;
  object-fit: contain;
  background: #f5f7fa;
  display: block;
}
.thumb figcaption {
  font-size: 12px;
  color: #909399;
  text-align: center;
  padding: 4px;
}
</style>
