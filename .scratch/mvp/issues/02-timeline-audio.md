# 02: 时间轴公式 + 音轨拼接

**What to build:** `timeline` 阶段端到端：输入每页音频 wav，产出 `concat.txt`（每页 duration 按唯一公式）与拼接后的 `voice_full.wav`（页间 0.6s 静音、末尾 1.0s 淡出），CLI `--json` 返回每页时长、翻页点、总时长。测试用合成 wav（正弦波），不依赖真 TTS。

**Blocked by:** 01

**Status:** ready-for-agent

时间轴唯一公式（来自 timeline spike 实测）：第 i 页画面时长 = d_i + g（i < N）；第 N 页画面时长 = d_N；总时长 T = Σd_i + (N-1)·g；g = 0.6s；空白页的 5 秒计入 d_i。

- [ ] 5 页（3.0/2.0/4.0/2.5/3.5s 音频）→ 总时长 17.400s，容器/音轨/concat 期望一致
- [ ] 翻页点误差 ≤ 33ms（1 帧 @30fps，帧对齐理论下限）
- [ ] 页间 0.6s 静音、末尾 1.0s 淡出可从产物中验证
- [ ] ffmpeg 用参数数组调用（不拼 shell 字符串）
- [ ] timeline 指纹（所有页音频时长 + g + 页数）写入 project.json
