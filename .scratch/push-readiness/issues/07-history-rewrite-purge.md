# 07: 历史重写清污

**What to build:** 重写所有本地分支历史，彻底移除 spike 生成物、本机路径产物、按 03 移除的声纹文件，以及按 03 决定的作者邮箱；清理旧 refs/reflog 并 gc。重写后 HEAD 树与清理后的工作树一致，完整测试与构建通过，仓库体积显著下降；因还没有远端，不需要 force push。

**Blocked by:** 03 发布决策：仓库可见性、声纹、邮箱与远端、05 真人声纹与旧 tts-test 清理、06 重写前安全快照

**Status:** wontfix

依据：`docs/PRE-PUSH-REVIEW-2026-10-28.md` §2 P0-2/P0-3、§6；执行前需 03 授权与 06 恢复点。

- [ ] 历史中不再出现 spike 编译/输出产物与相关本机路径字符串
- [ ] 按 03 的决定，历史中不再出现真人声纹/克隆输出（或保留授权记录）
- [ ] 若 03 选择重写邮箱，`git log --all --format=%ae` 只出现新地址
- [ ] 旧 ticket 分支/refs 清理或同步重写；`git fsck` 无缺失，reflog 过期，`gc --prune=now` 后无旧对象
- [ ] `git log --all --name-only` 不含被清除路径；`git count-objects -vH` 记录体积，显著低于当前约 60 MB
- [ ] HEAD 树与 05 后的工作树一致；`cargo test --workspace`、web build、vue-tsc 全绿
- [ ] 06 的 bundle 恢复验证仍可用于回滚；验证后按隐私决定处置 bundle

## Comments

2026-10-29 关闭为 superseded：03 已选择 **orphan 干净快照** 推送公开仓库，旧本地历史
不公开，因此不需要重写历史。06 的 bundle 保留为本地恢复点；当前树已由 04/05 清理。
