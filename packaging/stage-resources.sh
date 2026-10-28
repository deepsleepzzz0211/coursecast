#!/usr/bin/env bash
# ticket 14：把随包运行时汇编进 packaging/stage/（安装根布局：全部在 exe 旁）。
# 来源：本仓库构建产物 + 主仓库 vendor/（gitignored，t14 检出里没有）+ 系统凭据无关的开发机组件。
set -euo pipefail

HERE="$(cd "$(dirname "$0")" && pwd)"
ROOT="$(cygpath -m "$HERE/..")"
MAIN_REPO="${MAIN_REPO:-$(cygpath -m "$ROOT/../..")}"   # t14 在 .worktrees/ 下，主仓库在其上两级
STAGE="$ROOT/packaging/stage"
rm -rf "$STAGE" && mkdir -p "$STAGE"

need() { [ -f "$1" ] || { echo "缺少 $1（$2）" >&2; exit 1; }; }

# 1. 最小 ffmpeg（build-ffmpeg.sh 的产物）
need "$ROOT/packaging/dist/ffmpeg.exe" "先跑 packaging/build-ffmpeg.sh"
cp "$ROOT/packaging/dist/ffmpeg.exe" "$STAGE/ffmpeg.exe"

# 2. ONNX Runtime 动态库：ORT_DLL 显式指定，或开发机 venv（ort crate 同源）
if [ -z "${ORT_DLL:-}" ]; then
  CAND="$MAIN_REPO/.venv/Lib/site-packages/onnxruntime/capi/onnxruntime.dll"
  [ -f "$CAND" ] || { echo "找不到 onnxruntime.dll（设 ORT_DLL 指定）" >&2; exit 1; }
  ORT_DLL="$CAND"
fi
cp "$ORT_DLL" "$STAGE/onnxruntime.dll"
echo "onnxruntime: $ORT_DLL"

# 3. RVM 模型 + pdfium（vendor 在主仓库，gitignored）
need "$MAIN_REPO/vendor/rvm_mobilenetv3_fp32.onnx" "主仓库 vendor/"
need "$MAIN_REPO/vendor/pdfium/bin/pdfium.dll" "主仓库 vendor/pdfium/bin/"
cp "$MAIN_REPO/vendor/rvm_mobilenetv3_fp32.onnx" "$STAGE/"
cp "$MAIN_REPO/vendor/pdfium/bin/pdfium.dll" "$STAGE/"

# 4. CLI（安装包随附，干净环境验收与高级用户走同一条 cc-core 路）
cargo build --release -p cc-cli --manifest-path "$ROOT/Cargo.toml"
cp "$ROOT/target/release/cc-cli.exe" "$STAGE/cc-cli.exe"

# 5. GPL 文件
cp "$ROOT/LICENSE" "$STAGE/LICENSE"
cp "$HERE/SOURCE-OFFER.txt" "$STAGE/SOURCE-OFFER.txt"

echo ">> stage 就绪："
ls -la "$STAGE"
