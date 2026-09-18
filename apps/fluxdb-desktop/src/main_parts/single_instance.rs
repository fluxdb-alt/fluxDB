// Included in crate-root scope by ../../src/main.rs.
//
// 单实例守卫（方案 §12.1，选定方案 A）：
// - Unix（macOS/Linux）：对锁文件 flock 独占；锁随进程退出（含强杀）自动释放，
//   不存在"崩溃后无法再启动"的陈旧锁。
// - Windows：命名互斥量 `Global\FluxDBSingleInstance`，生命周期由 OS 管理，语义相同。
// 第二实例：明确提示并退出，不静默失败。锁基础设施失败时降级放行（单实例是并发
// 保护而非安全边界），并输出诊断日志。

/// 尝试成为唯一实例。返回 true 表示本进程持有单实例锁（或基础设施降级放行）；
/// 返回 false 表示已有实例在运行。
pub fn acquire_single_instance() -> bool {
    #[cfg(unix)]
    {
        acquire_single_instance_unix()
    }
    #[cfg(windows)]
    {
        acquire_single_instance_windows()
    }
    #[cfg(not(any(unix, windows)))]
    {
        // 非目标平台：无对应锁原语，放行（本项目首版仅支持三大平台）。
        true
    }
}

#[cfg(unix)]
fn acquire_single_instance_unix() -> bool {
    let lock_path = fluxdb_storage::runtime_lock_file();
    if let Some(parent) = lock_path.parent() {
        // 锁文件目录缺失时尽力创建（XDG_RUNTIME_DIR 通常已存在）。
        let _ = std::fs::create_dir_all(parent);
    }
    let file = match std::fs::OpenOptions::new()
        .create(true)
        .truncate(false)
        .write(true)
        .open(&lock_path)
    {
        Ok(file) => file,
        Err(err) => {
            tracing::warn!(
                target: "fluxdb_desktop",
                error = %err,
                path = %lock_path.display(),
                "单实例锁文件无法创建，降级放行（无并发保护）"
            );
            return true;
        }
    };

    // 独占非阻塞 flock：失败即已有实例。Flock 句柄需持有到进程结束才保持锁，
    // 这里用 forget 避开其 Drop 解锁（进程退出由 OS 回收并释放锁）。
    match nix::fcntl::Flock::lock(file, nix::fcntl::FlockArg::LockExclusiveNonblock) {
        Ok(locked) => {
            std::mem::forget(locked);
            true
        }
        Err(_) => false,
    }
}

#[cfg(windows)]
fn acquire_single_instance_windows() -> bool {
    use windows::core::PCWSTR;
    use windows::Win32::Foundation::{GetLastError, ERROR_ALREADY_EXISTS};
    use windows::Win32::System::Threading::CreateMutexW;

    let mutex_name: Vec<u16> = "Global\\FluxDBSingleInstance\0"
        .encode_utf16()
        .collect();
    // 命名互斥量：持有句柄存活期间锁有效，进程退出（含强杀）由 OS 释放。
    match unsafe { CreateMutexW(None, false, PCWSTR(mutex_name.as_ptr())) } {
        Ok(handle) => {
            // GetLastError 必须紧跟创建调用读取：已有实例时句柄同样有效但错误码为 ALREADY_EXISTS。
            if unsafe { GetLastError() } == ERROR_ALREADY_EXISTS {
                false
            } else {
                // HANDLE 是裸句柄包装（Copy，无 Drop）：无需也无法显式持有，
                // 命名互斥量随进程退出由 OS 统一释放。
                let _ = handle;
                true
            }
        }
        Err(err) => {
            tracing::warn!(
                target: "fluxdb_desktop",
                error = %err,
                "单实例命名互斥量创建失败，降级放行（无并发保护）"
            );
            true
        }
    }
}

/// 第二实例的提示与退出：Windows GUI 子系统无控制台，用系统消息框保证可见；
/// macOS/Linux 用 stderr。均以非零码退出（§10.4-4：失败必须有证据）。
pub fn notify_second_instance_and_exit() -> ! {
    #[cfg(windows)]
    {
        use windows::core::{HSTRING, PCWSTR};
        use windows::Win32::UI::WindowsAndMessaging::{
            MessageBoxW, MB_ICONINFORMATION, MB_OK,
        };
        let title = HSTRING::from("FluxDB");
        let text = HSTRING::from("FluxDB 已经在运行。\n请使用已打开的 FluxDB 窗口。");
        unsafe {
            let _ = MessageBoxW(
                None,
                PCWSTR(text.as_ptr()),
                PCWSTR(title.as_ptr()),
                MB_OK | MB_ICONINFORMATION,
            );
        }
    }
    eprintln!("FluxDB 已在运行，本次启动退出。");
    std::process::exit(1);
}
