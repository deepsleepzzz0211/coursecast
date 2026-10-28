# 11: 声音配置 + 试听 + 授权批次

**What to build:** voice_configure、voice_status、voice_consent（读/写）、voice_preview 收进命令层；API Key 只入凭据存储，永不回显；试听复用 08 的引擎/降级策略；授权记录只有一条路径。用户可观察：在 app 里试听的声音和生成时用的是同一引擎策略；三个壳看到同一份授权状态。

**Blocked by:** 01, 05, 08

**Status:** done

- [x] CLI/MCP/app 的 voice_configure/status/consent/preview 共用实现与 JSON 形状（命令存在之处）。
- [x] API Key 不出现在 project.json、日志、输出或错误里；测试扫描会话输出。
- [x] 试听使用与合成相同的引擎/降级策略，并落盘固定样例句。
- [x] 授权读/写只有一条路径；generate 与 voice_synthesize 的授权门结论一致。
- [x] 现有 voice configure/consent 测试通过；参考音频转码行为不变。
