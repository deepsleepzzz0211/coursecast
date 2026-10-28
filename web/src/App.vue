<script setup lang="ts">
import { onMounted, ref } from "vue";
import { ElMessage, ElMessageBox } from "element-plus";
import { call, inTauri } from "./api";
import { wizard } from "./wizard";
import type { ProjectEntry } from "./types";
import Step1Create from "./steps/Step1Create.vue";
import Step2Deck from "./steps/Step2Deck.vue";
import Step3Script from "./steps/Step3Script.vue";
import Step4Voice from "./steps/Step4Voice.vue";
import Step5Person from "./steps/Step5Person.vue";
import Step6Generate from "./steps/Step6Generate.vue";

// 无路由库（PRD §6.4）：单页状态机 currentStep 切换六步
const view = ref<"home" | "wizard">("home");
const projects = ref<ProjectEntry[]>([]);
const active = ref<ProjectEntry | null>(null);
const currentStep = ref(1);
const loading = ref(false);

const stepTitles = ["新建项目", "上传课件", "逐字稿", "配置声音", "出镜视频", "生成"];

const stageText: Record<string, string> = {
  created: "已创建",
  parsing: "渲染中",
  parsed: "课件就绪",
  tts_running: "合成中",
  tts_done: "语音就绪",
  matte_running: "抠像中",
  matte_done: "人像就绪",
  rendering: "渲染中",
  done: "已完成",
  failed: "失败",
};

async function refreshProjects() {
  loading.value = true;
  const v = await call<{ ok: boolean; projects: ProjectEntry[]; error?: any }>("list_projects");
  if (v.ok) projects.value = v.projects;
  else ElMessage.error(v.error?.message || "读取项目列表失败");
  loading.value = false;
}

onMounted(refreshProjects);

function onCreated(p: ProjectEntry) {
  active.value = p;
  wizard.projectId = p.project_id;
  wizard.pageCount = 0;
  wizard.consent = false;
  wizard.engine = "mimo";
  wizard.hasPerson = false;
  currentStep.value = 2;
  view.value = "wizard";
  refreshProjects();
}

// 重启恢复：从落盘产物推断回到哪一步（文件即真相，没有单独存的 UI 状态）。
async function openProject(p: ProjectEntry) {
  active.value = p;
  view.value = "wizard";
  await wizard.refresh(p.project_id);
  if (!wizard.pageCount) {
    currentStep.value = 2;
    return;
  }
  if (p.progress.film) currentStep.value = 6;
  else if (p.progress.person) currentStep.value = 5;
  else if (p.progress.script) currentStep.value = 4;
  else currentStep.value = 3;
}

function onSlidesRendered(count: number) {
  wizard.setPageCount(count);
  currentStep.value = 3;
}

function onScriptSaved() {
  currentStep.value = 4;
}

async function removeProject(p: ProjectEntry) {
  try {
    await ElMessageBox.confirm(
      `删除项目「${p.name}」？项目文件与成片会一并删除，此操作不可恢复。`,
      "删除项目",
      { type: "warning", confirmButtonText: "删除", cancelButtonText: "取消" }
    );
  } catch {
    return;
  }
  const v = await call<any>("delete_project", { projectId: p.project_id, confirm: true });
  if (v.ok) {
    projects.value = projects.value.filter((x) => x.project_id !== p.project_id);
    ElMessage.success("项目已删除");
  } else {
    ElMessage.error(v.error?.message || "删除失败");
  }
}

function onBackHome() {
  view.value = "home";
  active.value = null;
  refreshProjects();
}

function advance(step: number) {
  if (!active.value) return;
  const gate = wizard.canEnter(step);
  if (!gate.ok) {
    ElMessage.warning(gate.reason || "请先完成前面的步骤");
    return;
  }
  currentStep.value = step;
}

function stepClicked(i: number) {
  advance(i + 1);
}
</script>

<template>
  <div class="app-shell">
    <header class="app-header">
      <div class="brand">
        <span class="brand-name">课影 CourseCast</span>
        <span class="brand-sub">课件 + 逐字稿 → 出镜微课</span>
      </div>
      <el-button v-if="view === 'wizard'" text @click="onBackHome">← 项目列表</el-button>
    </header>

    <!-- 项目入口：列表 + 新建 -->
    <main v-if="view === 'home'" class="home">
      <section class="home-left">
        <h2>新建项目</h2>
        <Step1Create @created="onCreated" />
      </section>
      <section class="home-right" v-loading="loading">
        <h2>我的项目（{{ projects.length }}）</h2>
        <el-empty v-if="!projects.length" description="还没有项目，从左侧开始" />
        <ul v-else class="project-list">
          <li v-for="p in projects" :key="p.project_id" class="project-card" @click="openProject(p)">
            <div class="project-main">
              <div class="project-name">{{ p.name }}</div>
              <div class="project-meta">
                {{ stageText[p.stage] || p.stage }} · {{ p.resolution }} ·
                {{ new Date(p.created_at).toLocaleString("zh-CN") }}
              </div>
            </div>
            <div class="project-progress">
              <el-tag :type="p.progress.slides ? 'success' : 'info'" size="small">课件</el-tag>
              <el-tag :type="p.progress.script ? 'success' : 'info'" size="small">逐字稿</el-tag>
              <el-button link type="danger" size="small" @click.stop="removeProject(p)">
                删除
              </el-button>
            </div>
          </li>
        </ul>
      </section>
    </main>

    <!-- 向导 -->
    <main v-else class="wizard">
      <el-steps :active="currentStep - 1" simple>
        <el-step
          v-for="(t, i) in stepTitles"
          :key="t"
          :title="t"
          :class="{ clickable: !!active }"
          @click="stepClicked(i)"
        />
      </el-steps>
      <section class="wizard-body">
        <Step2Deck
          v-if="currentStep === 2"
          :project="active!"
          @rendered="onSlidesRendered"
        />
        <Step3Script
          v-else-if="currentStep === 3"
          :project="active!"
          :page-count="wizard.pageCount"
          @saved="onScriptSaved"
        />
        <Step4Voice
          v-else-if="currentStep === 4"
          :project="active!"
          @next="advance(5)"
        />
        <Step5Person
          v-else-if="currentStep === 5"
          :project="active!"
          @next="advance(6)"
        />
        <Step6Generate v-else-if="currentStep === 6" :project="active!" />
      </section>
      <div class="wizard-nav" v-if="currentStep >= 2">
        <el-button v-if="currentStep > 2" @click="currentStep--">上一步</el-button>
        <el-button
          v-if="currentStep === 2 && wizard.pageCount"
          type="primary"
          @click="advance(3)"
        >
          下一步：逐字稿
        </el-button>
        <el-button
          v-if="currentStep === 3 && wizard.pageCount"
          type="primary"
          @click="advance(4)"
        >
          下一步：配置声音
        </el-button>
      </div>
    </main>

    <footer class="app-footer">
      <span v-if="!inTauri">浏览器预览模式（mock 数据，非真实项目）</span>
    </footer>
  </div>
</template>

<style>
html,
body,
#app {
  height: 100%;
  margin: 0;
}
body {
  font-family: "Segoe UI", "Microsoft YaHei", sans-serif;
  background: #f5f7fa;
  color: #303133;
}
.app-shell {
  height: 100%;
  display: flex;
  flex-direction: column;
}
.app-header {
  display: flex;
  align-items: center;
  justify-content: space-between;
  padding: 10px 24px;
  background: #fff;
  border-bottom: 1px solid #e4e7ed;
}
.brand-name {
  font-size: 18px;
  font-weight: 600;
  margin-right: 12px;
}
.brand-sub {
  color: #909399;
  font-size: 13px;
}
.app-footer {
  padding: 4px 24px;
  color: #c0c4cc;
  font-size: 12px;
}
.home {
  flex: 1;
  display: flex;
  gap: 24px;
  padding: 24px;
  overflow: auto;
}
.home-left,
.home-right {
  background: #fff;
  border-radius: 8px;
  padding: 20px 24px;
}
.home-left {
  width: 380px;
  align-self: flex-start;
}
.home-right {
  flex: 1;
}
.project-list {
  list-style: none;
  margin: 0;
  padding: 0;
}
.project-card {
  display: flex;
  align-items: center;
  justify-content: space-between;
  padding: 14px 16px;
  border: 1px solid #e4e7ed;
  border-radius: 8px;
  margin-bottom: 10px;
  cursor: pointer;
  transition: border-color 0.15s;
}
.project-card:hover {
  border-color: #409eff;
}
.project-name {
  font-weight: 600;
}
.project-meta {
  color: #909399;
  font-size: 12px;
  margin-top: 4px;
}
.project-progress {
  display: flex;
  gap: 6px;
}
.wizard {
  flex: 1;
  display: flex;
  flex-direction: column;
  padding: 16px 24px 24px;
  overflow: hidden;
}
.wizard-body {
  flex: 1;
  overflow: auto;
  margin-top: 16px;
}
.wizard-nav {
  display: flex;
  justify-content: flex-end;
  gap: 8px;
  padding-top: 12px;
}
.clickable {
  cursor: pointer;
}
</style>
