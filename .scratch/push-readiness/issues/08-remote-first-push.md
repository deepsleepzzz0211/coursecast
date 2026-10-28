# 08: 远端配置与首次推送

**What to build:** 按 03 的远端与分支决策配置 origin，把 main 快进到 mvp 或按决定设置默认分支，首次推送 mvp/main；从远端 fresh clone 验证历史干净、无生成物、体积正常、HEAD 正确。推送前确认不再需要 force push（历史上尚无远端）。

**Blocked by:** 03 发布决策：仓库可见性、声纹、邮箱与远端、07 历史重写清污

**Status:** done

依据：`docs/PRE-PUSH-REVIEW-2026-10-28.md` §2 P0-1、§6 建议顺序。

- [x] origin 配置为 03 指定的 URL；推送范围与默认分支符合决策
- [x] main 与 mvp 的关系符合决策（公开仓库采用 main + orphan 快照）
- [x] fresh clone 的 `git ls-files` 无生成物，`.git` 体积与重写后记录同一量级
- [x] fresh clone 的 HEAD、分支、工作树状态符合预期；无旧 refs/大对象
- [x] 票内记录远端 URL、分支列表与首次推送结果；不执行 force push

## Comments

2026-10-29 完成。origin：`https://github.com/deepsleepzzz0211/coursecast`
（public）。首次公开内容采用 **orphan 快照**，没有推送本地旧历史：

- 远端 `main` = `8af1e55394495c6a726fb76a71ed99004ab95407`
  （`chore: CourseCast MVP public snapshot`，作者 noreply）。
- fresh clone 验证：1 个提交、226 个文件、`.git` 787 KB；无 root `tts-test/`、
  `data/`、`deliverables/`、wav/mp3/m4a；无本机路径/声纹哈希；工作树干净。
- 未执行 force push。此前误推的未脱敏快照已通过删除并重建空仓库清除。
