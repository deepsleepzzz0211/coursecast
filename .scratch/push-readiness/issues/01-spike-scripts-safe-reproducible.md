# 01: Spike 脚本安全化与可复现

**What to build:** 运行性能与时间轴 spike 时，脚本先校验素材再清理输出；性能 spike 可用内置小样本或显式素材目录跑通，输出不再进入版本库；README 的复现命令与实际参数一致。任何人跑完 spike 后工作树保持干净。

**Blocked by:** None (can start immediately)

**Status:** done

依据：`docs/PRE-PUSH-REVIEW-2026-10-28.md` §7.5（素材路径写死、先删后失败、work 被跟踪）。

- [x] 性能 spike 支持显式素材目录（命令行参数或环境变量），并在创建/清空输出目录之前校验素材存在且完整
- [x] 无外部素材时提供可入库的小样本或自生成素材，能完成一次 smoke 运行
- [x] 时间轴 spike 保持自生成素材；两个脚本都只写入 git 忽略的输出目录
- [x] 现有 `work/` 输出不再被 git 跟踪；在干净 clone 上运行脚本后 `git status` 无新增/删除
- [x] 素材缺失或中途失败时不会留下“已清空输出但未生成新结果”的半清理状态
- [x] README 的 spike 章节写明参数、依赖与预期输出，且不包含本机绝对路径

## Comments

2026-10-28 完成。perf/timeline 改为共享 `spikes/lib.sh`：先解析/校验素材，写入
`work.tmp.<pid>`，成功后替换 `work/`；失败保留旧输出。`work/` 已从索引移除并加入
ignore；smoke（PAGES=2）、缺失素材、`..` 路径与 Windows 盘符防护均已验证。README
补充 ffmpeg+ffprobe 依赖与 `work/out.mp4`/`work/detected.txt` 产物。matte target 的
跟踪清理属于 04。干净 clone（d267dba）默认 `work/` 实跑后 `git status` 为空，
perf/timeline 的 work 跟踪数均为 0。
