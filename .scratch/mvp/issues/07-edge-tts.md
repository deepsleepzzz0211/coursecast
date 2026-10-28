# 07: edge-tts 客户端（第二级）

**What to build:** edge-tts 引擎实现（WebSocket 调用微软朗读接口，非官方），预置音色 zh-CN-XiaoxiaoNeural / zh-CN-YunxiNeural 可选，产出 wav。它是降级链的第二级：MiMo 不可用但网络可达微软接口时用它，比 SAPI 音质好。

**Blocked by:** 04

**Status:** done

注意：此接口为逆向、不稳定，代码路径必须把"连接失败/被阻断"当作正常情况处理（结构化错误 → 链路继续滑向 SAPI），不能 panic 或挂死。

- [x] 网络可达时 `engine=edge` 产出可播放 wav
      2026-10-25 真实接口实测：4.61s 音频 / 1.08s，zh-CN-XiaoxiaoNeural（v2 令牌）
- [x] 阻断/超时场景：快速失败（有界超时），错误结构化，降级链继续
      403 → `engine_blocked`（同一 5 分钟窗口不 0ms 重试；服务器 Date 跨窗口时
      抖动退避重试一次）；连接拒绝/挂死/无音频均为结构化错误，链路滑向 SAPI
- [x] 音色参数可配置，默认 Xiaoxiao

跟进（2026-10-25）：微软轮换 Sec-MS-GEC 到 v2（v1 403）。已从 edge-tts-rust 0.1.7
（MIT）移植 CS64+SHA-256 令牌（含固定测试向量），修正二进制帧“头部长度不含 2 字节
前缀”的解析 bug；真实网络哨兵测试转绿。专利声明与风险见 `docs/adr/0002-edge-tts-v2-token.md`。
