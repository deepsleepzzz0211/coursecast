#!/usr/bin/env bash
# Shared helpers for the spike scripts.

die() { printf '✗ %s\n' "$*" >&2; exit 1; }

need_cmd() { command -v "$1" >/dev/null 2>&1 || die "缺少命令：$1"; }

# resolve_out_dir <script_dir> <raw_out>
# Sets OUT_ABS. Rejects empty/root/script-dir, .. traversal and Windows drive paths.
resolve_out_dir() {
  local script_dir="$1" raw="$2" abs
  raw="${raw%/}"
  [ -n "$raw" ] || die "输出目录不能为空"
  case "$raw" in
    [A-Za-z]:*) die "输出目录请用 Git Bash 路径（例如 ./work），不要使用 Windows 盘符" ;;
  esac
  case "/$raw/" in
    *"/../"*) die "输出目录不能包含 .." ;;
  esac
  case "$raw" in
    /*) abs="$raw" ;;
    *) abs="$script_dir/$raw" ;;
  esac
  case "$abs" in
    "$script_dir"|"$script_dir/"|"$script_dir/."|"/") die "输出目录不能是脚本目录或根目录" ;;
  esac
  OUT_ABS="$abs"
}

# begin_staging <out_abs>
# Creates OUT_ABS.tmp.<pid>, registers cleanup, and sets TMP.
begin_staging() {
  TMP="${1}.tmp.$$"
  rm -rf "$TMP"
  mkdir -p "$TMP"
  trap 'rm -rf "$TMP"' EXIT
}

# commit_staging <script_dir> <out_abs>
commit_staging() {
  cd "$1"
  rm -rf "$2"
  mv "$TMP" "$2"
  trap - EXIT
}
