#!/usr/bin/env bash
# 票 16：脚本化端到端验收 + 三类故障注入（Windows x64 / Git Bash）。
#
# ① 端到端：干净临时数据目录，fake 引擎，PAGES×PAGE_SECONDS ≈ 10 分钟成片；
#    记录各阶段耗时、成片体积、容器时长与翻页误差。
# ② 注入 TTS 401：本地 mock 端点回 401（MiMo Key 失效）→ 降级链一路到底，
#    主流程（timeline/render）不中断。
# ③ 注入 ffmpeg 渲染中途失败：垫片写半截输出后退出 1 → stage=failed；
#    同一路径重试该段成功，前序阶段产物哈希不变。
# ④ 注入合成中途 kill -9：stage=tts_running、已完成页落盘；重进只补未完成页。
#
# 用法：
#   bash scripts/acceptance.sh                                  # 完整验收（默认 ~10 分钟成片）
#   PAGES=6 PAGE_SECONDS=2 bash scripts/acceptance.sh           # 快速冒烟
#   KEEP=1 bash scripts/acceptance.sh                           # 保留临时现场
# 依赖：cargo/rustc（构建 release CLI + 注入垫片）、ffmpeg（素材/时长/翻页检测）、node（JSON 断言与 mock）
set -euo pipefail

HERE="$(cd "$(dirname "$0")" && pwd)"
ROOT="$(cd "$HERE/.." && pwd)"
JSON_HELPER="$HERE/acceptance/json.js"
PAGES="${PAGES:-50}"
PAGE_SECONDS="${PAGE_SECONDS:-12}"
KEEP="${KEEP:-0}"
MAX_TURN_MS="${MAX_TURN_MS:-120}"
ONLY="${ONLY:-all}" # all | e2e | 401 | ffmpeg | kill9（调试单个注入用）

FF="$(command -v "${FFMPEG:-ffmpeg}")"
FF_WIN="$(cygpath -w "$FF")"
CLI="$ROOT/target/release/cc-cli.exe"

WORK="$(mktemp -d)"
DATA="$WORK/data"
DATA_WIN="$(cygpath -w "$DATA")"
mkdir -p "$DATA"
MOCK_PID=""

cleanup() {
  if [ -n "$MOCK_PID" ]; then
    kill -9 "$MOCK_PID" 2>/dev/null || true
  fi
  if [ "$KEEP" = "1" ]; then
    echo ">> 保留现场：$WORK"
  else
    rm -rf "$WORK"
  fi
}
trap cleanup EXIT

say() { printf '\n===== %s =====\n' "$*"; }
fail() { printf '✗ %s\n' "$*" >&2; exit 1; }
now_ms() { date +%s%3N; }
elapsed() { echo $(( $2 - $1 )); }
cli() { "$CLI" --data-dir "$DATA_WIN" "$@"; }
jget() { node "$JSON_HELPER" get "$1" "$2"; }
jeq() { node "$JSON_HELPER" assert "$1" "$2" "$3" || fail "$1: $2 != $3"; }
jincludes() { node "$JSON_HELPER" assert-includes "$1" "$2" "$3" || fail "$1: $2 不含 $3"; }
jmatch() { node "$JSON_HELPER" assert-match "$1" "$2" "$3" || fail "$1: $2 不匹配 /$3/"; }

make_deck() { # <dir> <pages>
  local dir="$1" pages="$2" i color
  mkdir -p "$dir"
  for i in $(seq 1 "$pages"); do
    # 深浅交替：scene 检测基于亮度差（验证报告 §2.3 的教训）
    if (( i % 2 == 1 )); then color=0x202020; else color=0xE0E0E0; fi
    "$FF" -y -v error -f lavfi -i "color=c=$color:s=1920x1080:d=1" \
      -frames:v 1 "$dir/slide_$(printf '%03d' "$i").png"
  done
}

write_script() { # <project_id> <pages> <seconds-per-page>
  local id="$1" pages="$2" secs="$3" chars repeats p r line
  chars=$(awk -v s="$secs" 'BEGIN{printf "%d", (s-0.2)*100}')
  repeats=$(( (chars + 5) / 6 )) # 「课堂演示内容」6 字
  local file="$DATA/projects/$id/script.md"
  : > "$file"
  for p in $(seq 1 "$pages"); do
    line="第${p}页："
    for ((r = 0; r < repeats; r++)); do line+="课堂演示内容"; done
    printf '%s\n' "$line" >> "$file"
    if [ "$p" -lt "$pages" ]; then printf -- '---\n' >> "$file"; fi
  done
}

make_person() { # <project_id>：静态色块人像，走 render 的白卡兜底分支
  local id="$1"
  mkdir -p "$DATA/projects/$id/source"
  "$FF" -y -v error -f lavfi -i "color=c=orange:s=480x360:d=2" \
    -c:v libx264 -preset ultrafast -pix_fmt yuv420p \
    "$DATA/projects/$id/source/person.mp4"
}

make_ref() { # <project_id>：参考音频（401 注入需要 MiMo 请求带 reference）
  local id="$1"
  "$FF" -y -v error -f lavfi -i "sine=frequency=220:duration=1" -ar 24000 -ac 1 "$WORK/ref.wav"
  cli voice_configure --project "$id" --audio "$(cygpath -w "$WORK/ref.wav")" --json > "$WORK/configure.json"
}

new_project() { # <name> <pages> <seconds-per-page> → stdout: project_id
  local name="$1" pages="$2" secs="$3" deck id
  cli project create --name "$name" --json > "$WORK/create.json"
  id="$(jget "$WORK/create.json" project_id)"
  [ -n "$id" ] || fail "建项目未返回 project_id"
  deck="$WORK/deck-$id"
  make_deck "$deck" "$pages"
  cli slide_render --project "$id" --source "$(cygpath -w "$deck")" --json > "$WORK/slide-$id.json"
  write_script "$id" "$pages" "$secs"
  make_person "$id"
  make_ref "$id"
  printf '%s' "$id"
}

container_sec() { # <mp4>
  # ffmpeg 纯探测（-i 无输出文件）本身退出 1；pipefail 下必须显式忽略，
  # 否则赋值语句被 set -e 误判。也不用 head 提前关管道（上游 sed 会吃 SIGPIPE）。
  local probe
  probe=$("$FF" -hide_banner -i "$1" 2>&1 || true)
  printf '%s\n' "$probe" \
    | sed -n 's/.*Duration: \([0-9][0-9]:[0-9][0-9]:[0-9.]*\).*/\1/p' \
    | awk -F '[:.]' 'NR==1{printf "%.3f", $1 * 3600 + $2 * 60 + $3 + $4 / 100}'
}

sha256sums() { # <project_dir> <out_file>：记录当前已存在产物的内容哈希（kill 注入时还没有整轨）
  (
    cd "$1"
    for f in work/audio/slide_*.wav work/audio/voice_full.wav work/concat.txt; do
      if [ -f "$f" ]; then sha256sum "$f"; fi
    done
  ) > "$2"
}

require_build() {
  say "构建 release cc-cli"
  cargo build --release -p cc-cli --manifest-path "$ROOT/Cargo.toml"
  [ -x "$CLI" ] || fail "缺少 $CLI"
}

# ---------------------------------------------------------------------------
# ① 端到端（fake 引擎，~10 分钟成片）
# ---------------------------------------------------------------------------
run_e2e() {
  say "① 端到端：fake 引擎，${PAGES} 页 × ${PAGE_SECONDS}s（目标 ≈10 分钟成片）"
  local t_all t_create t_fixtures_done t_slides t_tts t_timeline t_render id deck final
  export CC_FFMPEG="$FF_WIN"

  t_all=$(now_ms)
  cli project create --name "验收-十分钟成片" --json > "$WORK/e2e-create.json"
  id="$(jget "$WORK/e2e-create.json" project_id)"
  t_create=$(now_ms)

  # 测试素材（课件 PNG / 逐字稿 / 人像占位）不是产品耗时，计时窗口排除
  deck="$WORK/deck-$id"
  make_deck "$deck" "$PAGES"
  write_script "$id" "$PAGES" "$PAGE_SECONDS"
  make_person "$id"
  t_fixtures_done=$(now_ms)

  cli slide_render --project "$id" --source "$(cygpath -w "$deck")" --json > "$WORK/e2e-slides.json"
  t_slides=$(now_ms)

  cli voice_synthesize --project "$id" --engine fake --json > "$WORK/e2e-tts.json"
  t_tts=$(now_ms)

  cli timeline --project "$id" --json > "$WORK/e2e-timeline.json"
  t_timeline=$(now_ms)

  cli render --project "$id" --json > "$WORK/e2e-render.json"
  t_render=$(now_ms)

  jeq "$WORK/e2e-tts.json" ok true
  jeq "$WORK/e2e-tts.json" stage '"tts_done"'
  jeq "$WORK/e2e-tts.json" generated.length "$PAGES"
  jeq "$WORK/e2e-render.json" ok true
  jeq "$WORK/e2e-render.json" stage '"done"'

  final="$DATA/projects/$id/output/final.mp4"
  [ -f "$final" ] || fail "未出片：$final"
  local total_sec duration size partial_ms
  total_sec="$(jget "$WORK/e2e-timeline.json" total_sec)"
  duration="$(container_sec "$final")"
  size=$(stat -c %s "$final")
  awk -v a="$total_sec" -v b="$duration" 'BEGIN{d=a-b; if (d<0) d=-d; exit !(d<=0.2)}' \
    || fail "容器时长 $duration 与时间轴 $total_sec 误差超过 200ms"

  # 翻页误差：scene 检测 vs timeline 期望
  "$FF" -hide_banner -i "$final" -filter:v "select='gt(scene,0.02)',showinfo" -f null - \
    > "$WORK/scene.log" 2>&1
  grep -o "pts_time:[0-9.]*" "$WORK/scene.log" | cut -d: -f2 > "$WORK/detected.txt" || true
  [ -s "$WORK/detected.txt" ] || fail "scene 检测没有检出任何翻页点"
  local turns
  turns="$(node "$HERE/acceptance/turns.js" "$WORK/e2e-timeline.json" "$WORK/detected.txt" "$MAX_TURN_MS")"
  echo "$turns"

  local create_ms slides_ms tts_ms timeline_ms render_ms wall_ms
  create_ms=$(elapsed "$t_all" "$t_create")
  slides_ms=$(elapsed "$t_fixtures_done" "$t_slides")
  tts_ms=$(elapsed "$t_slides" "$t_tts")
  timeline_ms=$(elapsed "$t_tts" "$t_timeline")
  render_ms=$(elapsed "$t_timeline" "$t_render")
  wall_ms=$(( $(elapsed "$t_all" "$t_render") - $(elapsed "$t_create" "$t_fixtures_done") ))
  printf '\nE2E 耗时（毫秒）：建项目 %s，课件解析 %s，TTS %s，时间轴 %s，渲染 %s，合计 %s\n' \
    "$create_ms" "$slides_ms" "$tts_ms" "$timeline_ms" "$render_ms" "$wall_ms"
  printf '成片：%s 秒，%s 字节（%.2f MiB），%s\n' "$duration" "$size" \
    "$(awk -v b="$size" 'BEGIN{printf "%.2f", b/1048576}')" "$turns"

  E2E_SUMMARY="project_id=$id|pages=$PAGES|total_sec=$total_sec|duration=$duration|size=$size|create_ms=$create_ms|slides_ms=$slides_ms|tts_ms=$tts_ms|timeline_ms=$timeline_ms|render_ms=$render_ms|wall_ms=$wall_ms|turns=$turns"
}

# ---------------------------------------------------------------------------
# ② TTS 401 → 降级链，主流程不中断
# ---------------------------------------------------------------------------
run_401() {
  say "② 注入 TTS 401：MiMo Key 失效 → 降级链（edge 不可达 → SAPI），主流程继续"
  local id
  export CC_FFMPEG="$FF_WIN"
  id="$(new_project "注入-401降级" 2 2)"

  node "$HERE/acceptance/mock401.js" "$WORK/mock-port" &
  MOCK_PID=$!
  local i
  for i in $(seq 1 100); do [ -s "$WORK/mock-port" ] && break; sleep 0.1; done
  [ -s "$WORK/mock-port" ] || fail "401 mock 未启动"
  local port
  port=$(cat "$WORK/mock-port")

  printf 'sk-acceptance-invalid-key' > "$WORK/mimo-key.txt"
  export CC_CREDENTIALS_FILE="$(cygpath -w "$WORK/mimo-key.txt")"
  export CC_MIMO_BASE_URL="http://127.0.0.1:$port"
  export CC_EDGE_ENDPOINT="ws://127.0.0.1:1/edge/v1" # 立即拒连：让降级链落到 SAPI
  # 克隆合规门：用 CLI 记录《声音授权确认》（与 UI 同一份文件）
  cli voice_consent --project "$id" --grant --json > "$WORK/401-consent.json"

  cli voice_synthesize --project "$id" --engine mimo --json > "$WORK/401-tts.json"
  unset CC_CREDENTIALS_FILE CC_MIMO_BASE_URL CC_EDGE_ENDPOINT

  jeq "$WORK/401-tts.json" ok true
  jeq "$WORK/401-tts.json" stage '"tts_done"'
  jeq "$WORK/401-tts.json" failed_pages.length 0
  jeq "$WORK/401-tts.json" generated.length 2
  jeq "$WORK/401-tts.json" "fallbacks.0.from" '"mimo"'
  jeq "$WORK/401-tts.json" "fallbacks.0.to" '"edge"'
  jeq "$WORK/401-tts.json" "fallbacks.1.to" '"sapi"'
  jmatch "$WORK/401-tts.json" fallbacks 'HTTP 401'
  echo "降级事件：$(jget "$WORK/401-tts.json" fallbacks)"

  cli timeline --project "$id" --json > "$WORK/401-timeline.json"
  cli render --project "$id" --json > "$WORK/401-render.json"
  jeq "$WORK/401-render.json" ok true
  [ -f "$DATA/projects/$id/output/final.mp4" ] || fail "401 注入后主流程未出片"
  echo "主流程继续：timeline + render 出片成功"
}

# ---------------------------------------------------------------------------
# ③ ffmpeg 渲染中途失败 → failed + 单段重试
# ---------------------------------------------------------------------------
run_ffmpeg_failure() {
  say "③ 注入 ffmpeg 渲染中途失败：写半截输出后退出 1 → stage=failed，重试该段成功"
  local id shim final
  export CC_FFMPEG="$FF_WIN"
  id="$(new_project "注入-ffmpeg失败" 3 1)"
  cli voice_synthesize --project "$id" --engine fake --json > "$WORK/ff-tts.json"
  cli timeline --project "$id" --json > "$WORK/ff-timeline.json"
  sha256sums "$DATA/projects/$id" "$WORK/ff-before.sha"

  shim="$WORK/ffmpeg-shim.exe"
  rustc -O "$HERE/acceptance/ffmpeg-shim.rs" -o "$(cygpath -m "$shim")"
  export CC_FFMPEG="$(cygpath -w "$shim")"
  export CC_SHIM_REAL_FFMPEG="$FF_WIN"
  export CC_SHIM_FAIL_RENDER=1

  if cli render --project "$id" --json > "$WORK/ff-render-fail.json"; then
    fail "注入后 render 不应成功"
  fi
  jeq "$WORK/ff-render-fail.json" ok false
  jeq "$WORK/ff-render-fail.json" error.code '"ffmpeg_failed"'

  cli project status --project "$id" --json > "$WORK/ff-status.json"
  jeq "$WORK/ff-status.json" stage '"failed"'
  jeq "$WORK/ff-status.json" failed.stage '"rendering"'

  final="$DATA/projects/$id/output/final.mp4"
  [ -f "$final" ] || fail "中途失败应留下半截产物（用于验证不静默过期）"
  local partial
  partial=$(stat -c %s "$final")
  if "$FF" -v error -i "$final" -f null - > /dev/null 2>&1; then
    fail "半截产物居然可解码，注入无效"
  fi
  echo "失败现场：stage=failed(rendering)，半截 final.mp4=${partial}B（不可解码）"

  unset CC_SHIM_FAIL_RENDER
  cli render --project "$id" --json > "$WORK/ff-render-retry.json"
  jeq "$WORK/ff-render-retry.json" ok true
  jeq "$WORK/ff-render-retry.json" stage '"done"'
  cli project status --project "$id" --json > "$WORK/ff-status-retry.json"
  jeq "$WORK/ff-status-retry.json" failed null

  (cd "$DATA/projects/$id" && sha256sum -c "$WORK/ff-before.sha" > /dev/null) \
    || fail "重试触发了前序阶段重做（TTS/时间轴产物哈希变化）"
  "$FF" -v error -i "$final" -f null - > /dev/null 2>&1 || fail "重试后的成片不可解码"
  local good
  good=$(stat -c %s "$final")
  [ "$good" -gt "$partial" ] || fail "重试后成片未覆盖半截产物"
  echo "单段重试：stage=done，成片 ${partial}B → ${good}B 且可解码，前序 3 页产物哈希不变"

  export CC_FFMPEG="$FF_WIN"
  unset CC_SHIM_REAL_FFMPEG
}

# ---------------------------------------------------------------------------
# ④ 合成中途 kill -9 → 重进续跑不重做
# ---------------------------------------------------------------------------
run_kill9() {
  say "④ 注入合成中途 kill -9：已完成页落盘，重进只补未完成页"
  local id pid audio done_before i total=10
  export CC_FFMPEG="$FF_WIN"
  id="$(new_project "注入-kill9续跑" "$total" 1)"
  audio="$DATA/projects/$id/work/audio"

  export CC_FAKE_TTS_DELAY_MS=400
  # 直接起 exe，不走 cli() 函数：函数进后台是子 shell，kill 只杀 shell、孤儿 CLI
  # 会继续写完并持锁（本脚本自测时踩过）
  "$CLI" --data-dir "$DATA_WIN" voice_synthesize --project "$id" --engine fake --json \
    > "$WORK/kill-out.json" &
  pid=$!
  for i in $(seq 1 100); do
    done_before=$(find "$audio" -maxdepth 1 -name 'slide_*.wav' 2>/dev/null | wc -l)
    [ "$done_before" -ge 1 ] && break
    sleep 0.1
  done
  sleep 0.5
  kill -9 "$pid" 2>/dev/null || true
  wait "$pid" 2>/dev/null || true
  unset CC_FAKE_TTS_DELAY_MS

  done_before=$(find "$audio" -maxdepth 1 -name 'slide_*.wav' 2>/dev/null | wc -l)
  [ "$done_before" -ge 1 ] || fail "kill -9 前没有任何页完成，注入无效"
  [ "$done_before" -lt "$total" ] || fail "kill -9 前所有页已完成，未能落在合成中途"
  local crashed_tmp
  crashed_tmp=$(find "$DATA/projects/$id" -name '*.wav.tmp' | wc -l)
  echo "kill -9：已完成 ${done_before}/${total} 页（崩溃时 .tmp 残留 ${crashed_tmp} 个，属半写产物、无成功记录）"

  cli project status --project "$id" --json > "$WORK/kill-status.json"
  jeq "$WORK/kill-status.json" stage '"tts_running"'

  sha256sums "$DATA/projects/$id" "$WORK/kill-before.sha"

  cli voice_synthesize --project "$id" --engine fake --json > "$WORK/kill-resume.json"
  jeq "$WORK/kill-resume.json" ok true
  jeq "$WORK/kill-resume.json" stage '"tts_done"'
  jeq "$WORK/kill-resume.json" failed_pages.length 0
  jeq "$WORK/kill-resume.json" skipped.length "$done_before"
  jeq "$WORK/kill-resume.json" generated.length "$(( total - done_before ))"
  for i in $(seq 1 "$done_before"); do
    jincludes "$WORK/kill-resume.json" skipped "$i"
  done
  (cd "$DATA/projects/$id" && sha256sum -c "$WORK/kill-before.sha" > /dev/null) \
    || fail "续跑重做了已完成页（产物哈希变化）"

  if find "$DATA/projects/$id" -name '*.wav.tmp' | grep -q .; then
    fail "续跑结束后仍残留 .wav.tmp"
  fi
  local wavs
  wavs=$(find "$audio" -maxdepth 1 -name 'slide_*.wav' | wc -l)
  [ "$wavs" -eq "$total" ] || fail "续跑后 wav 数 $wavs != $total"
  cli timeline --project "$id" --json > "$WORK/kill-timeline.json"
  jeq "$WORK/kill-timeline.json" ok true
  echo "续跑：只补第 $((done_before + 1))–${total} 页，已完成页产物哈希不变，无 .tmp 残留"
}

require_build
case "$ONLY" in
  all)
    run_e2e
    run_401
    run_ffmpeg_failure
    run_kill9
    ;;
  e2e) run_e2e ;;
  401) run_401 ;;
  ffmpeg) run_ffmpeg_failure ;;
  kill9) run_kill9 ;;
  *) fail "未知 ONLY=$ONLY（all|e2e|401|ffmpeg|kill9）" ;;
esac

say "✓ 票 16 验收全部通过"
if [ -n "${E2E_SUMMARY:-}" ]; then echo "E2E_SUMMARY $E2E_SUMMARY"; fi
