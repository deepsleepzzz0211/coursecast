# 01: 运行时定位 module：三个壳共用一个环境探测与引擎构造路径

**What to build:** 三个壳（CLI/MCP/Tauri）不再各自探测 ORT 运行时、RVM 模型、打包目录和构造 TTS/抠像引擎；cc-core 的运行时定位 module 负责按 env → 打包目录 → dev fallback 解析路径，并构造真实引擎与 fake seam。用户可观察：同一台机器上三个壳对同一项目走同一套运行时；CLI 也能用 `CC_MATTE_ENGINE=fake` 离线抠像；安装包 exe 旁的文件仍被找到。

**Blocked by:** None (can start immediately)

**Status:** done

- [x] CLI/MCP/app 的 ORT dylib、RVM 模型、打包目录探测都由同一个 module 提供；壳里不再有第二份 ensure/model_path 逻辑。
- [x] `CC_MATTE_ENGINE=fake` 在三个壳都可用（CLI 补齐）；`CC_TTS_ENGINE=fake` 的行为只定义一次。
- [x] 打包形态仍能解析 exe 旁的 ffmpeg/onnxruntime/RVM/pdfium；显式环境变量优先。
- [x] 删掉无人调用的引擎构造副本；workspace 测试全绿。
- [x] 有一个测试钉住 fake seam 的跨壳一致性与打包路径优先级。
