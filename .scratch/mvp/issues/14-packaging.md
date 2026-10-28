# 14: 最小 ffmpeg 构建 + 安装包

**What to build:** 自定义最小 ffmpeg 构建（只保留 libx264 + aac + concat + scale/pad/overlay + ProRes 解码等所需能力，目标单 exe ≤ 45MB，**不打包 ffprobe**——时长从 wav 头解析），构建脚本入库可复现；pdfium.dll、RVM onnx、ORT 运行时随包进 resources；Tauri bundler 出 NSIS 单文件安装包；干净环境安装后全流程可用（fake 引擎），体积实测 ≤ 130MB。

**Blocked by:** 13

**Status:** done

依据：验证报告 §4（现成构建 110–217MB 全超标，必须自定义）；spec 分发节。

- [x] 构建脚本一键产出最小 ffmpeg，验证所需全部滤镜/编码器可用
      `packaging/build-ffmpeg.sh`（zlib 1.3.1 → x264 stable → ffmpeg 8.1.1 静态链）→
      **8.63 MiB**（9,051,662 B，预算 45MB）；`packaging/check-ffmpeg-capabilities.sh` 8/8 全绿
- [x] 安装包体积实测 ≤ 130MB（记录实际数字进验证报告）
      `CourseCast_0.1.0_x64-setup.exe` **27.74 MiB**（29,091,319 B，预算 130MB），
      已记入 docs/VERIFICATION-2026-10-24.md §4.1
- [x] 干净目录（无 Python/Node/ffmpeg 的环境模拟）安装后 fake 引擎全流程出片
      `packaging/verify-installer.sh`：静默安装到临时目录 + `env -i`（PATH 仅 System32）下
      用包内 cc-cli/ffmpeg 建项目→解析 3 页 PNG→fake TTS→时间轴→render 出片（32KB）
- [x] 包内不含 ffprobe；时长解析走 wav 头（verify 脚本自检包内容）

备注：SOURCE-OFFER.txt 的发行方联系方式暂留 `__发行方补充__` 占位（无真实发行方信息可填）；
bundle resources 七件套（ffmpeg/onnxruntime/RVM/pdfium/cc-cli/LICENSE/SOURCE-OFFER）随包安装到 exe 旁。
