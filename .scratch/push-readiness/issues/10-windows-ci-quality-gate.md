# 10: Windows CI 质量门禁

**What to build:** 在 windows-latest 上建立 push/PR 质量门禁：Rust fmt/clippy/test、Web 构建与 vue-tsc、npm/cargo 依赖审计；处理干净 clone 的测试前提（web 构建顺序、ffmpeg、RVM 模型），让 CI 在默认分支稳定变绿。安装包构建不在本票范围。

**Blocked by:** 08 远端配置与首次推送

**Status:** ready-for-agent

依据：`docs/PRE-PUSH-REVIEW-2026-10-28.md` §1 门禁结果、§3 P1（无 CI、cargo audit 缺口）。

- [ ] workflow 在 push/PR 触发，windows-latest 上运行 `cargo fmt --check`、`clippy --workspace --all-targets`、`test --workspace`
- [ ] 先安装 Node 依赖并构建 web，再跑 cc-app 相关测试；ffmpeg 与 RVM 模型在干净 runner 上可获取，或环境依赖测试显式 `#[ignore]` 并在票内说明
- [ ] `npm ci/build`、`vue-tsc --noEmit`、`npm audit`、`cargo audit`（或等价 rustsec 检查）纳入门禁；审计告警有修复或书面豁免
- [ ] 依赖缓存生效；CI 全绿且失败可见；默认分支保护（若权限允许）阻止红灯合并
- [ ] 不构建 NSIS/ffmpeg；与 09 的本地验收职责分离
