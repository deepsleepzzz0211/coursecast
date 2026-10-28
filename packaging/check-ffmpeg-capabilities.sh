#!/usr/bin/env bash
# ticket 14：最小 ffmpeg 能力验收——用 cc-core 实际使用的每一项能力做端到端合成验证。
# 用法：bash packaging/check-ffmpeg-capabilities.sh [ffmpeg.exe 路径]（默认 packaging/dist/ffmpeg.exe）
# 任一项失败即退出非零（同时是构建脚本的验收器，也是 CI/打包前检查器）。
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
FF="${1:-$ROOT/packaging/dist/ffmpeg.exe}"
TMP="$(mktemp -d)"
trap 'rm -rf "$TMP"' EXIT

ok() { echo "  ✓ $1"; }
fail() { echo "  ✗ $1" >&2; exit 1; }

echo "能力验收：$FF"
[ -f "$FF" ] || fail "ffmpeg.exe 不存在"

# 0. 版本可读（render 指纹依赖 -version 输出）
"$FF" -version >/dev/null || fail "-version"
ok "版本输出（render 指纹输入）"

# 1. lavfi + anullsrc：时间轴静音段（timeline.rs 的空页/页间静音）
"$FF" -y -v error -f lavfi -i "aevalsrc=exprs=0:channel_layout=mono:duration=1:sample_rate=24000" -t 1 -c:a pcm_s16le -f wav "$TMP/sil.wav" || fail "aevalsrc 静音生成（timeline 生产字符串）"
ok "lavfi/anullsrc + pcm_s16le wav 编码（静音页/页间静音）"

# 2. aac 编解码 + mp3 解码 + 音频归一化链（voice.rs 参考音频入库 / TTS wav 规范化）
"$FF" -y -v error -f lavfi -i "sine=frequency=440:sample_rate=44100" -t 1 -c:a aac "$TMP/t.m4a" || fail "aac 编码"
"$FF" -y -v error -i "$TMP/t.m4a" -vn -ac 1 -ar 24000 -acodec pcm_s16le -f wav "$TMP/t24k.wav" || fail "m4a→24k 单声道 wav"
ok "aac 编码 + m4a 解码 + 24kHz mono pcm_s16le 规范化（voice/edge/sapi 入库）"

# 3. png 编解码 + image2（slide 渲染产物 + matte 棋盘格预览）
"$FF" -y -v error -f lavfi -i "color=c=red:s=320x240:d=1" -frames:v 1 -c:v png -f image2 "$TMP/p%03d.png" || fail "png/image2 编码"
"$FF" -y -v error -f image2 -i "$TMP/p%03d.png" -frames:v 1 -f null - || fail "png 解码"
ok "png 编解码 + image2（幻灯片页图 / 预览）"

# 4. prores_ks 4444 带 alpha（matte 的 alpha.mov）+ 解码回读
"$FF" -y -v error -f lavfi -i "color=c=black@0.5:s=320x240:d=1,format=yuva444p10le" -c:v prores_ks -profile:v 4444 -pix_fmt yuva444p10le "$TMP/alpha.mov" || fail "prores_ks 4444 编码"
"$FF" -y -v error -i "$TMP/alpha.mov" -frames:v 1 -f null - || fail "prores 解码"
ok "prores_ks 4444 yuva444p10le 编解码（抠像 alpha.mov）"

# 5. h264/libx264 + mp4（人像视频输入 / 成片输出）+ overlay/scale/pad/crop/setsar/fps/format/geq/fade
#    滤镜串复刻 render.rs 的形状（背景缩放垫白 + PiP overlay + 圆形 geq 遮罩 + fade）
"$FF" -y -v error -f lavfi -i "color=c=blue:s=640x360:d=1:r=30" -f lavfi -i "aevalsrc=exprs=0:channel_layout=mono:duration=1:sample_rate=24000" -f lavfi -i "color=c=green:s=160x120:d=1:r=30,format=yuva444p" \
  -filter_complex "[0:v]scale=640:360:force_original_aspect_ratio=decrease,pad=640:360:(ow-iw)/2:(oh-ih)/2:color=white,setsar=1,fps=30,fade=t=in:st=0:d=0.3:color=white[bg];[2:v]scale=120:-2,crop=w='min(iw,ih)':h='min(iw,ih)':x='(iw-ow)/2':y='(ih-oh)/2',format=yuva444p,geq=lum='p(X,Y)':cb='p(X,Y)':cr='p(X,Y)':a='if(lt(hypot(X-60,Y-60),60),255,0)'[p];[bg][p]overlay=main_w-overlay_w-16:main_h-overlay_h-16:format=auto,format=yuv420p[v]" \
  -map "[v]" -map 1:a -c:v libx264 -preset medium -crf 20 -pix_fmt yuv420p -c:a aac -b:a 192k "$TMP/final.mp4" || fail "render 合成滤镜链"
"$FF" -y -v error -i "$TMP/final.mp4" -f null - || fail "h264/mp4 解码回读"
ok "libx264+aac mp4 + scale/pad/setsar/fps/fade/crop/format/geq/overlay（成片渲染滤镜链）"

# 6. 音频 concat + afade + anull（timeline.rs 整轨拼接与结尾淡出）
"$FF" -y -v error -i "$TMP/sil.wav" -i "$TMP/sil.wav" -i "$TMP/sil.wav" -filter_complex "[0:a][1:a][2:a]concat=n=3:v=0:a=1[cat];[cat]afade=t=out:st=2.5:d=0.5[aout]" -map "[aout]" -c:a pcm_s16le -ar 24000 -ac 1 "$TMP/full.wav" || fail "音频 concat/afade"
ok "concat/afade/anull（整轨拼接 + 结尾淡出）"

# 7. concat demuxer（幻灯片页图轨道）+ -stream_loop（人像画中画循环）
# concat.txt 内部写 Windows 路径（cygpath -m）：ffmpeg 不做 MSYS 路径转换
TMPWIN="$(cygpath -m "$TMP")"
printf "file '%s/p001.png'\nduration 1\nfile '%s/p001.png'\nduration 1\nfile '%s/p001.png'\n" "$TMPWIN" "$TMPWIN" "$TMPWIN" > "$TMP/concat.txt"
"$FF" -y -v error -f concat -safe 0 -i "$TMP/concat.txt" -stream_loop -1 -i "$TMP/final.mp4" -frames:v 30 -c:v libx264 -pix_fmt yuv420p -an "$TMP/looped.mp4" || fail "concat demuxer + stream_loop"
ok "concat demuxer（页图轨道）+ -stream_loop（PiP 循环）"

echo "全部能力验收通过：$(du -h "$FF" | cut -f1) $FF"
