# 08: ffmpeg 视频合成（render 阶段）

**What to build:** `render` 阶段端到端：读 layout.toml（人像位置/宽度/形状/动效档位），ffmpeg 单次调用产出 H.264 MP4——concat 图片轨（duration 按时间轴公式、末尾重复最后一张图、`-t {T}` 硬锁）+ `voice_full.wav` + `-stream_loop -1` 循环人像 alpha 轨 overlay 画中画，`+faststart`。硬切为默认，可选每页独立 fade 到白底 0.3s（不用 xfade 串链）。抠像被跳过时走"纯色圆角卡片包原视频"分支。测试用合成素材（色块 PNG、正弦 wav、ffmpeg 生成的合成 alpha mov），不依赖真实抠像。

**Blocked by:** 02

**Status:** ready-for-agent

依据：PRD §8.6/§8.7；翻页点实测下限 33ms = 1 帧 @30fps。

- [ ] 5 页合成素材 → final.mp4；容器/视频/音轨三口径时长一致
- [ ] 翻页点误差 ≤ 33ms（复用 timeline spike 的灰度阶梯检测法）
- [ ] `-t` 硬锁生效（concat 重复末图不多出时长）
- [ ] fade 动效档位产出正确（淡到白底）
- [ ] 画中画位置/宽度/形状按 layout.toml 生效
- [ ] matte 跳过标志 → 卡片包原视频分支可用
- [ ] render 指纹 = timeline 指纹 + 版式参数 + ffmpeg 版本 + 编码参数
