# 06: 重写前安全快照

**What to build:** 在历史重写前创建可验证的恢复点：全量 git bundle（所有分支）、HEAD/分支/远端记录、恢复步骤；从 bundle 恢复到临时目录后与当前仓库一致。bundle 含重写前的敏感数据，存放位置受限，重写验证后按 03 的决定销毁或保留。

**Blocked by:** 04 仓库瘦身与文档真相、05 真人声纹与旧 tts-test 清理

**Status:** done

依据：`docs/PRE-PUSH-REVIEW-2026-10-28.md` §6 建议顺序；07 的前置。

- [x] 在仓库外生成全量 bundle（所有分支），记录 SHA-256、HEAD、分支列表与无远端状态
- [x] 从 bundle 恢复到临时目录，`git rev-parse HEAD`、`git status`、`git ls-files` 与当前一致
- [x] 恢复步骤写入票内，明确 bundle 含大文件/声纹且访问受限
- [x] 重写完成并验证后，按 03 的隐私决定删除或保留 bundle

## Comments

2026-10-29 完成。远端此时为空（origin 已配置，无 refs），当前 HEAD
`816d70079b81abfc6704a23e42f6a183db9c2b87`，tree
`0479370403b68aea1885f5b9121b5ee987808a0d`。

- bundle：仓库外受限目录（52 MB），
  sha256 `aaa14672e1cdc7fce63253a992cfcf4df4d1a33e61a67c3a8c23305774e944e6`。
- 恢复验证：从 bundle clone 到临时目录，HEAD/tree 与当前完全一致。
- 该 bundle 含历史中的声纹与大对象，只作本地恢复点，勿上传；03 选择 orphan
  快照后不需要重写历史，bundle 保留作本地备份。
