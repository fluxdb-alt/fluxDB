#!/usr/bin/env bash
# 本地门禁：提交/合并前跑这一条，等价于 CI 里三平台都跑的那几步的本地版本。
#
# 与 .github/workflows/ci.yml 的关系：本脚本覆盖 fmt 与 Rust 静态检查/编译，
# 但**不能**替代 Windows/Linux 的原生构建（Mac 本地 cargo check 看不到
# fxc/HLSL、OpenSSL、xkbcommon 等平台差异）。跨平台验证靠打 tag 前手动
# dispatch 一次 CI 工作流，见 AGENTS.md「测试与验收」。
set -euo pipefail

cd "$(dirname "$0")/.."

echo "==> 检查界面文本字号是否走 rem"
# 界面字号通过 gpui 的 rem 基准整体缩放，文本尺寸必须用 ui_text_size(...)；
# 编辑器/终端等有独立字号的代码区域在下方白名单内。
raw_text_sizes=$(grep -rnE '\.(text_size|line_height)\(px\([0-9]' \
    apps/fluxdb-desktop/src/main_parts \
    --include='*.rs' \
    | grep -vE '/(editor_component|json_editor|json_component|terminal_component)/|/sql_preview\.rs:|text_size\(px\(0\.\)\)' \
    || true)
if [[ -n "$raw_text_sizes" ]]; then
    echo "以下界面文本仍使用硬编码 px 字号，无法跟随「界面字号」缩放：" >&2
    echo "$raw_text_sizes" >&2
    echo "请改用 ui_text_size(...)（定义见 foundation.rs）。" >&2
    exit 1
fi

echo "==> cargo fmt --all"
cargo fmt --all

echo "==> cargo check --workspace --locked"
cargo check --workspace --locked

echo "==> cargo clippy --workspace --all-targets --locked"
cargo clippy --workspace --all-targets --locked

echo "本地门禁通过。跨平台验证：打 tag 前手动跑一次 CI（workflow dispatch）。"