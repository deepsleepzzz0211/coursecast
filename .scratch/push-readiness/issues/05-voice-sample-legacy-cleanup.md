# 05: 真人声纹与旧 tts-test 清理

**What to build:** 按 03 的决定，当前树不再包含可复刻真人的参考音频与克隆输出；早期 `tts-test/` 目录退役，或只保留不敏感的脚本并改读 git 忽略的本地数据目录。仓库中不出现声纹文件，历史清理留到 07。

**Blocked by:** 03 发布决策：仓库可见性、声纹、邮箱与远端、04 仓库瘦身与文档真相

**Status:** done

依据：`docs/PRE-PUSH-REVIEW-2026-10-28.md` §2 P0-3；用户 2026-10-29 明确“声纹不进仓库”。

- [x] 当前树无真人参考音频、克隆输出或等价声纹数据（`git ls-files` 可证）
- [x] 若 03 决定保留：样本只存 git 忽略的 data 目录，授权记录不入库，脚本改为可配置路径
- [x] 若 03 决定移除：旧 `tts-test/` 整体退役；仍被引用的测试能力由 `spikes/tts-test` 或正式测试覆盖
- [x] `.gitignore` 覆盖本地声纹/克隆输出；README 与脚本不指向本机绝对路径
- [x] 相关 Rust/Python 测试或 smoke 不回归

## Comments

2026-10-29 完成。用户决定声纹不进仓库。根 `tts-test/` 已从索引与工作树移除
（`output.wav` 移到 ignored `data/tts-test/legacy-output.wav`；参考音频继续只保留在
ignored `data/tts-test/voice_ref.wav`）；README 目录树同步更新，`/tts-test/` 加入
ignore。`spikes/tts-test/run.py` 仍只读 ignored `data/`，不受影响。历史中的旧文件
仍需 07 重写后才会从公开历史消失。
