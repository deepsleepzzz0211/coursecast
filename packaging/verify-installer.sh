#!/usr/bin/env bash
# ticket 14：干净环境验收——静默安装到临时目录，在剥离 PATH（无 Python/Node/ffmpeg/
# 主仓库）的最小环境下用安装包内的 cc-cli.exe 走完 fake 引擎全流程并出片。
# 课件源用安装包内 ffmpeg.exe 现场生成的 PNG 图片文件夹（顺带证明它独立可用）。
# 用法：bash packaging/verify-installer.sh [CourseCast_*-setup.exe]
set -euo pipefail

HERE="$(cd "$(dirname "$0")" && pwd)"
ROOT="$(cygpath -m "$HERE/..")"
SETUP="${1:-$(ls "$ROOT/target/release/bundle/nsis/"*-setup.exe | head -1)}"
[ -f "$SETUP" ] || { echo "找不到安装包：$SETUP" >&2; exit 1; }
SETUP_WIN="$(cygpath -w "$SETUP")"

WORK="$(mktemp -d)"
INSTALL="$WORK/app"
DATA="$WORK/data"
mkdir -p "$INSTALL"
APP_PID=""
cleanup() {
  if [ -n "$APP_PID" ]; then kill -9 "$APP_PID" 2>/dev/null || true; fi
}
trap cleanup EXIT

echo ">> 静默安装到 $INSTALL"
cmd //c "$(cygpath -w "$SETUP") /S /D=$(cygpath -w "$INSTALL")"
sleep 2

echo ">> 包内自检"
[ -f "$INSTALL/cc-app.exe" ] || { echo "✗ 主程序缺失" >&2; exit 1; }
if ls "$INSTALL" | grep -qi 'ffprobe'; then
  echo "✗ 包内含 ffprobe（票面禁止）" >&2
  exit 1
fi
for f in ffmpeg.exe onnxruntime.dll rvm_mobilenetv3_fp32.onnx pdfium.dll LICENSE SOURCE-OFFER.txt cc-cli.exe; do
  [ -f "$INSTALL/$f" ] || { echo "✗ 缺 $f" >&2; exit 1; }
done
echo "  ✓ 运行时与许可文件齐全，无 ffprobe"

# 剥离环境（PATH 只有 System32）：任何对开发机 Python/Node/ffmpeg/主仓库的隐式依赖都会当场暴露
DATA_WIN="$(cygpath -w "$DATA")"
CLI="$INSTALL/cc-cli.exe"
run() { # run <cc-cli 子命令…>（--data-dir 统一注入）
  env -i \
    PATH="/c/Windows/System32" \
    SYSTEMROOT='C:\Windows' \
    TEMP="$TEMP" TMP="$TMP" \
    "$CLI" --data-dir "$DATA_WIN" "$@"
}
step() { # step <说明> <cc-cli 子命令…>：失败时把子命令输出打出来（不静默吞错）
  local what="$1"; shift
  local out
  if ! out="$(run "$@")"; then
    echo "✗ $what 失败" >&2
    printf '%s\n' "$out" >&2
    exit 1
  fi
  printf '%s' "$out"
}

echo ">> 用包内 ffmpeg 生成 3 页 PNG 课件源"
SLIDES="$WORK/slides"; mkdir -p "$SLIDES"
i=1
for c in steelblue seagreen indianred; do
  env -i PATH="/c/Windows/System32" SYSTEMROOT='C:\Windows' \
    "$INSTALL/ffmpeg.exe" -y -v error -f lavfi -i "color=c=$c:s=640x360:d=1" \
    -frames:v 1 "$(cygpath -w "$SLIDES/slide_00$i.png")"
  i=$((i+1))
done
[ -f "$SLIDES/slide_003.png" ] || { echo "✗ 包内 ffmpeg 生成课件失败" >&2; exit 1; }
echo "  ✓ 3 页 PNG 就绪"

echo ">> 干净环境 fake 全流程"
CREATE_JSON="$(step 建项目 project create --name 干净环境验收 --json)"
ID="$(printf '%s\n' "$CREATE_JSON" | sed -n 's/^ *"project_id": *"\([^"]*\)".*$/\1/p')"
[ -n "$ID" ] || { echo "✗ 建项目未返回 project_id：$CREATE_JSON" >&2; exit 1; }
step 课件解析 slide_render --project "$ID" --source "$(cygpath -w "$SLIDES")" --json >/dev/null
# CLI 无 script 子命令：逐字稿是文件即真相（script.md，--- 分页），直接落盘
printf '第一页：干净环境验收，安装包内全流程。\n---\n第二页：剥离 PATH 后出片。\n---\n第三页：完。\n' \
  > "$DATA/projects/$ID/script.md"
# render 必须有人像输入（未抠像走白卡包裹兜底），用包内 ffmpeg 现生成一段
mkdir -p "$DATA/projects/$ID/source"
env -i PATH="/c/Windows/System32" SYSTEMROOT='C:\Windows' \
  "$INSTALL/ffmpeg.exe" -y -v error -f lavfi -i "color=c=orange:s=320x240:d=2" \
  -c:v libx264 -preset ultrafast -pix_fmt yuv420p \
  "$(cygpath -w "$DATA/projects/$ID/source/person.mp4")" \
  || { echo "✗ 包内 ffmpeg 生成人像素材失败" >&2; exit 1; }
step 语音合成 voice_synthesize --project "$ID" --engine fake --json >/dev/null
step 时间轴 timeline --project "$ID" --json >/dev/null
step 渲染 render --project "$ID" --json >/dev/null
FINAL="$DATA/projects/$ID/output/final.mp4"
[ -f "$FINAL" ] || { echo "✗ 未出片：$FINAL" >&2; exit 1; }
echo "  ✓ 出片 $(du -h "$FINAL" | cut -f1)"

echo ">> 真实 Tauri 壳启动 smoke（CC_DATA_DIR + APPDATA/LOCALAPPDATA 隔离）"
APP_APPDATA="$WORK/appdata"
APP_LOCALAPPDATA="$WORK/localappdata"
APP_TEMP="$WORK/temp"
mkdir -p "$APP_APPDATA" "$APP_LOCALAPPDATA" "$APP_TEMP"
# Tauri 的 app_data_dir() 走 Windows known-folder API，不认 APPDATA 环境变量；
# 数据目录必须用 CC_DATA_DIR 覆盖，否则 smoke 会读写真实 profile。
EXISTING=$(powershell -NoProfile -Command "(Get-Process cc-app -ErrorAction SilentlyContinue | Measure-Object).Count" 2>/dev/null | tr -d '\r')
if [ "${EXISTING:-0}" -gt 0 ]; then
  echo "✗ 已有 cc-app 进程在运行；干净环境验收要求退出后再跑" >&2
  exit 1
fi
CC_DATA_DIR="$(cygpath -w "$DATA")" \
APPDATA="$(cygpath -w "$APP_APPDATA")" LOCALAPPDATA="$(cygpath -w "$APP_LOCALAPPDATA")" \
  TEMP="$(cygpath -w "$APP_TEMP")" TMP="$(cygpath -w "$APP_TEMP")" \
  "$INSTALL/cc-app.exe" > "$WORK/cc-app.stdout" 2> "$WORK/cc-app.stderr" &
APP_PID=$!
STARTED=$(date +%s)
WINDOW=0
while [ $(( $(date +%s) - STARTED )) -lt 20 ]; do
  if ! kill -0 "$APP_PID" 2>/dev/null; then break; fi
  WINDOW=$(powershell -NoProfile -Command "\$p = Get-Process cc-app -ErrorAction SilentlyContinue | Sort-Object StartTime -Descending | Select-Object -First 1; if (\$p -and \$p.MainWindowHandle -ne 0) { 1 } else { 0 }" 2>/dev/null | tr -d '\r')
  [ "${WINDOW:-0}" -gt 0 ] && break
  sleep 1
done
if ! kill -0 "$APP_PID" 2>/dev/null; then
  echo "✗ Tauri 壳启动后退出（manifest/WebView2？）" >&2
  cat "$WORK/cc-app.stderr" >&2 || true
  cat "$DATA/cc-app.log" >&2 2>/dev/null || true
  exit 1
fi
if [ "${WINDOW:-0}" -eq 0 ]; then
  echo "✗ Tauri 壳没有主窗口（进程活着但 WebView2 未初始化）" >&2
  cat "$DATA/cc-app.log" >&2 2>/dev/null || true
  exit 1
fi
echo "  ✓ Tauri 壳主窗口已出现"
kill -9 "$APP_PID" 2>/dev/null || true
wait "$APP_PID" 2>/dev/null || true
APP_PID=""

rm -rf "$WORK"
echo "✓ 干净环境验收全部通过"
