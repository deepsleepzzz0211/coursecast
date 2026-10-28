<script setup lang="ts">
import { ref } from "vue";
import { ElMessage } from "element-plus";
import { call } from "../api";
import type { ProjectEntry } from "../types";

const emit = defineEmits<{ (e: "created", p: ProjectEntry): void }>();

const name = ref("");
const resolution = ref("1920x1080");
const creating = ref(false);

async function submit() {
  if (!name.value.trim()) {
    ElMessage.warning("请输入项目名称");
    return;
  }
  creating.value = true;
  try {
    const v = await call<any>("create_project", { name: name.value, resolution: resolution.value });
    if (v.ok) {
      ElMessage.success(`项目「${v.name}」已创建`);
      emit("created", {
        project_id: v.project_id,
        name: v.name,
        resolution: v.resolution,
        stage: v.stage,
        created_at: new Date().toISOString(),
        progress: { slides: false, script: false },
      });
      name.value = "";
    } else {
      ElMessage.error(v.error?.message || "创建失败");
    }
  } catch (e: any) {
    ElMessage.error(e?.error?.message || "创建失败");
  } finally {
    creating.value = false;
  }
}
</script>

<template>
  <el-form label-position="top" @submit.prevent>
    <el-form-item label="项目名称">
      <el-input v-model="name" placeholder="例如：第五章 药品包装机械" maxlength="100" />
    </el-form-item>
    <el-form-item label="输出分辨率">
      <el-radio-group v-model="resolution">
        <el-radio value="1920x1080">1920 × 1080（默认）</el-radio>
        <el-radio value="1280x720">1280 × 720</el-radio>
      </el-radio-group>
    </el-form-item>
    <el-button type="primary" :loading="creating" @click="submit">创建并进入向导</el-button>
  </el-form>
</template>
