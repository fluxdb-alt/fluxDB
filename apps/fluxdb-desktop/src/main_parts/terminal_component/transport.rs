// 真实 PTY 传输边界：拉起子进程、读写字节流、resize、关闭。
//
// 边界只关心“字节流”，不懂 Redis/MySQL/SSH 语义——对应 fluxdb-core 的 `TerminalTransport`。
// 由 portable-pty 提供跨平台 PTY；读线程在后台阻塞读，把增量字节/退出事件送回 UI 线程的 channel。

// `as _`：仅导入方法、不把名字绑定进 crate-root 作用域，避免与其它 include! 文件
// / main.rs 的 `use gpui::prelude::*` 发生 E0252 重名冲突。
use std::io::Read as _;
use std::sync::mpsc::{channel, Receiver, Sender};

use portable_pty::{CommandBuilder, NativePtySystem, PtySize, PtySystem};

/// 读线程回传给 UI 的事件。
enum TerminalTransportEvent {
    /// 一段增量输出（交给 adapter + grid 解析）。
    Output(Vec<u8>),
    /// 子进程侧 EOF（多为进程退出）。
    Exited,
}

/// PTY 传输句柄：持有 master（供 resize）与 writer；reader 在后台线程。
struct TerminalPty {
    master: Box<dyn portable_pty::MasterPty + Send>,
    writer: Option<Box<dyn Write + Send>>,
    /// 子进程句柄（UI 线程持有，供 close 时 kill）。
    child: Option<Box<dyn portable_pty::Child + Send + Sync>>,
    /// UI 线程消费的读取事件接收端。
    rx: Receiver<TerminalTransportEvent>,
    /// 写入侧错误（最后一次失败原因）。
    last_error: Option<String>,
}

impl TerminalPty {
    /// 按给定启动参数拉起 PTY 子进程，并启动后台读线程。
    fn spawn(spec: &fluxdb_core::terminal::TerminalSpawnSpec) -> anyhow::Result<Self> {
        let pty_system = NativePtySystem::default();
        let pair = pty_system.openpty(PtySize {
            rows: spec.rows,
            cols: spec.cols,
            pixel_width: 0,
            pixel_height: 0,
        })?;

        // 从 Finder / LaunchServices 启动桌面应用时，进程通常拿不到 shell 初始化脚本
        // 注入的 PATH（尤其是 Homebrew 的 /opt/homebrew/bin）。先按当前 PATH 查找，
        // 再补充 macOS 常见安装目录，避免 redis-cli 明明已安装却被报告为“启动失败”。
        let program = resolve_program_path(&spec.program);
        let mut cmd = CommandBuilder::new(&program);
        cmd.args(&spec.args);
        for (key, value) in &spec.env {
            cmd.env(key.clone(), value.clone());
        }
        if let Some(cwd) = &spec.cwd {
            cmd.cwd(cwd);
        }

        // 子进程在属主 slave 上运行；drop slave 释放资源。
        let child = pair.slave.spawn_command(cmd)?;
        drop(pair.slave);

        let mut reader = pair.master.try_clone_reader()?;
        let master_writer = pair.master.take_writer()?;
        let (tx, rx): (Sender<TerminalTransportEvent>, Receiver<TerminalTransportEvent>) = channel();

        // 后台读线程：阻塞读 PTY，读到增量即回传；EOF 表示进程退出。
        std::thread::spawn(move || {
            let mut buffer = [0u8; 4096];
            loop {
                match reader.read(&mut buffer) {
                    Ok(0) => {
                        let _ = tx.send(TerminalTransportEvent::Exited);
                        break;
                    }
                    Ok(n) => {
                        if tx.send(TerminalTransportEvent::Output(buffer[..n].to_vec())).is_err() {
                            break;
                        }
                    }
                    Err(_) => {
                        let _ = tx.send(TerminalTransportEvent::Exited);
                        break;
                    }
                }
            }
        });

        Ok(TerminalPty {
            master: pair.master,
            writer: Some(master_writer),
            child: Some(child),
            rx,
            last_error: None,
        })
    }

    /// 非阻塞取一条读事件；无则在界面上表现为“等待”。
    fn try_recv(&mut self) -> Option<TerminalTransportEvent> {
        match self.rx.try_recv() {
            Ok(event) => Some(event),
            Err(std::sync::mpsc::TryRecvError::Empty) => None,
            Err(std::sync::mpsc::TryRecvError::Disconnected) => None,
        }
    }

    /// 把用户输入字节写入 PTY（错误记录到 last_error，不 panic）。
    fn write(&mut self, data: &[u8]) {
        if let Some(writer) = self.writer.as_mut() {
            if let Err(err) = writer.write_all(data) {
                self.last_error = Some(err.to_string());
            }
            let _ = writer.flush();
        }
    }

    /// 通知子进程改变视口尺寸（内核 grid 也同步 reflow）。
    fn resize(&mut self, cols: u16, rows: u16) {
        let _ = self.master.resize(PtySize { cols, rows, pixel_width: 0, pixel_height: 0 });
    }

    /// 子进程已 EOF（多为退出）后，取回真实退出状态（非阻塞解析一次）。
    /// 端口：Read 线程只拿到 EOF，拿不到退出码；UI 线程持有 child，据此 `try_wait()`。
    /// 返回 `(退出码, 信号名)`；尚未退出 / 解析失败时返回 None。正常退出 0、参数/连接失败非 0。
    fn takeover_exit_status(&mut self) -> Option<(u32, Option<String>)> {
        let child = self.child.as_mut()?;
        match child.try_wait() {
            Ok(Some(status)) => Some((status.exit_code(), status.signal().map(String::from))),
            // 进程可能仍在回读 EOF→ 真正退出的间隙，这里读不到就回 None，交给 on_exit 兜底。
            Ok(None) | Err(_) => None,
        }
    }

    /// 关闭：杀掉子进程并丢弃 writer；读线程随之 EOF 退出。
    fn close(&mut self) {
        self.writer.take();
        if let Some(mut child) = self.child.take() {
            let _ = child.kill();
        }
    }
}

/// 为 PTY 子进程解析可执行文件路径。
///
/// `std::process::Command` 在 PATH 缺失时不会自动知道 Homebrew 等包管理器目录；
/// 这里仅对不带目录的程序名做解析，显式路径仍完全按调用方指定的值使用。
fn resolve_program_path(program: &str) -> String {
    let path = std::path::Path::new(program);
    if path.is_absolute() || program.contains('/') || program.contains('\\') {
        return program.to_string();
    }

    let mut search_paths: Vec<std::path::PathBuf> = std::env::var_os("PATH")
        .map(|value| std::env::split_paths(&value).collect())
        .unwrap_or_default();

    #[cfg(target_os = "macos")]
    {
        // Finder 启动时常见的 PATH 不包含这些目录；按稳定顺序去重，避免重复 stat。
        for directory in ["/opt/homebrew/bin", "/usr/local/bin", "/opt/local/bin"] {
            let directory = std::path::PathBuf::from(directory);
            if !search_paths.contains(&directory) {
                search_paths.push(directory);
            }
        }
    }

    for directory in search_paths {
        let candidate = directory.join(program);
        if candidate.is_file() {
            let resolved = candidate.to_string_lossy().into_owned();
            tracing::debug!(program, resolved = %resolved, "终端已解析可执行文件路径");
            return resolved;
        }
    }

    tracing::warn!(program, "终端未找到可执行文件，将交由 PTY 返回启动错误");
    program.to_string()
}
