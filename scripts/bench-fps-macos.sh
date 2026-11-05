#!/usr/bin/env bash
# 分析 macOS 窗口录屏的有效帧率与卡顿。
#
# 录制必须只包含目标应用窗口，并在整个片段中持续滚动同一份结果。脚本通过
# mpdecimate 去掉录屏容器中的重复帧，以窗口最终呈现的帧作为比较口径，不读取
# 应用内部 FPS，也不模拟鼠标或键盘。
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
OUT_DIR="$ROOT/bench-results"
TARGET_FPS=60
LABEL=""
VIDEO=""
START_SECONDS=0
END_SECONDS=""

usage() {
  cat <<'EOF'
用法：
  scripts/bench-fps-macos.sh <名称> <录屏文件> [选项]

选项：
  --target-fps <fps>  录屏基准帧率，默认 60
  --start <秒>        分析起点，默认 0
  --end <秒>          分析终点，默认视频结尾
  --out <目录>        输出目录，默认 bench-results/
  -h, --help          显示帮助

录制建议：
  1. 使用 macOS 的“截屏/录屏”工具栏，只录制目标应用窗口。
  2. 统一窗口大小、缩放、主题和结果集。
  3. 录制开始后持续滚动结果，不要中途停顿或切换窗口。

输出：
  bench-results/fps-results.csv
  包含有效帧率、平均/P95/最大帧间隔、估算掉帧率。
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
    --target-fps)
      need_value "$@"
      TARGET_FPS="$2"
      shift 2
      ;;
    --start)
      need_value "$@"
      START_SECONDS="$2"
      shift 2
      ;;
    --end)
      need_value "$@"
      END_SECONDS="$2"
      shift 2
      ;;
    --out)
      need_value "$@"
      OUT_DIR="$2"
      shift 2
      ;;
    -*)
      die "未知选项：$1"
      ;;
    *)
      if [ -z "$LABEL" ]; then
        LABEL="$1"
      elif [ -z "$VIDEO" ]; then
        VIDEO="$1"
      else
        die "只能分析一个录屏文件"
      fi
      shift
      ;;
  esac
done

[ -n "$LABEL" ] && [ -n "$VIDEO" ] || {
  usage
  exit 1
}
[ -f "$VIDEO" ] || die "录屏文件不存在：$VIDEO"

command -v ffmpeg >/dev/null 2>&1 || die "找不到 ffmpeg"
command -v ffprobe >/dev/null 2>&1 || die "找不到 ffprobe"

case "$TARGET_FPS" in
  *[!0-9]*|"")
    die "target-fps 必须是正整数"
    ;;
esac
[ "$TARGET_FPS" -gt 0 ] || die "target-fps 必须大于 0"

DURATION="$(ffprobe -v error -show_entries format=duration -of default=nw=1:nk=1 "$VIDEO" 2>/dev/null | head -1)"
[ -n "$DURATION" ] || die "无法读取视频时长：$VIDEO"

case "$DURATION" in
  *[!0-9.]*|"")
    die "视频时长格式异常：$DURATION"
    ;;
esac

[ -n "$END_SECONDS" ] || END_SECONDS="$DURATION"
awk -v start="$START_SECONDS" -v end="$END_SECONDS" -v duration="$DURATION" 'BEGIN {
  if (start < 0 || end <= start || end > duration + 0.05) exit 1
}' || die "分析区间无效：start=$START_SECONDS end=$END_SECONDS duration=$DURATION"

ANALYSIS_DURATION="$(awk -v start="$START_SECONDS" -v end="$END_SECONDS" 'BEGIN {
  printf "%.6f", end - start
}')"

TIMESTAMPS="$(mktemp "${TMPDIR:-/tmp}/fluxdb-fps-pts.XXXXXX")"
FILTERED_TIMESTAMPS="$(mktemp "${TMPDIR:-/tmp}/fluxdb-fps-trim.XXXXXX")"
GAPS="$(mktemp "${TMPDIR:-/tmp}/fluxdb-fps-gaps.XXXXXX")"
cleanup() {
  rm -f "$TIMESTAMPS" "$FILTERED_TIMESTAMPS" "$GAPS"
}
trap cleanup EXIT INT TERM

# mpdecimate 后 showinfo 的 pts_time 是窗口实际产生新画面的时刻。
# 在持续滚动时，重复帧通常来自应用只渲染到 30fps、卡顿或录屏补帧。
ffmpeg -hide_banner -i "$VIDEO" \
  -vf "mpdecimate=hi=64*12:lo=64*5:frac=0.33,showinfo" \
  -an -f null - 2>&1 \
  | sed -n 's/.*pts_time:\([0-9][0-9.]*\).*/\1/p' >"$TIMESTAMPS" || true

awk -v start="$START_SECONDS" -v end="$END_SECONDS" \
  '$1 >= start && $1 < end' "$TIMESTAMPS" >"$FILTERED_TIMESTAMPS"

FRAMES="$(wc -l <"$FILTERED_TIMESTAMPS" | tr -d ' ')"
[ "$FRAMES" -gt 1 ] || die "有效帧不足；请确认录屏中目标内容持续变化，例如正在滚动结果"

awk 'NR == 1 { prev = $1; next } {
  gap = $1 - prev
  if (gap > 0) print gap * 1000
  prev = $1
}' "$FILTERED_TIMESTAMPS" | sort -n >"$GAPS"

read -r AVG_GAP P95_GAP MAX_GAP < <(
  awk '{
    value[NR] = $1
    sum += $1
  } END {
    if (NR == 0) {
      print "0 0 0"
      exit
    }
    idx = int(NR * 0.95)
    if (idx < 1) idx = 1
    if (idx > NR) idx = NR
    printf "%.2f %.2f %.2f\n", sum / NR, value[idx], value[NR]
  }' "$GAPS"
)

read -r EFFECTIVE_FPS DROPPED_RATIO < <(
  awk -v frames="$FRAMES" -v duration="$ANALYSIS_DURATION" -v target="$TARGET_FPS" 'BEGIN {
    effective = frames / duration
    expected = duration * target
    dropped = 1 - frames / expected
    if (dropped < 0) dropped = 0
    printf "%.2f %.2f\n", effective, dropped * 100
  }'
)

mkdir -p "$OUT_DIR"
RESULT_CSV="$OUT_DIR/fps-results.csv"
if [ ! -f "$RESULT_CSV" ]; then
  printf 'timestamp,label,video,duration_seconds,target_fps,unique_frames,effective_fps,dropped_ratio_percent,avg_gap_ms,p95_gap_ms,max_gap_ms\n' >"$RESULT_CSV"
fi

printf '%s,%s,%s,%.3f,%s,%s,%s,%s,%s,%s,%s\n' \
  "$(date -u +%Y-%m-%dT%H:%M:%SZ)" \
  "$LABEL" \
  "$(basename "$VIDEO")" \
  "$ANALYSIS_DURATION" \
  "$TARGET_FPS" \
  "$FRAMES" \
  "$EFFECTIVE_FPS" \
  "$DROPPED_RATIO" \
  "$AVG_GAP" \
  "$P95_GAP" \
  "$MAX_GAP" >>"$RESULT_CSV"

printf '分析完成：%s\n' "$LABEL"
printf '  视频时长：%s 秒\n' "$DURATION"
printf '  分析区间：%s–%s 秒（%s 秒）\n' "$START_SECONDS" "$END_SECONDS" "$ANALYSIS_DURATION"
printf '  有效帧数：%s\n' "$FRAMES"
printf '  有效帧率：%s fps（基准 %s fps）\n' "$EFFECTIVE_FPS" "$TARGET_FPS"
printf '  估算掉帧：%s%%\n' "$DROPPED_RATIO"
printf '  帧间隔：平均 %s ms，P95 %s ms，最大 %s ms\n' "$AVG_GAP" "$P95_GAP" "$MAX_GAP"
printf '  CSV：%s\n' "$RESULT_CSV"
