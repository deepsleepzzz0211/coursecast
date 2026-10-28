<script setup lang="ts">
import { computed, onMounted, onUnmounted, ref } from "vue";
import { ElMessage } from "element-plus";
import { call, inTauri } from "../api";
import { wizard } from "../wizard";
import type { EdgeVoice, ProjectEntry, VoiceEngineKind } from "../types";

const props = defineProps<{ project: ProjectEntry }>();
const emit = defineEmits<{ (e: "next"): void }>();

const engine = ref<VoiceEngineKind>("mimo");
const edgeVoice = ref<EdgeVoice>("xiaoxiao");
const engineSaving = ref(false);
const keyInputRef = ref<any>(null);
const hasKey = ref(false);
const keyInput = ref("");
const savingKey = ref(false);
const refName = ref<string | null>(null);
const refDuration = ref<number | null>(null);
const uploadingRef = ref(false);
const previewing = ref(false);
const previewUrl = ref<string | null>(null);
const previewText = ref("");
const consent = ref(false);
const consentLoading = ref(true);
const error = ref<string | null>(null);
const dragOver = ref(false);
const canPreview = computed(
  () => engine.value === "edge" || (hasKey.value && !!refName.value && consent.value)
);
const footerHint = computed(() => {
  if (engine.value === "edge") return "Edge 朗读无需声音授权，生成时使用所选音色";
  return consent.value ? "授权已确认" : "MiMo 克隆需要勾选授权；也可以改用 Edge 朗读";
});

let unlisten: (() => void) | null = null;

onMounted(async () => {
  await refresh();
  if (inTauri) {
    const { getCurrentWebview } = await import("@tauri-apps/api/webview");
    unlisten = await getCurrentWebview().onDragDropEvent((ev: any) => {
      if (ev.payload.type === "over") dragOver.value = true;
      else if (ev.payload.type === "leave") dragOver.value = false;
      else if (ev.payload.type === "drop") {
        dragOver.value = false;
        const path: string | undefined = ev.payload.paths?.[0];
        if (path) uploadRef(path);
      }
    });
  }
});
onUnmounted(() => unlisten?.());

async function refresh() {
  consentLoading.value = true;
  const v = await call<any>("voice_status", { projectId: props.project.project_id });
  if (v.ok) {
    hasKey.value = v.has_key;
    refName.value = v.ref_audio ? "source/voice_ref.wav" : null;
    refDuration.value = v.ref_audio ? v.ref_audio.duration_sec : null;
    consent.value = v.consent;
    wizard.consent = !!v.consent;
    engine.value = v.tts?.engine === "edge" ? "edge" : "mimo";
    edgeVoice.value = v.tts?.edge_voice === "yunxi" ? "yunxi" : "xiaoxiao";
    wizard.engine = engine.value;
  } else {
    error.value = v.error?.message || "读取声音配置失败";
  }
  consentLoading.value = false;
}

async function persistPreference(nextEngine: VoiceEngineKind, nextVoice: EdgeVoice) {
  engineSaving.value = true;
  const v = await call<any>("voice_configure", {
    projectId: props.project.project_id,
    engine: nextEngine,
    edgeVoice: nextVoice,
  });
  if (v.ok) {
    engine.value = v.tts?.engine === "edge" ? "edge" : "mimo";
    edgeVoice.value = v.tts?.edge_voice === "yunxi" ? "yunxi" : "xiaoxiao";
    wizard.engine = engine.value;
    ElMessage.success(
      engine.value === "edge" ? "已切换为 Edge 朗读（免 Key）" : "已切换为 MiMo 克隆音色"
    );
  } else {
    ElMessage.error(v.error?.message || "保存配音方式失败");
    await refresh();
  }
  engineSaving.value = false;
}

async function setEngine(value: string | number | boolean | undefined) {
  const next: VoiceEngineKind = value === "edge" ? "edge" : "mimo";
  engine.value = next;
  await persistPreference(next, edgeVoice.value);
}

async function setEdgeVoice(value: string | number | boolean | undefined) {
  const next: EdgeVoice = value === "yunxi" ? "yunxi" : "xiaoxiao";
  edgeVoice.value = next;
  await persistPreference(engine.value, next);
}

function focusKeyInput() {
  keyInputRef.value?.focus?.();
}

async function saveKey() {
  const key = keyInput.value.trim();
  if (!key) {
    ElMessage.warning("请输入 MiMo API Key；没有 Key 可跳过本步，生成时自动使用兜底音色");
    return;
  }
  savingKey.value = true;
  const v = await call<any>("voice_configure", { projectId: props.project.project_id, apiKey: key });
  if (v.ok) {
    hasKey.value = true;
    keyInput.value = "";
    ElMessage.success("API Key 已保存到系统凭据存储（不会出现在任何文件与日志中）");
  } else {
    ElMessage.error(v.error?.message || "保存失败");
  }
  savingKey.value = false;
}

async function uploadRef(path: string) {
  if (!/\.(mp3|wav|m4a)$/i.test(path)) {
    error.value = "参考音频支持 mp3 / wav / m4a（手机录音多为 m4a，入库自动转码）";
    return;
  }
  uploadingRef.value = true;
  error.value = null;
  const v = await call<any>("voice_configure", { projectId: props.project.project_id, audioPath: path });
  if (v.ok) {
    refName.value = v.audio?.path || "source/voice_ref.wav";
    refDuration.value = v.audio?.duration_sec ?? null;
    ElMessage.success(`参考音频已入库（${Math.round(refDuration.value || 0)} 秒，24 kHz 单声道）`);
  } else {
    error.value = v.error?.message || "参考音频入库失败";
  }
  uploadingRef.value = false;
}

async function onDomDrop(ev: DragEvent) {
  dragOver.value = false;
  const file = ev.dataTransfer?.files?.[0];
  if (file) await uploadRef(file.name);
}

async function listenPreview() {
  if (engine.value === "mimo" && (!hasKey.value || !refName.value || !consent.value)) {
    ElMessage.warning("MiMo 试听需要先保存 Key、上传参考音频并勾选授权");
    return;
  }
  previewing.value = true;
  previewUrl.value = null;
  const v = await call<any>("voice_preview", { projectId: props.project.project_id });
  if (v.ok) {
    previewText.value = v.text;
    previewUrl.value = v.path_abs;
  } else {
    ElMessage.error(v.error?.message || "试听失败");
  }
  previewing.value = false;
}

async function toggleConsent(value: boolean) {
  const v = await call<any>("voice_consent_set", { projectId: props.project.project_id, granted: value });
  if (v.ok) {
    consent.value = v.granted;
    wizard.consent = !!v.granted;
  } else {
    ElMessage.error(v.error?.message || "保存授权状态失败");
  }
}
</script>

<template>
  <div class="step4">
    <el-alert v-if="error" :title="error" type="error" show-icon :closable="true" />

    <section class="card">
      <h3>配音方式</h3>
      <el-radio-group v-model="engine" :disabled="engineSaving" @change="setEngine">
        <el-radio-button value="mimo">MiMo 克隆（我的声音）</el-radio-button>
        <el-radio-button value="edge">Edge 朗读（免 Key）</el-radio-button>
      </el-radio-group>
      <p class="hint voice-hint">
        MiMo 用你的参考音频克隆音色；Edge 使用微软预置音色，不需要 Key 和声音授权。
      </p>
    </section>

    <template v-if="engine === 'mimo'">
      <el-alert
        v-if="!hasKey"
        type="warning"
        show-icon
        :closable="false"
        title="还没有 MiMo API Key"
        class="banner"
      >
        <div class="guide">
          <span>首次使用请先填写 Key：在 MiMo 控制台创建 API Key，粘贴到下方保存，再上传参考音频并勾选授权。</span>
          <div class="guide-actions">
            <el-button size="small" type="primary" @click="focusKeyInput">去填写 Key</el-button>
            <el-button size="small" plain @click="setEngine('edge')">
              改用 Edge 朗读（无需 Key）
            </el-button>
          </div>
        </div>
      </el-alert>
      <el-alert
        v-else
        type="success"
        show-icon
        :closable="false"
        title="已配置 MiMo API Key：将用你的音色克隆逐页配音"
        class="banner"
      />

      <section class="card">
        <h3>MiMo API Key</h3>
        <p class="hint">声音克隆使用小米 MiMo 接口。Key 只存入系统凭据存储，不落项目文件、不进日志。</p>
        <div class="key-row">
          <el-input
            ref="keyInputRef"
            v-model="keyInput"
            type="password"
            show-password
            placeholder="粘贴 MiMo API Key"
            :disabled="hasKey"
          />
          <el-button type="primary" :loading="savingKey" :disabled="hasKey" @click="saveKey">
            {{ hasKey ? "已保存" : "保存 Key" }}
          </el-button>
        </div>
      </section>

      <section class="card">
        <h3>参考音频</h3>
        <p class="hint">建议 10–30 秒本人清晰说话（实测 6 秒可用）。收 mp3 / wav / m4a，入库自动转 24 kHz 单声道。</p>
        <div
          class="ref-drop"
          :class="{ over: dragOver, done: !!refName }"
          @dragover.prevent="dragOver = true"
          @dragleave.prevent="dragOver = false"
          @drop.prevent="onDomDrop"
          v-loading="uploadingRef"
          element-loading-text="转码入库中…"
        >
          <template v-if="!refName">把参考音频拖到这里（mp3 / wav / m4a）</template>
          <template v-else>
            已入库：{{ refName }}<template v-if="refDuration">（{{ refDuration.toFixed(1) }} 秒）</template>，重新拖入可替换
          </template>
        </div>
      </section>

      <section class="card">
        <h3>试听克隆效果</h3>
        <p class="hint">用固定一句样例文本合成，确认声音像你本人。需先保存 Key、参考音频并勾选授权。</p>
        <el-button :loading="previewing" :disabled="!canPreview" @click="listenPreview">
          试听克隆效果
        </el-button>
        <div v-if="previewUrl" class="preview-audio">
          <div class="preview-text">「{{ previewText }}」</div>
          <audio :src="previewUrl" controls />
        </div>
      </section>

      <section class="card consent-card">
        <h3>声音授权确认</h3>
        <el-checkbox
          :model-value="consent"
          :disabled="consentLoading"
          @change="toggleConsent"
        >
          我确认参考音频为<strong>本人声音，或已获得声音本人授权</strong>，允许课影使用该音色克隆生成课程视频（合规必勾，不勾选无法生成）。
        </el-checkbox>
      </section>
    </template>

    <template v-else>
      <section class="card">
        <h3>Edge 音色</h3>
        <p class="hint">选择微软 Edge 朗读的预置音色；无需 Key、无需声音授权。</p>
        <el-radio-group v-model="edgeVoice" :disabled="engineSaving" @change="setEdgeVoice">
          <el-radio-button value="xiaoxiao">晓晓（女声）</el-radio-button>
          <el-radio-button value="yunxi">云希（男声）</el-radio-button>
        </el-radio-group>
        <p class="hint edge-note">
          Edge 使用非官方在线接口，可能被临时阻断；失败时自动滑到 Windows 系统语音。
        </p>
      </section>

      <section class="card">
        <h3>试听所选音色</h3>
        <p class="hint">用固定一句样例文本合成，确认音色和语速。</p>
        <el-button :loading="previewing" :disabled="!canPreview" @click="listenPreview">
          试听 Edge 音色
        </el-button>
        <div v-if="previewUrl" class="preview-audio">
          <div class="preview-text">「{{ previewText }}」</div>
          <audio :src="previewUrl" controls />
        </div>
      </section>
    </template>

    <div class="footer-row">
      <span class="hint">{{ footerHint }}</span>
      <el-button type="primary" @click="emit('next')">下一步：出镜视频</el-button>
    </div>
  </div>
</template>

<style scoped>
.step4 {
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
  margin: 0 0 6px;
  font-size: 15px;
}
.hint {
  color: #909399;
  font-size: 13px;
  margin: 0 0 10px;
}
.voice-hint {
  margin: 10px 0 0;
}
.guide {
  display: flex;
  flex-direction: column;
  gap: 8px;
  color: #606266;
  font-size: 13px;
}
.guide-actions {
  display: flex;
  gap: 8px;
}
.edge-note {
  margin: 10px 0 0;
}
.key-row {
  display: flex;
  gap: 10px;
}
.ref-drop {
  border: 2px dashed #c0c4cc;
  border-radius: 8px;
  padding: 22px 16px;
  text-align: center;
  color: #909399;
  transition: border-color 0.15s, background 0.15s;
}
.ref-drop.over {
  border-color: #409eff;
  background: #ecf5ff;
}
.ref-drop.done {
  border-style: solid;
  border-color: #67c23a;
  color: #67c23a;
}
.preview-audio {
  margin-top: 12px;
}
.preview-text {
  color: #606266;
  font-size: 13px;
  margin-bottom: 6px;
}
.consent-card {
  border-color: #e6a23c;
  background: #fdf6ec;
}
.footer-row {
  display: flex;
  justify-content: space-between;
  align-items: center;
}
.footer-row .hint {
  margin: 0;
}
</style>
