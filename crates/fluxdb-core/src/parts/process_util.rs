// 外部命令子进程的跨平台辅助。

/// 在 Windows 下为子进程设置 `CREATE_NO_WINDOW`，避免备份/恢复、客户端检测、
/// SQL 文件执行等操作在 Windows 上弹出黑色控制台窗口；非 Windows 平台原样返回。
#[cfg(target_os = "windows")]
pub fn no_console(mut command: std::process::Command) -> std::process::Command {
    use std::os::windows::process::CommandExt;
    const CREATE_NO_WINDOW: u32 = 0x0800_0000;
    command.creation_flags(CREATE_NO_WINDOW);
    command
}

/// 非 Windows 平台：无需隐藏控制台，原样返回。
#[cfg(not(target_os = "windows"))]
pub fn no_console(command: std::process::Command) -> std::process::Command {
    command
}
