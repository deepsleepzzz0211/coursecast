# 06: 三级降级链 + SAPI 兜底

**What to build:** 引擎选择逻辑：MiMo（有 Key）→ edge-tts → Windows SAPI。无 Key 或 MiMo 不可用时自动落到下一级，降级事件写进 project.json 并出现在 CLI JSON 的提示字段，流程不中断。SAPI 兜底通过 PowerShell System.Speech 离线产出 wav（真正底线，不依赖网络）。

**Blocked by:** 04

**Status:** ready-for-agent

依据：PRD §8.3——edge-tts 国内已有被阻断的报告，SAPI 是底线，必须保留。

- [ ] 无 Key 时 engine=mimo 请求自动降级到 SAPI 并产出 wav
- [ ] 降级提示出现在 CLI JSON 与 project.json（用户可见的明确提示条数据源）
- [ ] SAPI 合成中文文本 → 可播放的 wav（本机 Windows 即可验证）
- [ ] 选择逻辑单测覆盖：有 Key/无 Key/引擎失败滑落/全部失败的结构化错误
