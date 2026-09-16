# FluxDB Windows / Linux 适配实施状态

依据 `docs/2026-09-14-windows-linux-support-plan.md`（尤其 §10、§10.5）记录。每个任务结束更新一次。

## 总览

| 任务 | 状态 |
| --- | --- |
| AI-00 复查基线、依赖、原生环境，建立最小 CI 构建任务 | 通过（三平台 fmt/check/release 构建/产物/core 测试全绿，仅剩既有 connector 失败） |
| AI-01 目录/日志/资源定位 | 未开始 |
| AI-02 凭据后端与错误传播 | 未开始 |
| AI-03 窗口/托盘/字体/快捷键/IME | 未开始 |
| AI-04 工具执行/SSH 隧道/PTY | 未开始 |
| AI-05 原生 release 打包 | 未开始 |
| AI-06 验收与发布材料 | 未开始 |

---

## AI-00

**任务 ID / 状态**：AI-00 / 通过（run 4 证实三平台 fmt/check/release 构建/产物/core 测试全绿；唯一失败为既有 connector 测试，与跨平台无关）。

**代码基线**：
- commit：`2e299c8507ece2d77d71e830657c8b36debe50c2`（`feat(tls): MySQL TLS 参数生效注入与连接对话框 SSL 模式下拉化`）
- git status：工作区干净，无未提交用户改动
- 分支：`main`（领先 `origin/main` 158 个提交）
- 本地 Rust：`rustc 1.97.0 (2d8144b78 2026-07-07)`，`cargo 1.97.0`，默认 toolchain `stable-aarch64-apple-darwin`
- 已安装 target：aarch64-apple-darwin、aarch64-unknown-linux-musl、x86_64-unknown-linux-gnu、x86_64-unknown-linux-musl
- 无 `rust-toolchain` 锁定文件（CI 目前用浮动 stable，AI-00 记为风险）

**目标环境**：
- macOS（本地，可用）：Apple Silicon，`stable-aarch64-apple-darwin`
- Windows x64（无本机，用 GitHub Actions 原生 runner）：`x86_64-pc-windows-msvc`，windows-2022
- Linux x64（无本机，用 GitHub Actions 原生 runner）：`x86_64-unknown-linux-gnu`，ubuntu-22.04 基线
- CI：GitHub Actions 已启用（`Actions permissions: allow all`）；远程 `origin = git@github.com:fluxdb-alt/fluxDB.git`，`gh` 登录账号 `fluxdb-alt`

**依赖版本（据 Cargo.lock 核对，与方案一致）**：
- `gpui-pre 0.3.3`、`gpui-pre-platform 0.3.3`、`gpui-component 0.6.0`、`gpui-base 0.6.0`
- `gpui-pre-linux 0.3.3`、`gpui-pre-windows 0.3.3`、`gpui-pre-wgpu 0.3.3`
- `tray-icon 0.23.1`（desktop，`default-features=false`）
- `ssh2 0.9.6` → `libssh2-sys 0.3.2`、`openssl-sys 0.9.117`
- `rusqlite 0.32.1`（`bundled`）→ `libsqlite3-sys 0.30.1`（本地另有 0.36.0 但未入锁）
- `tree-sitter =0.26.13`、`tree-sitter-json =0.24.8`、`tree-sitter-sequel =0.3.11`
- `portable-pty 0.9.0`

**改动**：
- 新增 `.github/workflows/ci.yml`：三平台（macOS aarch64 / Windows MSVC / Ubuntu 22.04）原生矩阵。步骤：checkout → Linux apt 依赖安装 → rust-toolchain(stable, 带 target) → Swatinem/rust-cache → `cargo fmt --all -- --check` → `cargo check --workspace --locked --target` → 两组单测（core/editor；storage/app/connectors）→ `cargo build --locked --release -p fluxdb-desktop` → 上传内部候选产物（retention 14 天）。只生成内部候选，不创建 tag、不发布 Release。`concurrency cancel-in-progress` 控制消耗，job `timeout-minutes: 90`。
- Linux 托盘 `tray-icon` 条件裁剪（AI-00 最小修复，见 §4.3/§10.3 允许项）：
  - `apps/fluxdb-desktop/Cargo.toml`：把 `tray-icon` 移入 `[target.'cfg(not(target_os = "linux"))'.dependencies]`，Linux 不再编译/链接 GTK（tray-icon 在 Linux 会无条件拉 libappindicator→gtk）。
  - `apps/fluxdb-desktop/src/main.rs`：`include!("main_parts/tray_icon.rs")` 加 `#[cfg(not(target_os = "linux"))]`。
  - `apps/fluxdb-desktop/src/main_parts/app_boot.rs`：`install_tray_icon(cx)` 与 `install_close_to_tray(window, cx)` 两处调用加 `#[cfg(not(target_os = "linux"))]`；Linux 走默认正常关闭/退出流程（不注册“关闭到托盘”拦截）。Windows/macOS 托盘行为不变。
  - Cargo.lock 未改动（同版本仅移动 target 条件）。
- **AI-00 首次 CI 跑暴露并修复的跨平台问题（三个提交）**：
  - **Linux 编译**：Ubuntu `cargo check` 报 `E0425/E0433: cannot find type RefCell`。根因：`tray_icon.rs` 顶部 `use std::cell::RefCell;` 依托 `include!` 的 crate-root 共享作用域，被 `app_boot.rs:2378`、`app_state.rs:712` 裸用；Linux 排除托盘 include 后导出消失才暴露。修复：两处裸 `RefCell` 改为全限定 `std::cell::RefCell`（与同文件其他写法一致）。提交 `8418daf`。
  - **CI 顺序**：connectors 既有失败用例中断 job，导致 release 构建未跑。把 `cargo build --release` 与 artifact 上传移到测试之前（release 产物总是产出验证，测试仍如实执行）。提交 `00aa831`。
  - **Linux release 链接**：Ubuntu release 链接报 `rust-lld: error: unable to find library -lxkbcommon-x11`。链接行其余 `-lxcb/-lfontconfig/-lfreetype/-lxkbcommon/-lssl/-lcrypto` 均由现有 apt 包满足，仅缺 xkbcommon 的 X11 变体。按实际日志固化包名（方案 §3.4）：apt 加 `libxkbcommon-x11-dev`。提交 `beec408`。

**为何依赖/feature 决策与源码一致（已实读本地 registry 源码确认）**：
- `gpui-pre-windows 0.3.3`：build.rs 仅在 `cfg(target_os="windows")` 且 `not(debug_assertions)` 时用 `fxc.exe` 编译 HLSL（release-only）；debug 构建 build.rs 为空。release 构建需要 fxc.exe（`GPUI_FXC_PATH` → PATH `where fxc.exe` → Windows SDK 注册表扫描），缺失则 `panic!`。windows-2022 镜像自带 VS Build Tools + Windows SDK 的 fxc。
- `gpui-pre-linux 0.3.3`：默认 feature `["wayland","x11"]`；wayland 依赖 wayland-client/cursor/protocols/xkbcommon(wayland)；x11 依赖 x11rb(0.13.1)/xkbcommon(x11)/x11-clipboard。链接阶段需要 xkbcommon、wayland、X11/xcb/xrandr/xi/xcursor 开发库。
- `gpui-base 0.6.0`：非 WASM target 依赖显式启用 `gpui-pre-platform` 的 `x11`+`wayland` feature —— 与方案 §3.1 一致，当前项目 Linux 图形后端确实已由组件间接启用。
- `gpui-pre-platform 0.3.3`：默认 feature 为空；`x11`/`wayland` 各映射到 `gpui_linux/x11|wayland`；**没有** `windows`/`macos` cargo feature（是 target 条件依赖，不是 feature）。
- `tray-icon 0.23.1`：`default-features=false` 只去掉 `libxdo`；Linux/BSD target 仍**无条件**引入 `libappindicator 0.9` → `gtk 0.18`/`glib`。即 Linux 上即使不调用托盘函数、即使 default-features=false，GTK 仍会被链接。→ 印证宏决定：Linux 必须从依赖图排除 tray-icon，不能只靠“不调用”。已按此把 desktop 的 tray-icon 改为 `cfg(not(target_os="linux"))` 目标条件依赖。
- `libssh2-sys 0.3.2`：直接用 `cc` 编译 vendored libssh2 源码（不需 cmake/autotools）；Unix 链接 OpenSSL（`openssl-sys 0.9.117`）；Windows 默认用 CNG 加密（`LIBSSH2_WINCNG`，不开 `openssl-on-win32`），MSVC 下若 vcpkg 可用会优先用 vcpkg。
- `openssl-sys 0.9.117`：`openssl-sys` 非可选；Linux 构建需要 `libssl-dev`（已入 CI apt 清单）。仅凭数据库用 rustls 不能免除 OpenSSL —— 与方案 §3.2 一致。
- `rusqlite 0.32.1`：storage 用 `bundled` feature → libsqlite3-sys 0.30.1 从源码编译 sqlite，不需要系统 sqlite3。锁文件中 `libsqlite3-sys` 仅 0.30.1 一个版本，rusqlite 与 sqlx-sqlite 共用，**无版本冲突**（本地 registry 里的 0.36.0 未入锁）。
- `portable-pty 0.9.0`：Windows 用 `winapi 0.3`（ConPTY，winuser/consoleapi/fileapi/namedpipeapi 等）；非 Windows 用 `nix 0.28`/`libc 0.2`。
- `tree-sitter =0.26.13`、`=0.24.8`、`sequel =0.3.11`：均为 C 解析器，配目标 C 工具链即可，不要求用户装 tree-sitter CLI。
- 全部锁版本与方案一致：`gpui-pre 0.3.3`、`gpui-component 0.6.0`、`tray-icon 0.23.1`、`ssh2 0.9.6`、`rusqlite 0.32.1`、`tree-sitter 0.26.13`、`portable-pty 0.9.0`。

**验证（真实执行/退出码，全部在本地 macOS Apple Silicon）**：
- `cargo check --workspace --locked` → 退出码 0
- `cargo check --locked -p fluxdb-desktop`（tray 裁剪后）→ 编译通过
- `cargo build --locked --release -p fluxdb-desktop`（tray 裁剪前基线）→ 退出码 0
- `cargo fmt --all -- --check` → 通过
- `git diff --check` → 通过
- `cargo test --locked -p fluxdb-core -p fluxdb-editor-core -p fluxdb-editor-language` → 通过（1 passed, 0 failed）
- `cargo test --locked -p fluxdb-storage` → 通过（22 passed, 0 failed）
- `cargo test --locked -p fluxdb-app` → 通过（459 passed, 0 failed, 1 ignored）
- `cargo test --locked -p fluxdb-connectors` → **失败（4 个，均为既有、与 AI-00 无关）**：
  - `pg_plan_apply_tests::pg_apply_role_plan_end_to_end_and_rename`、`::pg_apply_role_plan_rolls_back_as_a_whole`：`Connection refused (os error 61)`，需要真实运行的 PostgreSQL 服务（集成测试）。
  - `tests::pg_create_database_sql_builds_options_and_quotes`：断言失配（预期无 `TEMPLATE template0`）。
  - `tests::pg_build_table_ddl_round_trips_clauses`：断言失配（GENERATED 子句）。
  - 这 4 个失败与本次 AI-00 改动无关（AI-00 仅改 desktop 托盘 cfg；失败都在 `crates/fluxdb-connectors`，改动前即可复现）。属既有基线问题，不在 AI-00 范围；对应的 CI 步骤保留（不能靠跳过假装通过），会如实保持失败，由后续任务（或独立修复）处理。
- Windows / Linux：无本机，用 GitHub Actions 原生 runner；mac 上交叉 check 不能作为这些平台的验证（已确证由 CI 原生 runner 验证）

**验证（GitHub Actions 原生 runner 实际运行）**：
- **Windows x86_64-pc-windows-msvc（windows-2022）**：`cargo fmt --check` ✓；`cargo check --workspace --locked` ✓；core/editor 测试 ✓；**release 构建 ✓ 且上传候选产物 ✓**（run 3，18m+，含 fxc/HLSL release 链路，链接通过）；`Test storage/app/connectors` ✗ 为既有 connector 失败。
- **macOS aarch64（macos-14）**：fmt ✓；workspace check ✓；core 测试 ✓；**release 构建 ✓ 且上传候选产物 ✓**（run 3）；connector 测试 ✗ 为既有失败。
- **Ubuntu 22.04 x86_64**（run 4 全过到 connector 测试）：
  - run 1（35086174873）：workspace check ✗ `exit 101`（`RefCell` 作用域，已修复）。
  - run 2（35087627727）：workspace check ✓（RefCell 修复生效）；connector 测试 ✗ 为既有失败。
  - run 3（35088628163）：workspace check ✓；release build ✗ `rust-lld: unable to find library -lxkbcommon-x11`（已修复，apt 加 `libxkbcommon-x11-dev`，提交 beec408）。
  - run 4（35090704424）：**全链路通过到 connector 测试为止**：fmt ✓ / workspace check ✓ / **release build ✓** / 候选产物上传 ✓ / core 测试 ✓。
- 三平台既有 connector 4 个失败（三平台均复现）：`pg_plan_apply_*`（2 个，需真实 PG 服务，Connection refused）、`pg_create_database_sql_builds_options_and_quotes`、`pg_build_table_ddl_round_trips_clauses`（2 个真实断言失配）。基线缺陷，非 AI-00 引入，保留不跳过。

**CI 链接/日志**（PR #5）：
- run 1：https://github.com/fluxdb-alt/fluxDB/actions/runs/35086174873
- run 2：https://github.com/fluxdb-alt/fluxDB/actions/runs/35087627727
- run 3：https://github.com/fluxdb-alt/fluxDB/actions/runs/35088628163
- run 4：https://github.com/fluxdb-alt/fluxDB/actions/runs/35090704424

**GUI/安装**：
- 未覆盖（无 Windows/Linux 实体图形会话）；托盘/DPI/IME/真实 GPU 标记待验证

**已知问题**：
- 未验证项（属于 AI-03 及后续）：Windows/Linux GUI、IME、DPI、真实 GPU、托盘（Linux 首版排除）、安装包/运行库；Linux 桌面/X11/Wayland 图形会话；Windows fxc/HLSL 仅验证了 release 链接通过，未验证运行期着色器渲染。这些均不影响 AI-00 的“三平台原生编译/链接/测试/产物”基础。
- 既有基线失败（非 AI-00 引入，阻塞 CI 全绿）：connectors 4 个测试失败（三平台一致，`178 passed; 4 failed; 16 ignored`）。2 个需真实 PG 服务（Connection refused）、2 个真实断言失配。已如实保留，不经由跳过伪装通过；属既有质量负债，与跨平台无关，建议独立修复或按需给 connector 测试启动真实 PG（AI-04 集成范畴），否则该步骤三平台长红。
- 待验证/后续决策：`gpui-fps 0.6.0` 默认启用 profiler（方案 §3.1 建议改诊断 feature 默认关闭），AI-00 不改动，标记为后续；CI 用浮动 stable 工具链（无 rust-toolchain 锁），方案 §3.1 建议固定通过验证的版本，后续落实。

**阻塞项**：
- AI-00 本身无硬阻塞：三平台原生 fmt/check/**release 构建**/产物上传/core 测试均已通过（run 4）。
- CI 全绿：暂被既有 connector 测试失败阻塞（三平台同因），属既有缺陷，非 AI-00 引入。
- 正式跨平台可用声明：等待后续 AI-01～AI-06 与桌面 GUI 验收，见第 7 节门槛；当前仅内部候选产物。

**下一步**：
- 更新并提交本状态文档到分支（含 run 4 全链路结果）。
- 触发 doc-only 复跑确认最终状态一致后可关闭 draft PR（不合并到 main，供审查）。
- 既有 4 个 connector 测试失败与跨平台无关，评估独立修复/启动真实 PG，避免三平台长红。
- 进入 AI-01（目录/日志/资源定位），需先确认真机/VM 与发布布局预期。
- 需要的环境：GitHub Actions 原生 runner（已有）；Windows 11 / Linux 桌面 VM 用于后续 GUI/IME/DPI 验收（非 AI-00 必需）。

---
