<script setup lang="ts">
import { computed, onMounted, ref } from "vue";
import { ElMessage } from "element-plus";
import { call, assetUrl } from "../api";
import type { ProjectEntry } from "../types";

const props = defineProps<{ project: ProjectEntry; pageCount: number }>();
const emit = defineEmits<{ (e: "saved"): void }>();

const mode = ref<"paste" | "page" | "notes">("paste");
const pasteText = ref("");
const pages = ref<string[]>([]);
const thumbs = ref<string[]>([]);
const segments = ref<string[]>([]);
const mismatch = ref(false);
const adjusting = ref(false);
// 调整视图的数据：每页一个段落组 + 未分配池
const pageGroups = ref<string[][]>([]);
const unassigned = ref<string[]>([]);
const importing = ref(false);
const saving = ref(false);

const emptyPages = computed(() =>
  pages.value.map((t, i) => ({ page: i + 1, empty: !t.trim() })).filter((x) => x.empty)
);
// 全空 = 还没开始填，不弹警告；一旦有内容，剩余空页按 PRD 标黄提醒
const showEmptyHint = computed(
  () => emptyPages.value.length > 0 && pages.value.some((t) => t.trim())
);

onMounted(async () => {
  // 页数锚定课件：先取页图（thumbs + 数量），再读 script.md 对齐补齐（read 端
  // 不保留尾部空页，见 script.rs 契约），避免 pad 用到 0 页数
  try {
    const list = await call<any>("slides_list", { projectId: props.project.project_id });
    thumbs.value = list.files || [];
  } catch {
    /* ignore */
  }
  try {
    const v = await call<any>("script_get", { projectId: props.project.project_id });
    if (v.ok && v.pages.length) {
      pages.value = pad(v.pages);
      mode.value = "page";
    }
  } catch {
    /* no script yet */
  }
  if (!pages.value.length) pages.value = Array.from({ length: thumbCount() }, () => "");
});

function thumbCount(): number {
  return thumbs.value.length || props.pageCount || 0;
}

function pad(readback: string[]): string[] {
  const n = thumbCount();
  const out = readback.slice(0, n).map((t) => t || "");
  while (out.length < n) out.push("");
  return out;
}

// ---- 整段粘贴 / 备注导入：得到 segments，数量对不上走"自动均分 + 拖拽调整" ----

async function splitPaste() {
  if (!pasteText.value.trim()) {
    ElMessage.warning("请先粘贴逐字稿内容");
    return;
  }
  try {
    const v = await call<any>("script_split", { text: pasteText.value });
    if (!v.ok) {
      ElMessage.error(v.error?.message || "切分失败");
      return;
    }
    takeSegments(v.segments as string[]);
  } catch (e: any) {
    ElMessage.error(e?.error?.message || "切分失败");
  }
}

async function importNotes() {
  importing.value = true;
  try {
    const v = await call<any>("script_import_notes", { projectId: props.project.project_id });
    if (!v.ok) {
      ElMessage.error(v.error?.message || "备注导入失败");
      return;
    }
    // PPT 备注与页 1:1，直接进入逐页核对
    pages.value = pad(v.pages.map((p: string) => p || ""));
    mode.value = "page";
    ElMessage.success(`已导入 ${v.pages.length} 页备注，请逐页核对`);
  } catch (e: any) {
    ElMessage.error(e?.error?.message || "备注导入失败");
  } finally {
    importing.value = false;
  }
}

function takeSegments(segs: string[]) {
  segments.value = segs;
  const n = thumbCount();
  if (segs.length === n) {
    pages.value = pad(segs);
    mode.value = "page";
    ElMessage.success(`切分出 ${segs.length} 段，与页数一致，已逐页填入`);
  } else {
    mismatch.value = true;
    ElMessage.warning(`切分出 ${segs.length} 段，与课件 ${n} 页不一致`);
  }
}

async function autoBalance() {
  const n = thumbCount();
  try {
    const v = await call<any>("script_balance", { segments: segments.value, pageCount: n });
    if (!v.ok) {
      ElMessage.error(v.error?.message || "均分失败");
      return;
    }
    enterAdjust(v.pages as string[]);
  } catch (e: any) {
    ElMessage.error(e?.error?.message || "均分失败");
  }
}

/** 把均分结果拆成段落组，进入拖拽调整视图 */
function enterAdjust(balanced: string[]) {
  pageGroups.value = balanced.map((t) => t.split("\n").filter((s) => s.trim()));
  unassigned.value = [];
  mismatch.value = false;
  adjusting.value = true;
}

// ---- 拖拽调整：段落 chips 在页之间、以及与未分配池之间移动 ----

let dragSrc: { group: "unassigned" | number; index: number } | null = null;

function chipStart(group: "unassigned" | number, index: number, ev: DragEvent) {
  dragSrc = { group, index };
  ev.dataTransfer?.setData("text/plain", "chip");
  ev.dataTransfer && (ev.dataTransfer.effectAllowed = "move");
}

function chipDrop(group: "unassigned" | number) {
  const src = dragSrc;
  dragSrc = null;
  if (!src) return;
  const item =
    src.group === "unassigned"
      ? unassigned.value.splice(src.index, 1)[0]
      : pageGroups.value[src.group].splice(src.index, 1)[0];
  if (item === undefined) return;
  if (group === "unassigned") unassigned.value.push(item);
  else pageGroups.value[group].push(item);
}

function confirmAdjust() {
  pages.value = pad(pageGroups.value.map((g) => g.join("\n")));
  adjusting.value = false;
  mode.value = "page";
  ElMessage.success("已按当前分配填入各页");
}

// ---- 保存 ----

async function save() {
  saving.value = true;
  try {
    const v = await call<any>("script_set", { projectId: props.project.project_id, pages: pages.value });
    if (v.ok) {
      const emptyNote = emptyPages.value.length
        ? `；${emptyPages.value.map((x) => `第 ${x.page} 页`).join("、")} 将按 5 秒静默处理`
        : "";
      ElMessage.success(`逐字稿已保存（${v.page_count} 页${emptyNote}）`);
      emit("saved");
    } else {
      ElMessage.error(v.error?.message || "保存失败");
    }
  } catch (e: any) {
    ElMessage.error(e?.error?.message || "保存失败");
  } finally {
    saving.value = false;
  }
}
</script>

<template>
  <div class="step3">
    <!-- 空页提示条（PRD：空页按 5 秒静默处理并标黄，不阻塞流程） -->
    <el-alert
      v-if="showEmptyHint"
      type="warning"
      show-icon
      :closable="false"
      :title="`第 ${emptyPages.map((x) => x.page).join('、')} 页没有讲稿，将按 5 秒静默处理（不影响继续）`"
      class="empty-hint"
    />

    <!-- 拖拽调整视图：自动均分之后 -->
    <div v-if="adjusting" class="adjust">
      <div class="adjust-head">
        <span>拖动段落到目标页；空页将按 5 秒静默处理</span>
        <el-button type="primary" size="small" @click="confirmAdjust">确认分配</el-button>
      </div>
      <div class="group-grid">
        <div
          v-for="(g, i) in pageGroups"
          :key="i"
          class="page-group"
          :class="{ empty: !g.length }"
          @dragover.prevent
          @drop.prevent="chipDrop(i)"
        >
          <div class="group-title">第 {{ i + 1 }} 页</div>
          <img v-if="thumbs[i]" class="group-thumb" :src="assetUrl(thumbs[i])" />
          <span v-if="!g.length" class="group-empty-tag">空（5 秒静默）</span>
          <div
            v-for="(chip, j) in g"
            :key="chip + j"
            class="chip"
            draggable="true"
            @dragstart="chipStart(i, j, $event)"
          >
            {{ chip }}
          </div>
        </div>
      </div>
      <div class="pool" @dragover.prevent @drop.prevent="chipDrop('unassigned')">
        <div class="pool-title">未分配段落（拖到这里暂存）</div>
        <div
          v-for="(chip, j) in unassigned"
          :key="chip + j"
          class="chip"
          draggable="true"
          @dragstart="chipStart('unassigned', j, $event)"
        >
          {{ chip }}
        </div>
        <span v-if="!unassigned.length" class="pool-empty">（空）</span>
      </div>
    </div>

    <!-- 录入模式 -->
    <template v-else>
      <el-radio-group v-model="mode" class="mode-switch">
        <el-radio-button value="paste">整段粘贴</el-radio-button>
        <el-radio-button value="page">逐页填写</el-radio-button>
        <el-radio-button value="notes">PPT 备注导入</el-radio-button>
      </el-radio-group>

      <div v-if="mode === 'paste'" class="paste-pane">
        <el-input
          v-model="pasteText"
          type="textarea"
          :rows="12"
          placeholder="把整份讲稿粘贴到这里。用单独一行的 --- 分隔各页，或用「第1页」「## 第 2 页」这样的行开头"
        />
        <div class="paste-actions">
          <el-button type="primary" @click="splitPaste">切分并映射到页</el-button>
        </div>
        <el-alert
          v-if="mismatch"
          type="warning"
          show-icon
          :closable="false"
          class="empty-hint"
          title="段落数与课件页数不一致"
        >
          <div class="mismatch-body">
            共 {{ segments.length }} 段 / {{ thumbCount() }} 页。可以「自动均分」后在下一屏拖拽微调。
            <el-button type="primary" size="small" @click="autoBalance">自动均分 + 手动调整</el-button>
          </div>
        </el-alert>
      </div>

      <div v-else-if="mode === 'notes'" class="notes-pane">
        <p class="notes-hint">
          读取拖入的 pptx 课件里每页的演讲者备注（PDF 输入无此能力）。项目使用 PDF 课件时请改用整段粘贴或逐页填写。
        </p>
        <el-button type="primary" :loading="importing" @click="importNotes">
          从 PPT 备注导入
        </el-button>
      </div>

      <div v-else class="page-pane">
        <div
          v-for="(t, i) in pages"
          :key="i"
          class="page-editor"
          :class="{ empty: !t.trim() }"
        >
          <div class="page-editor-head">
            <span>第 {{ i + 1 }} 页</span>
            <span v-if="!t.trim()" class="empty-tag">空 → 5 秒静默</span>
            <img v-if="thumbs[i]" class="page-editor-thumb" :src="assetUrl(thumbs[i])" />
          </div>
          <el-input
            v-model="pages[i]"
            type="textarea"
            :rows="3"
            :placeholder="`第 ${i + 1} 页讲稿（留空则该页按 5 秒静默处理）`"
          />
        </div>
      </div>

      <div class="save-row">
        <el-button type="primary" :loading="saving" @click="save">保存逐字稿</el-button>
        <span class="save-hint">保存后进入下一步（配置声音）</span>
      </div>
    </template>
  </div>
</template>

<style scoped>
.step3 {
  display: flex;
  flex-direction: column;
  gap: 14px;
}
.empty-hint {
  margin-bottom: 0;
}
.mismatch-body {
  display: flex;
  align-items: center;
  gap: 12px;
}
.mode-switch {
  align-self: flex-start;
}
.paste-actions {
  display: flex;
  justify-content: flex-end;
  margin-top: 8px;
}
.notes-hint {
  color: #909399;
  font-size: 13px;
}
.page-pane {
  display: flex;
  flex-direction: column;
  gap: 12px;
}
.page-editor {
  background: #fff;
  border: 1px solid #e4e7ed;
  border-left: 4px solid transparent;
  border-radius: 8px;
  padding: 10px 14px;
}
.page-editor.empty {
  border-left-color: #e6a23c;
  background: #fdf6ec;
}
.page-editor-head {
  display: flex;
  align-items: center;
  gap: 10px;
  font-weight: 600;
  margin-bottom: 6px;
}
.page-editor-thumb {
  margin-left: auto;
  width: 96px;
  border-radius: 4px;
  border: 1px solid #e4e7ed;
}
.empty-tag {
  color: #e6a23c;
  font-size: 12px;
  font-weight: 400;
}
.save-row {
  display: flex;
  align-items: center;
  gap: 12px;
  margin-top: 4px;
}
.save-hint {
  color: #909399;
  font-size: 13px;
}
.adjust-head {
  display: flex;
  justify-content: space-between;
  align-items: center;
  color: #606266;
}
.group-grid {
  display: grid;
  grid-template-columns: repeat(auto-fill, minmax(260px, 1fr));
  gap: 12px;
}
.page-group {
  background: #fff;
  border: 1px solid #e4e7ed;
  border-radius: 8px;
  padding: 10px;
  min-height: 120px;
}
.page-group.empty {
  background: #fdf6ec;
  border-color: #e6a23c;
}
.group-title {
  font-weight: 600;
  font-size: 13px;
  margin-bottom: 6px;
}
.group-thumb {
  width: 100%;
  border-radius: 4px;
  margin-bottom: 6px;
  border: 1px solid #e4e7ed;
}
.group-empty-tag {
  color: #e6a23c;
  font-size: 12px;
}
.chip {
  background: #ecf5ff;
  border: 1px solid #b3d8ff;
  color: #409eff;
  border-radius: 6px;
  padding: 4px 8px;
  margin: 4px 4px 0 0;
  font-size: 13px;
  cursor: grab;
  display: inline-block;
  max-width: 100%;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}
.pool {
  background: #fff;
  border: 2px dashed #c0c4cc;
  border-radius: 8px;
  padding: 10px;
  min-height: 60px;
}
.pool-title {
  font-size: 12px;
  color: #909399;
  margin-bottom: 6px;
}
.pool-empty {
  color: #c0c4cc;
  font-size: 12px;
}
</style>
