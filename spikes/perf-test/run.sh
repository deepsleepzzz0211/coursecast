#!/usr/bin/env bash
# 性能基准：用真实课件 + 合成音轨，实测 10 分钟成片的各阶段耗时。
# 用法：
#   bash spikes/perf-test/run.sh [slides_dir]
#   PAGES=3 PER_SECONDS=2 bash spikes/perf-test/run.sh   # 无素材 smoke
# 不传 slides_dir 时生成合成幻灯片，只适合 smoke；真实基准请传素材目录。
# PERF_SRC / PERF_OUT_DIR 是测试与 CI 用的等价入口。
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
source "$SCRIPT_DIR/../lib.sh"
cd "$SCRIPT_DIR"

OUT="${PERF_OUT_DIR:-work}"
SRC="${PERF_SRC:-${1:-}}"
N="${PAGES:-29}"
PER="${PER_SECONDS:-20.7}"
GAP=0.6

need_cmd ffmpeg
need_cmd ffprobe
need_cmd awk
need_cmd date

[[ "$N" =~ ^[1-9][0-9]*$ ]] || die "PAGES 必须是正整数，当前：$N"
[[ "$PER" =~ ^[0-9]+([.][0-9]+)?$ ]] || die "PER_SECONDS 必须是非负数，当前：$PER"
awk -v p="$PER" 'BEGIN{exit !(p>0)}' || die "PER_SECONDS 必须大于 0"

if [ -n "$SRC" ]; then
  if ! SRC_ABS="$(cd "$SRC" 2>/dev/null && pwd)"; then
    die "素材目录不存在：$SRC"
  fi
  SRC="$SRC_ABS"
  missing=()
  for i in $(seq 1 "$N"); do
    slide="$SRC/slide_$(printf %03d "$i").png"
    [ -f "$slide" ] || missing+=("$slide")
  done
  if [ "${#missing[@]}" -gt 0 ]; then
    printf '✗ 素材不完整，缺少 %d 张幻灯片：\n' "${#missing[@]}" >&2
    printf '  %s\n' "${missing[@]}" >&2
    exit 1
  fi
fi

resolve_out_dir "$SCRIPT_DIR" "$OUT"
begin_staging "$OUT_ABS"
cd "$TMP"

echo "== 1. 准备素材 =="
if [ -n "$SRC" ]; then
  echo "   素材: $SRC"
else
  echo "   素材: 合成幻灯片（smoke）"
fi
T0=$(date +%s.%N)
for i in $(seq 1 "$N"); do
  slide="slide_$(printf %03d "$i").png"
  if [ -n "$SRC" ]; then
    cp "$SRC/$slide" "$slide"
  else
    color=$(printf '0x%02x%02x%02x' $((i * 37 % 256)) $((i * 73 % 256)) $((i * 109 % 256)))
    ffmpeg -y -v error -f lavfi -i "color=c=${color}:s=1920x1080" -frames:v 1 "$slide"
  fi
  ffmpeg -y -v error -f lavfi -i "sine=frequency=$((200+i*20)):duration=$PER" -ar 24000 -ac 1 "a_$(printf %03d "$i").wav"
done
ffmpeg -y -v error -f lavfi -i "anullsrc=r=24000:cl=mono" -t "$GAP" gap.wav
T1=$(date +%s.%N)
echo "   生成素材: $(awk "BEGIN{printf \"%.1f\", $T1-$T0}") s"

echo "== 2. 拼接音轨 =="
: > alist.txt
for i in $(seq 1 "$N"); do
  echo "file 'a_$(printf %03d "$i").wav'" >> alist.txt
  [ "$i" -lt "$N" ] && echo "file 'gap.wav'" >> alist.txt
done
T0=$(date +%s.%N)
ffmpeg -y -v error -f concat -safe 0 -i alist.txt -c copy voice_full.wav
T1=$(date +%s.%N)
echo "   音轨拼接: $(awk "BEGIN{printf \"%.1f\", $T1-$T0}") s"

echo "== 3. 生成时间轴 =="
: > clist.txt
TOTAL=0
for i in $(seq 1 "$N"); do
  if [ "$i" -lt "$N" ]; then SD=$(awk "BEGIN{printf \"%.3f\", $PER+$GAP}"); else SD=$PER; fi
  echo "file 'slide_$(printf %03d "$i").png'" >> clist.txt
  echo "duration $SD" >> clist.txt
  TOTAL=$(awk "BEGIN{printf \"%.3f\", $TOTAL+$SD}")
done
echo "file 'slide_$(printf %03d "$N").png'" >> clist.txt
echo "   总时长: $TOTAL s"

echo "== 4. 渲染 1080p30（preset medium, crf 20）=="
T0=$(date +%s.%N)
ffmpeg -y -v error -f concat -safe 0 -i clist.txt -i voice_full.wav \
  -vf "scale=1920:1080:force_original_aspect_ratio=decrease,pad=1920:1080:(ow-iw)/2:(oh-ih)/2:color=white,setsar=1,fps=30,format=yuv420p" \
  -c:v libx264 -preset medium -crf 20 -pix_fmt yuv420p \
  -c:a aac -b:a 192k -t "$TOTAL" -movflags +faststart out.mp4
T1=$(date +%s.%N)
R=$(awk "BEGIN{printf \"%.1f\", $T1-$T0}")
echo "   渲染耗时: $R s  (成片 ${TOTAL}s，实时倍率 $(awk "BEGIN{printf \"%.2fx\", $TOTAL/$R}"))"
ls -la out.mp4 | awk '{printf "   成片体积: %.1f MB\n", $5/1048576}'

echo "== 5. 校验 =="
ffprobe -v error -show_entries format=duration -of csv=p=0 out.mp4 | awk '{printf "   容器时长: %.3f s\n", $1}'
ffprobe -v error -select_streams v:0 -show_entries stream=duration -of csv=p=0 out.mp4 | awk '{printf "   视频时长: %.3f s\n", $1}'
ffprobe -v error -select_streams a:0 -show_entries stream=duration -of csv=p=0 out.mp4 | awk '{printf "   音轨时长: %.3f s\n", $1}'

commit_staging "$SCRIPT_DIR" "$OUT_ABS"
echo ">> 输出：$OUT_ABS/out.mp4"
