#!/usr/bin/env bash
# 采集 macOS 数据库客户端在空闲、查询、滚动和冷却阶段的内存。
#
# 脚本只观察指定进程树，不启动应用、不模拟鼠标键盘。每个阶段的边界由用户
# 按终端提示确认；footprint -t 会把 Electron / JVM 等子进程一并计入。
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
OUT_DIR="$ROOT/bench-results"
INTERVAL="0.5"
IDLE_SECONDS=60
SCROLL_SECONDS=20
COOLDOWN_SECONDS=30
QUERY="SELECT * FROM public.bench_orders ORDER BY id LIMIT 10000;"
APP_NAME=""
PATTERN=""
PID=""

usage() {
  cat <<'EOF'
用法：
  scripts/bench-memory-macos.sh <fluxdb|dbx|dbeaver> [选项]

选项：
  --pid <pid>              直接指定主进程 PID
  --pattern <pgrep 模式>   自定义进程匹配模式
  --out <目录>             输出目录，默认 bench-results/
  --interval <秒>          采样间隔，默认 0.5
  --idle-seconds <秒>      空闲采样时长，默认 60
  --scroll-seconds <秒>    滚动采样时长，默认 20
  --cooldown-seconds <秒>  冷却采样时长，默认 30
  --query <SQL>            查询阶段提示中展示的 SQL
  -h, --help               显示帮助

使用流程：
  1. 先打开一个客户端并连接 bench 数据库。
  2. 运行脚本，按提示保持空闲、执行查询、连续滚动和关闭结果页。
  3. 脚本结束后会输出按阶段汇总的内存摘要和 CSV 路径。
EOF
}

die() {
  printf '错误：%s\n' "$*" >&2
  exit 1
}

need_value() {
  [ "$#" -ge 2 ] && [ -n "$2" ] || die "$1 缺少参数值"
}

while [ "$#" -gt 0 ]; do
  case "$1" in
    -h|--help)
      usage
      exit 0
      ;;
    --pid)
      need_value "$@"
      PID="$2"
      shift 2
      ;;
    --pattern)
      need_value "$@"
      PATTERN="$2"
      shift 2
      ;;
    --out)
      need_value "$@"
      OUT_DIR="$2"
      shift 2
      ;;
    --interval)
      need_value "$@"
      INTERVAL="$2"
      shift 2
      ;;
    --idle-seconds)
      need_value "$@"
      IDLE_SECONDS="$2"
      shift 2
      ;;
    --scroll-seconds)
      need_value "$@"
      SCROLL_SECONDS="$2"
      shift 2
      ;;
    --cooldown-seconds)
      need_value "$@"
      COOLDOWN_SECONDS="$2"
      shift 2
      ;;
    --query)
      need_value "$@"
      QUERY="$2"
      shift 2
      ;;
    --)
      shift
      break
      ;;
    -*)
      die "未知选项：$1"
      ;;
    *)
      [ -z "$APP_NAME" ] || die "只能指定一个应用"
      APP_NAME="$1"
      shift
      ;;
  esac
done

if [ -z "$APP_NAME" ] && [ -z "$PID" ] && [ -z "$PATTERN" ]; then
  usage
  exit 1
fi

case "$APP_NAME" in
  fluxdb|FluxDB|FLUXDB)
    APP_NAME="FluxDB"
    [ -n "$PATTERN" ] || PATTERN="/Applications/FluxDB.app/Contents/MacOS/fluxdb-desktop"
    ;;
  dbx|DBX)
    APP_NAME="DBX"
    [ -n "$PATTERN" ] || PATTERN="/Applications/DBX.app/Contents/MacOS/dbx"
    ;;
  dbeaver|DBeaver|DBEAVER)
    APP_NAME="DBeaver"
    [ -n "$PATTERN" ] || PATTERN="/Applications/DBeaver.app/Contents/MacOS/dbeaver"
    ;;
  "")
    APP_NAME="custom"
    ;;
  *)
    die "未知应用：$APP_NAME（可用：fluxdb、dbx、dbeaver）"
    ;;
esac

command -v footprint >/dev/null 2>&1 || die "找不到 footprint；该脚本仅支持 macOS"
command -v pgrep >/dev/null 2>&1 || die "找不到 pgrep"

if [ -z "$PID" ]; then
  [ -n "$PATTERN" ] || die "未指定进程匹配模式"
  PID="$(pgrep -f "$PATTERN" 2>/dev/null | head -1 || true)"
  [ -n "$PID" ] || die "找不到 $APP_NAME 进程，请先启动应用，或用 --pid 指定"
fi

case "$PID" in
  *[!0-9]*|"")
    die "PID 必须是数字：$PID"
    ;;
esac

kill -0 "$PID" 2>/dev/null || die "进程 $PID 不存在或没有访问权限"

mkdir -p "$OUT_DIR"
STAMP="$(date +%Y%m%d-%H%M%S)"
SAFE_NAME="$(printf '%s' "$APP_NAME" | tr '[:upper:]' '[:lower:]' | tr -cs 'a-z0-9' '-')"
CSV="$OUT_DIR/${SAFE_NAME}-${STAMP}.csv"
printf 'timestamp,epoch_seconds,phase,app,pid,footprint_bytes,footprint_mib,processes\n' >"$CSV"

SAMPLER_PID=""
STOP_FILE=""

cleanup() {
  if [ -n "$STOP_FILE" ]; then
    touch "$STOP_FILE" 2>/dev/null || true
  fi
  if [ -n "$SAMPLER_PID" ]; then
    wait "$SAMPLER_PID" 2>/dev/null || true
  fi
}
trap cleanup EXIT INT TERM

read_footprint() {
  local raw
  raw="$(footprint -t -p "$PID" -f bytes 2>/dev/null)" || return 1

  local bytes
  bytes="$(printf '%s\n' "$raw" | sed -n 's/.*Summary Footprint: \([0-9][0-9]*\) B.*/\1/p' | tail -1)"
  if [ -z "$bytes" ]; then
    bytes="$(printf '%s\n' "$raw" | sed -n 's/.*Footprint: \([0-9][0-9]*\) B.*/\1/p' | head -1)"
  fi
  [ -n "$bytes" ] || return 1

  local processes
  processes="$(printf '%s\n' "$raw" | grep -E '\[[0-9]+\]: 64-bit' | wc -l | tr -d ' ' || true)"
  [ -n "$processes" ] || processes=1

  printf '%s %s\n' "$bytes" "$processes"
}

sample_once() {
  local phase="$1"
  local sample
  if ! sample="$(read_footprint)"; then
    printf '采样失败：进程 %s 可能已退出，或当前终端没有读取进程内存的权限。\n' "$PID" >&2
    return 1
  fi

  local bytes processes mib
  bytes="${sample%% *}"
  processes="${sample##* }"
  mib="$(awk -v b="$bytes" 'BEGIN { printf "%.2f", b / 1048576 }')"

  printf '%s,%s,%s,%s,%s,%s,%s,%s\n' \
    "$(date -u +%Y-%m-%dT%H:%M:%SZ)" \
    "$(date +%s)" \
    "$phase" \
    "$APP_NAME" \
    "$PID" \
    "$bytes" \
    "$mib" \
    "$processes" >>"$CSV"
}

sampler_loop() {
  local phase="$1"
  local stop_file="$2"
  while [ ! -f "$stop_file" ]; do
    sample_once "$phase" || return 1
    sleep "$INTERVAL"
  done
}

start_sampler() {
  local phase="$1"
  STOP_FILE="$(mktemp "${TMPDIR:-/tmp}/fluxdb-bench-stop.XXXXXX")"
  rm -f "$STOP_FILE"
  sampler_loop "$phase" "$STOP_FILE" &
  SAMPLER_PID="$!"
}

stop_sampler() {
  [ -n "$STOP_FILE" ] || return 0
  touch "$STOP_FILE"
  wait "$SAMPLER_PID" 2>/dev/null || true
  rm -f "$STOP_FILE"
  STOP_FILE=""
  SAMPLER_PID=""
}

wait_enter() {
  local prompt="$1"
  local reply=""
  printf '%s' "$prompt"
  IFS= read -r reply
}

run_timed_phase() {
  local phase="$1"
  local seconds="$2"
  local prompt="$3"
  wait_enter "$prompt"
  printf '采集中：%s，持续 %s 秒...\n' "$phase" "$seconds"
  start_sampler "$phase"
  sleep "$seconds"
  stop_sampler
  printf '阶段完成：%s\n\n' "$phase"
}

run_interactive_phase() {
  local phase="$1"
  local start_prompt="$2"
  local end_prompt="$3"
  wait_enter "$start_prompt"
  start_sampler "$phase"
  wait_enter "$end_prompt"
  stop_sampler
  printf '阶段完成：%s\n\n' "$phase"
}

printf '应用：%s\n进程：%s\n输出：%s\n' "$APP_NAME" "$PID" "$CSV"
printf '提示：测试期间不要打开其他数据库客户端；查询和滚动阶段请保持窗口大小、缩放和主题一致。\n\n'

run_timed_phase "idle" "$IDLE_SECONDS" \
  "1/4 保持应用空闲、不要操作。按回车开始 ${IDLE_SECONDS} 秒空闲采样："

run_interactive_phase "query" \
  "2/4 按回车开始查询阶段，然后切回应用执行：${QUERY}"$'\n' \
  "查询结果完全加载后回到终端，按回车结束查询阶段："

run_timed_phase "scroll" "$SCROLL_SECONDS" \
  "3/4 按回车开始 ${SCROLL_SECONDS} 秒滚动阶段，然后切回应用连续滚动结果："

run_timed_phase "cooldown" "$COOLDOWN_SECONDS" \
  "4/4 关闭查询结果页并保持应用空闲，按回车开始 ${COOLDOWN_SECONDS} 秒冷却采样："

summarize_phase() {
  local phase="$1"
  awk -F, -v phase="$phase" '
    NR > 1 && $3 == phase {
      n += 1
      sum += $6
      if (n == 1 || $6 < min) min = $6
      if (n == 1 || $6 > max) max = $6
      if ($8 > max_processes) max_processes = $8
    }
    END {
      if (n == 0) {
        printf "  %-9s 无有效样本\n", phase
        exit
      }
      printf "  %-9s 平均 %8.2f MiB  最小 %8.2f MiB  峰值 %8.2f MiB  %d 个样本  %d 个进程\n",
        phase, sum / n / 1048576, min / 1048576, max / 1048576, n, max_processes
    }
  ' "$CSV"
}

printf '采样完成，内存摘要：\n'
summarize_phase idle
summarize_phase query
summarize_phase scroll
summarize_phase cooldown
printf '\nCSV：%s\n' "$CSV"
printf '建议：每个客户端至少跑 3 次，取中位数；FPS 或掉帧率请另用统一录屏方法测量。\n'
