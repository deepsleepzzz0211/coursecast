# 08: voice_synthesize 命令：引擎策略、授权门、结果信封只写一次

**What to build:** 一个命令 operation 负责引擎选择与默认值、声音授权门、降级链构造、逐页合成调用和结果信封；CLI/MCP 的 voice_synthesize 与 app 生成里的语音段都调用它。用户可观察：同一项目在三个壳里得到相同的合成结果；未勾选《声音授权确认》时三个壳给出同一个结构化错误。

**Blocked by:** 01, 02, 05

**Status:** done

- [x] CLI/MCP/app 的 voice_synthesize 返回相同键与错误码；app 的生成语音段使用同一个 operation。
- [x] 未授权时三个壳都返回同一个结构化错误；任何路径都不能在未授权时调用克隆引擎。
- [x] 默认引擎策略只定义一次；fake/mimo/edge/sapi 选择与未知引擎错误三壳一致。
- [x] 降级事件、skipped/generated/failed、warnings 与阶段迁移保持不变；现有 TTS 测试通过。
- [x] 跨壳测试覆盖授权拒绝、降级与部分页失败。

## Comments

默认 preference 保留壳级差异：CLI/MCP 离线 fake（确定性 AI 工具），app 显式 mimo；选择/校验/降级链/错误码由 `runtime::tts_chain` 单一实现，见 AGENTS.md。
