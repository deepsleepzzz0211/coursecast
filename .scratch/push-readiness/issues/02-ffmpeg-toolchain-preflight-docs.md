# 02: ffmpeg 打包工具链自检与文档

**What to build:** 在未安装 winlibs MinGW、或 `MINGW_BIN` 指向错误路径的机器上，最小 ffmpeg 构建会立即给出可操作的前置条件说明，而不是下载源码后才在 configure/make 阶段失败；README 写清 Git Bash、gcc/nasm/mingw32-make、`MINGW_BIN` 与网络要求，且不出现本机绝对路径。现有构建流程与产物保持不变。

**Blocked by:** None (can start immediately)

**Status:** done

依据：`docs/PRE-PUSH-REVIEW-2026-10-28.md` §3 P1、§7.5。

- [x] 构建开始前预检 gcc、nasm、mingw32-make（以及 Git Bash/cygpath），缺失时一次性列出缺失项并退出
- [x] `MINGW_BIN` 指向错误路径时给出明确提示（含正确用法示例），不发生下载或编译
- [x] 默认工具链可用时，现有 ffmpeg 构建与能力验收保持不变
- [x] README 打包章节记录前置工具、`MINGW_BIN`、网络下载与 SHA-256 校验
- [x] README 不出现 `D:\...`、`/d/...`、`C:\Users\...`、`~/.cargo/...` 等本机路径；工具链位置用环境变量名或通用描述
- [x] 构建失败信息能透传到安装包构建入口，使用者知道该修环境而不是猜 ffmpeg 参数

## Comments

2026-10-28 完成。`--check` 在本机通过；`MINGW_BIN=/nonexistent` 与空目录
（批量列出 gcc/nasm/mingw32-make）均在下载前失败；多余参数直接报错。默认不再依赖
固定本机路径，未设置 `MINGW_BIN` 时使用 PATH。README 打包章节补充前置工具、
`MINGW_BIN`、SHA-256 校验，并移除本机路径。
