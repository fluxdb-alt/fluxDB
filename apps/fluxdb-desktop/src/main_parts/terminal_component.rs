// 终端组件（真正的终端模拟器 UI surface）：PTY 会话 + grid + adapter + 渲染 + 键盘输入。
//
// 子文件被 include 进 crate-root 命名空间（与其它 main_parts/*.rs 一致）：
//   - transport.rs：真实 PTY 传输（portable-pty）与后台读线程。
//   - model.rs：TerminalComponent 生命周期、去往 PTY 的键盘动作与危险命令确认。
//   - render.rs：把 TermGrid 画到 GPUI 的自定义 Element（含状态栏 / 确认条）。
//
// 本组件可被 Redis CLI / MySQL CLI / SSH 等多后端复用：差别只在于 adapter（fluxdb-app 提供）。
include!("terminal_component/transport.rs");
include!("terminal_component/model.rs");
include!("terminal_component/render.rs");
include!("terminal_component/app.rs");
