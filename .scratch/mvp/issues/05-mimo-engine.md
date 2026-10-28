# 05: MiMo voiceclone 真实引擎

**What to build:** `engine=mimo` 真实客户端：OpenAI 兼容 chat/completions，`api-key` 头鉴权，待念文本在 assistant 消息、user 放风格指令，参考音频以 `data:audio/wav;base64,…` 内联（编码后 URI 内存缓存），输出 24kHz mono pcm_s16le wav；每页请求 120s 超时 + 重试。`voice_configure` 负责参考音频入库：m4a/mp3 自动转 24kHz 单声道 wav（ffmpeg 参数数组），API Key 存 Windows Credential Manager，不进 project.json / 日志 / 返回值。

**Blocked by:** 04

**Status:** ready-for-agent

注意：真实 API 不进常规测试（spec 测试决策）。请求形状用本地 mock HTTP 服务器单测；真实验证由协调者提供 Key 跑一页（人工确认），参照 spikes/tts-test 的基线（RTF 0.36）。

- [ ] mock HTTP 断言请求形状：模型名、api-key 头、assistant/user 消息、audio.format=wav、data URI 前缀
- [ ] mock 返回 base64 wav → 正确解码落盘
- [ ] m4a 参考音频入库 → 24kHz 单声道 wav（体积比 48kHz 立体声小 4 倍）
- [ ] Key 存取走 Credential Manager；日志与 JSON 输出脱敏
- [ ] 401 返回结构化错误（降级链票与前端引导依赖此信号）
- [ ] 真实 Key 一页合成冒烟（协调者执行）：wav 时长 > 0、24kHz mono
