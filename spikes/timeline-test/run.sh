#!/usr/bin/env bash
# 时间轴对齐实测：验证「页时长 = 本页音频 + 页间静音（末页不加）」这一公式
# 输出：每页实际翻页时间 vs 期望时间，以及总时长误差
# TIMELINE_OUT_DIR 是测试与 CI 用的输出目录入口。
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
source "$SCRIPT_DIR/../lib.sh"
cd "$SCRIPT_DIR"

OUT="${TIMELINE_OUT_DIR:-work}"
GAP=0.6
DUR=(3.0 2.0 4.0 2.5 3.5)
COLORS=(0x101010 0x404040 0x808080 0xC0C0C0 0xF0F0F0)
N=${#DUR[@]}

need_cmd ffmpeg
need_cmd ffprobe
need_cmd awk

resolve_out_dir "$SCRIPT_DIR" "$OUT"
begin_staging "$OUT_ABS"
cd "$TMP"

# 1) 生成幻灯片（纯色，便于场景检测）
for i in $(seq 0 $((N-1))); do
  ffmpeg -y -v error -f lavfi -i "color=c=${COLORS[$i]}:s=1920x1080" -frames:v 1 "slide_$(printf %03d $((i+1))).png"
done

# 2) 生成每页音频（时长精确）
for i in $(seq 0 $((N-1))); do
  ffmpeg -y -v error -f lavfi -i "sine=frequency=$((300+i*80)):duration=${DUR[$i]}" \
    -ar 24000 -ac 1 "a_$(printf %03d $((i+1))).wav"
done
ffmpeg -y -v error -f lavfi -i "anullsrc=r=24000:cl=mono" -t "$GAP" gap.wav

# 3) 拼接整轨：页间插 GAP 静音
: > alist.txt
for i in $(seq 1 $N); do
  echo "file 'a_$(printf %03d $i).wav'" >> alist.txt
  if [ "$i" -lt "$N" ]; then echo "file 'gap.wav'" >> alist.txt; fi
done
ffmpeg -y -v error -f concat -safe 0 -i alist.txt -c copy voice_full.wav

# 4) 生成 concat 清单：每页时长 = 本页音频 + GAP（末页不加）
: > clist.txt
TOTAL=0
for i in $(seq 1 $N); do
  D=${DUR[$((i-1))]}
  if [ "$i" -lt "$N" ]; then SD=$(awk "BEGIN{printf \"%.3f\", $D+$GAP}"); else SD=$D; fi
  echo "file 'slide_$(printf %03d $i).png'" >> clist.txt
  echo "duration $SD" >> clist.txt
  TOTAL=$(awk "BEGIN{printf \"%.3f\", $TOTAL+$SD}")
done
echo "file 'slide_$(printf %03d $N).png'" >> clist.txt

echo "期望总时长: $TOTAL s"

# 5) 合成
ffmpeg -y -v error -f concat -safe 0 -i clist.txt -i voice_full.wav \
  -vf "scale=1920:1080:force_original_aspect_ratio=decrease,pad=1920:1080:(ow-iw)/2:(oh-ih)/2:color=white,setsar=1,fps=30,format=yuv420p" \
  -c:v libx264 -preset veryfast -crf 20 -c:a aac -b:a 128k \
  -t "$TOTAL" -movflags +faststart out.mp4

# 6) 校验：容器时长 / 音轨时长 / 实际翻页点
echo "--- 时长 ---"
ffprobe -v error -show_entries format=duration -of csv=p=0 out.mp4 | awk '{printf "容器时长: %.3f s\n", $1}'
ffprobe -v error -select_streams a:0 -show_entries stream=duration -of csv=p=0 out.mp4 | awk '{printf "音轨时长: %.3f s\n", $1}'
ffprobe -v error -select_streams v:0 -show_entries stream=duration -of csv=p=0 out.mp4 | awk '{printf "视频时长: %.3f s\n", $1}'

echo "--- 实际翻页点 ---"
if ! ffmpeg -hide_banner -i out.mp4 -filter:v "select='gt(scene,0.02)',showinfo" -f null - > scene.log 2>&1; then
  echo "✗ 场景检测失败" >&2
  cat scene.log >&2
  exit 1
fi
grep -o "pts_time:[0-9.]*" scene.log | cut -d: -f2 > detected.txt || : > detected.txt

EXPECTED=""
T=0
for i in $(seq 1 $((N-1))); do
  T=$(awk "BEGIN{printf \"%.3f\", $T+${DUR[$((i-1))]}+$GAP}")
  EXPECTED="$EXPECTED $T"
done

echo "期望: $EXPECTED"
echo "实测: $(tr '\n' ' ' < detected.txt)"
awk -v exp="$EXPECTED" 'BEGIN{
  n=split(exp,e," "); "cat detected.txt" | getline line; close("cat detected.txt")
}' 2>/dev/null || true

# 逐点误差
i=0
for e in $EXPECTED; do
  i=$((i+1))
  d=$(sed -n "${i}p" detected.txt)
  if [ -n "$d" ]; then
    awk -v e="$e" -v d="$d" -v i="$i" 'BEGIN{printf "第%d个翻页点: 期望 %.3f  实测 %.3f  误差 %+.0f ms\n", i, e, d, (d-e)*1000}'
  else
    echo "第${i}个翻页点: 未检出"
  fi
done

commit_staging "$SCRIPT_DIR" "$OUT_ABS"
echo ">> 输出：$OUT_ABS/out.mp4"
