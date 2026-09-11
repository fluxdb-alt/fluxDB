// Included in crate-root scope by ../../src/main.rs.
//
// 日志系统初始化：基于 tracing + tracing-subscriber。
// - 默认同时输出到 stderr 与滚动日志文件（~/Library/Application Support/fluxdb/logs/）。
// - 输出级别使用保存的应用配置，不受运行环境变量影响。
// - 日志文件不记录连接明文的密码等内容，敏感数据需在埋点时避免写入。

use tracing_appender::non_blocking::WorkerGuard;
use tracing_subscriber::{EnvFilter, fmt, layer::SubscriberExt, util::SubscriberInitExt};

/// 应用数据根目录（与 fluxdb-storage 保持一致：~/Library/Application Support/fluxdb）。
/// 注意：本文件通过 include! 并入 crate-root 作用域，因此这里用全限定路径避免
/// 与 main.rs 及同作用域其他 include 文件的 use 冲突。
fn app_data_dir() -> std::path::PathBuf {
    match std::env::var_os("HOME") {
        Some(home) => std::path::PathBuf::from(home).join("Library/Application Support/fluxdb"),
        None => std::path::PathBuf::from("."),
    }
}

fn configured_log_dir(log_path: &str) -> std::path::PathBuf {
    if log_path.trim().is_empty() {
        app_data_dir().join("logs")
    } else {
        std::path::PathBuf::from(log_path)
    }
}

/// 组装日志过滤规则。日志级别只来自保存的应用配置，不读取启动环境变量。
fn build_filter(configured_level: LogLevel) -> EnvFilter {
    // 第三方库（gpui/sqlx 等）保持 warn，避免刷屏，仅保留本项目日志。
    // `parse_lossy` 含有显式第三方 directive 时不会自动应用 builder 的默认值，
    // 因此必须显式追加应用级默认 directive，否则 Debug/Info 日志会全部被过滤。
    EnvFilter::builder()
        .parse_lossy("gpui=warn,sqlx=warn,tracing_subscriber=warn,tower_http=warn,hyper=warn,sqlparser=warn")
        .add_directive(match configured_level {
            LogLevel::Error => tracing::Level::ERROR.into(),
            LogLevel::Warn => tracing::Level::WARN.into(),
            LogLevel::Info => tracing::Level::INFO.into(),
            LogLevel::Debug => tracing::Level::DEBUG.into(),
            LogLevel::Trace => tracing::Level::TRACE.into(),
        })
}

/// 初始化全局日志系统。应在 main() 最早处调用一次（唯一入口），
/// 返回值 guard 需被持有到进程退出，否则日志 worker 线程会被立即关闭。
/// 探测目录是否真正可写。
///
/// 背景：log_path 可能指向 TCC 保护目录（如 ~/Downloads、~/Documents），
/// 从 Finder/LaunchServices 启动的 app 自身没有授权，创建文件会得到
/// "Operation not permitted"（EPERM）。tracing-appender 内部对初始化失败
/// 直接 expect panic，会导致整个应用启动闪退，因此必须先探针、失败则回退。
fn probe_dir_writable(dir: &std::path::Path) -> bool {
    if std::fs::create_dir_all(dir).is_err() {
        return false;
    }
    let probe = dir.join(".fluxdb-write-probe");
    match std::fs::File::create(&probe) {
        Ok(_) => {
            let _ = std::fs::remove_file(&probe);
            true
        }
        Err(err) => {
            eprintln!(
                "[fluxdb] 日志目录不可写（{}: {}），本次启动仅输出到 stderr；请在设置中更换日志路径",
                dir.display(),
                err
            );
            false
        }
    }
}

pub fn init_logging(configured_level: LogLevel, log_path: &str) -> Option<WorkerGuard> {
    let filter = build_filter(configured_level);

    // stderr 输出层：带时间戳、target、线程、字段。
    let stderr_layer = fmt::layer()
        .with_target(true)
        .with_thread_ids(true)
        .with_writer(std::io::stderr);

    // 文件输出层：滚动写入应用数据目录的 logs/ 下，前缀 gdb，按日期滚动。
    // 探针失败时跳过文件层，仅保留 stderr，保证应用本体可启动。
    let log_dir = configured_log_dir(log_path);
    if !probe_dir_writable(&log_dir) {
        tracing_subscriber::registry()
            .with(filter)
            .with(stderr_layer)
            .init();
        return None;
    }

    let file_appender = tracing_appender::rolling::daily(log_dir, "fluxdb.log");
    let (file_writer, file_guard) = tracing_appender::non_blocking(file_appender);
    let file_layer = fmt::layer()
        .with_target(true)
        .with_thread_ids(true)
        .with_ansi(false)
        .with_writer(file_writer);

    // 组合 subscriber：EnvFilter 控制级别，stderr + 文件两层输出。
    tracing_subscriber::registry()
        .with(filter)
        .with(stderr_layer)
        .with(file_layer)
        .init();

    Some(file_guard)
}
