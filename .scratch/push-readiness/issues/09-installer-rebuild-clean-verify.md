# 09: 安装包重建与干净环境验收

**What to build:** 清掉 cc-app 与 ffmpeg 的本地缓存后，按 README/脚本重建最小 ffmpeg 和 NSIS 安装包，确认新图标进入 exe/安装包；在干净环境跑安装、用包内 cc-cli 出片、启动真实 Tauri 主窗口。记录安装包/ffmpeg 体积与 hash。SOURCE-OFFER 未填前产物不对外分发。

**Blocked by:** 02 ffmpeg 打包工具链自检与文档

**Status:** done

依据：`docs/PRE-PUSH-REVIEW-2026-10-28.md` §1 图标提交、§3 P1、§4 P2 图标缓存。

- [x] 清理 cc-app/ffmpeg 构建缓存后，按文档完成 ffmpeg 重建并通过能力验收（同时验证 02 的文档与预检）
- [x] 安装包重建前确保图标 build script 重跑（clean 或 touch），提取 exe 图标确认是新设计
- [x] NSIS 安装包 ≤130 MB，最小 ffmpeg ≤45 MB；记录实际大小与 SHA-256
- [x] `verify-installer.sh` 全绿：静默安装、隔离 PATH 出片、真实 Tauri 主窗口
- [x] 产物路径、版本、日期记录在票内；在 11 完成前标记为不可分发

## Comments

2026-10-28 完成（HEAD 918fc0f 基础上）。构建前执行 `cargo clean -p cc-app`，删除
`packaging/dist/ffmpeg.exe` 与 `packaging/build/prefix`，因此图标 build script 重跑。
构建命令：`MAIN_REPO=<repo> bash packaging/build-installer.sh`（退出 0）。

| 产物 | 大小 | SHA-256 |
|---|---|---|
| `packaging/dist/ffmpeg.exe` | 9,051,150 B（8.63 MiB） | `1a7f9b0d5a4d9d17dfb69536afe940f3b70af2e33ae1fa46823697f8152a09c1` |
| `target/release/cc-app.exe` | 15,060,992 B | `1488d6453734304240e678b01d9dafac51e410c5f6760db814eb4aa9945f1228` |
| `target/release/bundle/nsis/CourseCast_0.1.0_x64-setup.exe` | 29,704,739 B（28.33 MiB） | `3e0fd5244dfd7cd534362a1dfd850729d148d0e1e5013e3913ae8bb96c5bda11` |

ffmpeg 8 项能力验收全过；32px 提取确认新图标（幻灯片+播放三角）已进入 exe。
`bash packaging/verify-installer.sh <setup>` 退出 0：静默安装、包内运行时/许可文件
齐全、隔离 PATH fake 全流程出片 32K、真实 Tauri 主窗口出现。

SOURCE-OFFER 仍有发行方联系方式占位；产物在 11 完成前**不可对外分发**。
