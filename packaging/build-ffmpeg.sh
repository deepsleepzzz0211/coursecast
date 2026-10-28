#!/usr/bin/env bash
# ticket 14：自定义最小 ffmpeg 构建（可复现、单文件静态 ffmpeg.exe）
#
# 能力面与 cc-core 的实际调用一一对应（见 check-ffmpeg-capabilities.sh）：
#   解码 h264/prores/aac/mp3/pcm_s16le/png，编码 libx264/aac/pcm_s16le/png/prores_ks，
#   滤镜 scale/pad/setsar/fps/fade/crop/format/geq/overlay/concat/afade/anull/anullsrc，
#   demuxer mov/mp4/m4a/mp3/wav/concat/image2/lavfi，muxer mp4/mov/wav/image2。
# 无 ffprobe（时长一律走 wav 头解析）、无网络协议、无 ffplay/doc。
#
# 依赖：Git Bash + winlibs MinGW（gcc/nasm/mingw32-make）。用 MINGW_BIN 指向
# MinGW 的 bin 目录；未设置时使用 PATH 上的工具链。
# 构建链：zlib（PNG 解码依赖）→ x264 → ffmpeg，全部静态。
# 版本钉死：zlib 1.3.1 / x264 stable 快照 / ffmpeg 8.1.1（与验证报告 §4 实测版本一致）。
set -euo pipefail

die() { printf '✗ %s\n' "$*" >&2; exit 1; }

if [ "$#" -gt 1 ]; then
  die "参数过多：只支持 --check"
fi

CHECK_ONLY=0
if [ "$#" -gt 0 ]; then
  case "$1" in
    --check) CHECK_ONLY=1 ;;
    -h|--help)
      printf '用法：bash packaging/build-ffmpeg.sh [--check]\n'
      exit 0
      ;;
    *) die "未知参数：$1（用法：bash packaging/build-ffmpeg.sh [--check]）" ;;
  esac
fi

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
command -v cygpath >/dev/null 2>&1 || die "需要 Git Bash（缺少 cygpath）；请在 Git Bash 中运行"
# Windows 混合式路径（D:/...）：MSYS 工具与原生 mingw 工具（ranlib 等）都接受；
# 纯 MSYS 路径（/d/...）会传进原生 ranlib/ar 导致 "No such file"
ROOT="$(cygpath -m "$ROOT")"
BUILD="${FFMPEG_BUILD_DIR:-$ROOT/packaging/build}"
DIST="$ROOT/packaging/dist"
PREFIX="$BUILD/prefix"

TOOLS=(gcc nasm mingw32-make)
if [ -n "${MINGW_BIN:-}" ]; then
  [ -d "$MINGW_BIN" ] || die "MINGW_BIN 指向的目录不存在：$MINGW_BIN
请把它设为 MinGW 的 bin 目录，例如 MINGW_BIN=<mingw64>/bin。"
  mingw_missing=()
  for tool in "${TOOLS[@]}"; do
    if [ ! -x "$MINGW_BIN/$tool" ] && [ ! -x "$MINGW_BIN/$tool.exe" ]; then
      mingw_missing+=("$tool")
    fi
  done
  if [ "${#mingw_missing[@]}" -gt 0 ]; then
    printf '✗ MINGW_BIN 中缺少工具：%s\n' "${mingw_missing[*]}" >&2
    die "请检查 MINGW_BIN 是否指向 MinGW 的 bin 目录：$MINGW_BIN"
  fi
  export PATH="$MINGW_BIN:$PATH"
fi

missing=()
for tool in "${TOOLS[@]}"; do
  command -v "$tool" >/dev/null 2>&1 || missing+=("$tool")
done
if [ "${#missing[@]}" -gt 0 ]; then
  printf '✗ 缺少构建工具：%s\n' "${missing[*]}" >&2
  cat >&2 <<'EOF'
需要 Git Bash + winlibs MinGW（gcc / nasm / mingw32-make）。
安装后把 MinGW 的 bin 目录加入 PATH，或用 MINGW_BIN 指定：
  MINGW_BIN=<mingw64>/bin bash packaging/build-ffmpeg.sh
只做工具链自检：
  bash packaging/build-ffmpeg.sh --check
EOF
  exit 1
fi

if [ "$CHECK_ONLY" -eq 1 ]; then
  echo "✓ ffmpeg 构建工具链就绪"
  for tool in "${TOOLS[@]}"; do
    printf '  %-15s %s\n' "$tool:" "$(command -v "$tool")"
  done
  printf '  %-15s %s\n' "MINGW_BIN:" "${MINGW_BIN:-<unset>}"
  exit 0
fi

MAKE="mingw32-make"
JOBS="${JOBS:-$(nproc 2>/dev/null || echo 8)}"

ZLIB_VER="1.3.1"
X264_VER="stable"
FFMPEG_VER="8.1.1"

mkdir -p "$BUILD" "$DIST" "$PREFIX"
cd "$BUILD"

fetch() { # fetch <url> <tarball> <sha256> —— 已存在则跳过（断点重跑友好），哈希不符即失败
  # x264 没有版本化 release（stable 是滚动快照），复现性靠这份哈希钉住；
  # 上游快照轮换时这里会红，届时人工升级版本+哈希，而不是静默换源码。
  local url="$1" out="$2" sha="$3"
  if [ ! -f "$out" ]; then
    echo ">> 下载 $url"
    curl -fL --retry 3 -o "$out" "$url"
  fi
  echo "$sha *$out" | sha256sum -c - >/dev/null \
    || { echo "✗ $out 的 SHA-256 与钉死值不符" >&2; exit 1; }
}

# ---------------------------------------------------------------------------
# 1. zlib（静态，供 PNG 解码）
# ---------------------------------------------------------------------------
if [ ! -f "$PREFIX/lib/libz.a" ]; then
  fetch "https://zlib.net/fossils/zlib-${ZLIB_VER}.tar.gz" "zlib-${ZLIB_VER}.tar.gz" \
    "9a93b2b7dfdac77ceba5a558a580e74667dd6fede4585b91eefb60f03b72df23"
  rm -rf "zlib-${ZLIB_VER}" && tar xzf "zlib-${ZLIB_VER}.tar.gz"
  cd "zlib-${ZLIB_VER}"
  CHOST=x86_64-w64-mingw32 ./configure --prefix="$PREFIX" --static
  $MAKE -j"$JOBS" && $MAKE install
  cd "$BUILD"
fi

# ---------------------------------------------------------------------------
# 2. x264（静态库，无 CLI）
# ---------------------------------------------------------------------------
if [ ! -f "$PREFIX/lib/libx264.a" ]; then
  fetch "https://code.videolan.org/videolan/x264/-/archive/${X264_VER}/x264-${X264_VER}.tar.bz2" "x264-${X264_VER}.tar.bz2" \
    "740126cb48549ca4e2f09b4ae13beed592d5b54744044e38f28662fd96d0ba56"
  rm -rf "x264-${X264_VER}" && tar xjf "x264-${X264_VER}.tar.bz2"
  cd "x264-${X264_VER}"
  ./configure --prefix="$PREFIX" --host=x86_64-w64-mingw32 \
    --enable-static --disable-cli --disable-opencl \
    --extra-cflags="-O2" --extra-ldflags="-static"
  $MAKE -j"$JOBS" && $MAKE install
  cd "$BUILD"
fi

# ---------------------------------------------------------------------------
# 3. ffmpeg 最小构建
# ---------------------------------------------------------------------------
# 本机无 pkg-config（Git Bash/winlibs 都不带），而 ffmpeg configure 对 libx264 走
# pkg-config 探测：生成一个只读 prefix 下 .pc 文件的最小垫片。
# 注意：垫片目录经 CCAST_PC_DIR 拿目录（不能解析 PKG_CONFIG_PATH——D:/ 盘符的冒号
# 会被当成路径分隔符）；PATH 条目必须用 MSYS 形式才能被命令查找命中。
CCAST_PC_DIR="$PREFIX/lib/pkgconfig"
export CCAST_PC_DIR
if [ ! -f "$BUILD/shim/pkg-config" ]; then
  mkdir -p "$BUILD/shim"
  cat > "$BUILD/shim/pkg-config" <<'SHIM'
#!/usr/bin/env bash
# minimal pkg-config (prefix-local .pc files only) — enough for ffmpeg configure
PC_DIR="${CCAST_PC_DIR:?CCAST_PC_DIR must be exported}"
mod=""
for a in "$@"; do case "$a" in -*) ;; *) mod="$a" ;; esac; done
pc="$PC_DIR/$mod.pc"
# configure 的开场探测是裸 `pkg-config --version`（无模块名）：必须成功
if [ -z "$mod" ]; then
  [ "$want" = "ver" ] && echo "0.29.1"
  exit 0
fi
[ -f "$pc" ] || exit 1
prefix="$(sed -n 's/^prefix=//p' "$pc" | head -1)"
want=""
for a in "$@"; do case "$a" in --modversion|--version) want=ver ;; --cflags*) want=cflags ;; --libs*) want=libs ;; --exists) want=exists ;; esac; done
case "$want" in
  exists) exit 0 ;;
  ver) sed -n 's/^Version:[[:space:]]*//p' "$pc" ;;
  # 带空格的 -I/-L 形式：与 gcc 等价，但对 configure 的 shell 分词无歧义
  # （黏连的 -ID:/... 曾在探测命令里留下裸 -I，吞掉后续 -c）
  cflags) sed -n 's/^Cflags:[[:space:]]*//p' "$pc" | sed "s|\${prefix}|$prefix|g;s|\${exec_prefix}|$prefix|g;s|\${includedir}|$prefix/include|g" | sed 's/-I\(\S\)/-I \1/g' ;;
  libs) sed -n 's/^Libs:[[:space:]]*//p' "$pc" | sed "s|\${prefix}|$prefix|g;s|\${exec_prefix}|$prefix|g;s|\${libdir}|$prefix/lib|g" | sed 's/-L\(\S\)/-L \1/g' ;;
  *) exit 0 ;;
esac
SHIM
  chmod +x "$BUILD/shim/pkg-config"
fi
export PATH="$(cygpath -u "$BUILD")/shim:$PATH"
command -v pkg-config >/dev/null || echo "内部错误：pkg-config 垫片未上 PATH" >&2

if [ ! -f "$DIST/ffmpeg.exe" ]; then
  fetch "https://ffmpeg.org/releases/ffmpeg-${FFMPEG_VER}.tar.xz" "ffmpeg-${FFMPEG_VER}.tar.xz" \
    "b6863adde98898f42602017462871b5f6333e65aec803fdd7a6308639c52edf3"
  rm -rf "ffmpeg-${FFMPEG_VER}" && tar xf "ffmpeg-${FFMPEG_VER}.tar.xz"
  cd "ffmpeg-${FFMPEG_VER}"
  PKG_CONFIG_PATH="$PREFIX/lib/pkgconfig" ./configure \
    --prefix="$PREFIX" --target-os=mingw32 --arch=x86_64 --cc=gcc \
    --enable-gpl --enable-libx264 --enable-zlib \
    --enable-static --disable-shared \
    --extra-cflags="-O2 -I $PREFIX/include" \
    --extra-ldflags="-L $PREFIX/lib -static" \
    --extra-libs="-lz" \
    --disable-everything --disable-autodetect --disable-network \
    --disable-ffprobe --disable-ffplay --disable-doc \
    --enable-decoder=h264,prores,aac,mp3,pcm_s16le,pcm_u8,pcm_f32le,pcm_f64le,png,wrapped_avframe \
    --enable-encoder=libx264,aac,pcm_s16le,png,prores_ks,wrapped_avframe \
    --enable-demuxer=mov,mp4,m4a,mp3,wav,concat,image2 \
    --enable-indev=lavfi \
    --enable-muxer=mp4,mov,ipod,wav,image2,null \
    --enable-filter=aresample,scale,pad,setsar,fps,fade,crop,format,geq,overlay,concat,afade,anull,anullsrc,aevalsrc,color,sine \
    --enable-protocol=file,pipe
  $MAKE -j"$JOBS"
  cp ffmpeg.exe "$DIST/ffmpeg.exe"
fi

echo ">> 完成：$DIST/ffmpeg.exe（$(du -h "$DIST/ffmpeg.exe" | cut -f1)）"
echo ">> 验收：bash packaging/check-ffmpeg-capabilities.sh \"$DIST/ffmpeg.exe\""
