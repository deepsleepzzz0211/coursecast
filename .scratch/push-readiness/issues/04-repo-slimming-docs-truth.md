# 04: 仓库瘦身与文档真相

**What to build:** 当前分支不再跟踪 spike 编译产物与残留图标文件；忽略规则覆盖 spike 的 target/work 输出；补齐行尾/二进制约定；README 的阶段、目录树、许可证与工具说明和仓库实际一致，且只使用仓库相对路径或环境变量名，不出现本机绝对路径。历史中的大对象留到 07 重写。

**Blocked by:** 01 Spike 脚本安全化与可复现、02 ffmpeg 打包工具链自检与文档

**Status:** done

依据：`docs/PRE-PUSH-REVIEW-2026-10-28.md` §2 P0-2、§4 P2、§7.3；README 无本地路径为用户明确要求。

- [x] `git ls-files` 不再包含 spike 的编译产物与输出目录；对应目录加入 `.gitignore`
- [x] 残留的 `icon.icns` 被删除或加入忽略，工作树不再有此类未跟踪文件
- [x] 新增最小 `.gitattributes`（文本/二进制/换行约定），应用后无大规模重归一化（除既有未跟踪用户产物外，`git status` 干净）
- [x] README 当前阶段、目录树、许可证/第三方组件、构建入口与仓库实际一致
- [x] README 不出现 `D:\...`、`/d/...`、`C:\Users\...`、`~/.cargo/...` 等本机路径；用户级配置与工具链位置改用通用描述或环境变量名
- [x] `AGENTS.md` 引用的 `GLOSSARY.md` 存在（补最小术语表）或引用改为实际文件
- [x] `cargo test --workspace`、web build、vue-tsc 不回归；票内记录当前 HEAD 仍含历史大对象，待 07 处理

## Comments

2026-10-28 完成。`spikes/matte-bench/target` 的 197 个跟踪文件已从索引移除并加入
ignore；`icon.icns` 删除并加入 ignore；新增 `.gitattributes`（无大规模重归一化）。
README 更新当前阶段、目录树，并保持无本机路径；新增根 `GLOSSARY.md`。web build 与
vue-tsc 通过；全量测试 255 passed / 0 failed / 6 ignored。当前 HEAD 历史仍含审查时
统计的大对象（工作树曾 143.95 MiB，当前树已移除，历史另计），待 07 重写。未触碰
用户产物 `deliverables/` 与 `docs/RENDER-ISSUES-2026-10-28.md`。
