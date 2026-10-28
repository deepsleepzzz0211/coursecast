# 09: 抠像（RVM 实装）

**What to build:** `person_matte` 阶段端到端：上传的出镜视频 → 逐帧 RVM MobileNetV3 推理（ort，CPU）→ alpha 上采样回原分辨率 → ProRes 4444 mov 保 alpha，后处理 despill + 1px 腐蚀去白边；CLI 返回预览帧路径（棋盘格上叠 alpha 供肉眼确认边缘）。推理后端封装为 trait（许可切换的前提）。512 宽推理（downsample_ratio 0.375），时序状态逐帧推进，**不要把 1080p 直接喂模型**。

**Blocked by:** 01

**Status:** ready-for-agent

注意：worktree 需从主检出复制 vendor 里的 RVM onnx；测试视频用 ffmpeg 合成（人物形状用几何图形模拟即可验证管道，真实质量人工评估）。抠像许可路线是独立票（15），本票只保证 RVM 实装可用。

- [ ] 合成测试视频 → alpha mov 产出，尺寸与源一致、含透明通道
- [ ] 512p 推理：30s@720p 素材处理耗时有界（< 2 分钟，实测基线 8s）
- [ ] 预览帧（棋盘格 + alpha）落盘，路径进 JSON
- [ ] matte 指纹 = 视频哈希 + 模型哈希 + downsample_ratio
- [ ] 推理 trait 抽象就位（换后端不需要动调用方）
