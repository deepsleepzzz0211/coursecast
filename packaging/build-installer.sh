#!/usr/bin/env bash
# ticket 14：一键产出 NSIS 单文件安装包并自检体积预算（≤130MB）。
# 产物：crates/cc-app/target/release/bundle/nsis/CourseCast_*-setup.exe
set -euo pipefail

HERE="$(cd "$(dirname "$0")" && pwd)"
ROOT="$(cygpath -m "$HERE/..")"
BUDGET_MB="${BUDGET_MB:-130}"

# 1. 最小 ffmpeg + 能力验收（能力不过关不出包）
if [ ! -f "$ROOT/packaging/dist/ffmpeg.exe" ]; then
  bash "$HERE/build-ffmpeg.sh"
fi
bash "$HERE/check-ffmpeg-capabilities.sh" "$ROOT/packaging/dist/ffmpeg.exe"

# 2. 资源汇编 + 前端构建 + 安装包（tauri CLI 是 web/ 的 devDependency；CWD 必须
#    是含 tauri.conf.json 的 cc-app。beforeBuildCommand 置空——vite 由本脚本用
#    绝对路径显式先跑，避免 tauri 钩子对相对 --prefix 的锚点歧义）
bash "$HERE/stage-resources.sh"
npm --prefix "$ROOT/web" run build
cd "$ROOT/crates/cc-app"
# 安装包才需要资源七件套；dev 构建用基础 tauri.conf.json（不含 resources），
# 避免主工作区没有 packaging/stage 时 tauri_build 校验失败。
"../../web/node_modules/.bin/tauri" build --config tauri.bundle.conf.json

# 3. 找到产物并核对体积预算（bundle 产在 workspace 的 target 下）
SETUP=$(ls "$ROOT/target/release/bundle/nsis/"*-setup.exe | head -1)
SIZE_MB=$(du -m "$SETUP" | cut -f1)
FF_SIZE_MB=$(du -m "$ROOT/packaging/dist/ffmpeg.exe" | cut -f1)
echo "安装包：$SETUP（${SIZE_MB} MB；预算 ≤${BUDGET_MB} MB）"
echo "最小 ffmpeg.exe：${FF_SIZE_MB} MB（预算 ≤45 MB）"
if [ "$SIZE_MB" -gt "$BUDGET_MB" ]; then
  echo "✗ 安装包体积超预算" >&2
  exit 1
fi
if [ "$FF_SIZE_MB" -gt 45 ]; then
  echo "✗ ffmpeg.exe 超 45MB 预算" >&2
  exit 1
fi
echo "✓ 体积预算通过"
echo ">> 干净环境验收：bash packaging/verify-installer.sh \"$SETUP\""
