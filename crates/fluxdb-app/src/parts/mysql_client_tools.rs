//! MySQL 客户端管理：发现本机安装、探测发行版、安装 Windows 官方客户端。
//! 与 PostgreSQL 共用限时探测和安全 ZIP 下载；不把 PG 的版本兼容规则用于 MySQL。
use super::{
    MySqlClientVersion, PgClientDownloadProgress, PgClientSource, install_native_client_files,
    internal_error, mysql_client_version, native_client_version_output,
};
use fluxdb_core::Settings;
use std::{
    collections::BTreeSet,
    path::{Path, PathBuf},
    sync::atomic::{AtomicBool, Ordering},
};

pub const MYSQL_CLIENT_VERSION: &str = "8.4.6";
pub const MYSQL_CLIENT_DEFAULT_SOURCE: &str =
    "https://cdn.mysql.com/archives/mysql-8.4/mysql-{version}-{platform}.zip";
const INSTALL_MARKER: &str = ".fluxdb-install-complete";

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum MySqlClientTool {
    Dump,
    Console,
}

impl MySqlClientTool {
    fn names(self) -> [&'static str; 2] {
        match self {
            Self::Dump => ["mysqldump", "mariadb-dump"],
            Self::Console => ["mysql", "mariadb"],
        }
    }
    fn find_in(self, dir: &Path) -> Option<PathBuf> {
        self.names()
            .iter()
            .map(|name| {
                dir.join(if cfg!(windows) {
                    format!("{name}.exe")
                } else {
                    name.to_string()
                })
            })
            .find(|path| path.is_file())
    }
}

#[derive(Clone, Debug)]
pub struct MySqlClientLocation {
    pub program: PathBuf,
    pub version: MySqlClientVersion,
    pub source: PgClientSource,
}

pub fn mysql_client_managed_root() -> PathBuf {
    fluxdb_storage::FileStorage::default_root()
        .unwrap_or_else(|_| PathBuf::from(".fluxdb"))
        .join("clients/mysql")
}

pub fn mysql_client_install_dir(settings: &Settings) -> PathBuf {
    let configured = settings.mysql_client_dir.trim();
    if configured.is_empty() {
        return mysql_client_managed_root().join(MYSQL_CLIENT_VERSION);
    }
    let path = PathBuf::from(configured);
    if path
        .file_name()
        .is_some_and(|name| name.eq_ignore_ascii_case("bin"))
    {
        return path
            .parent()
            .filter(|parent| !parent.as_os_str().is_empty())
            .unwrap_or(Path::new("."))
            .to_path_buf();
    }
    path
}

pub fn mysql_client_download_supported() -> bool {
    cfg!(all(target_os = "windows", target_arch = "x86_64"))
}

pub fn mysql_client_download_url(source: &str) -> Option<String> {
    if !mysql_client_download_supported() {
        return None;
    }
    Some(
        if source.trim().is_empty() {
            MYSQL_CLIENT_DEFAULT_SOURCE
        } else {
            source.trim()
        }
        .replace("{version}", MYSQL_CLIENT_VERSION)
        .replace("{platform}", "winx64"),
    )
}

pub fn mysql_client_install_hint() -> &'static str {
    if cfg!(target_os = "macos") {
        "未找到可用的 MySQL 客户端。请执行 brew install mysql-client@8.4，再到「设置 → 数据 → MySQL 客户端」检测或选择 bin 目录。"
    } else if cfg!(target_os = "linux") {
        "未找到可用的 MySQL 客户端。请用 apt install default-mysql-client 或 dnf install mysql 安装，再到「设置 → 数据 → MySQL 客户端」检测。"
    } else {
        "未找到可用的 MySQL 客户端。请到「设置 → 数据 → MySQL 客户端」下载或指定已安装客户端目录。"
    }
}

fn candidate_dirs(settings: &Settings) -> Vec<(PathBuf, PgClientSource)> {
    let mut dirs = Vec::new();
    if !settings.mysql_client_dir.trim().is_empty() {
        let root = PathBuf::from(settings.mysql_client_dir.trim());
        dirs.push((root.join("bin"), PgClientSource::Configured));
        dirs.push((root, PgClientSource::Configured));
    }
    if let Ok(entries) = std::fs::read_dir(mysql_client_managed_root()) {
        let mut managed: Vec<_> = entries
            .flatten()
            .map(|entry| entry.path())
            .filter(|path| path.join(INSTALL_MARKER).is_file())
            .collect();
        managed.sort_by(|a, b| b.cmp(a));
        dirs.extend(
            managed
                .into_iter()
                .map(|root| (root.join("bin"), PgClientSource::Managed)),
        );
    }
    let mut system = Vec::new();
    if cfg!(target_os = "macos") {
        for root in ["/opt/homebrew/opt", "/usr/local/opt"] {
            for package in [
                "mysql-client@8.4",
                "mysql-client@8.0",
                "mysql-client",
                "mysql@8.4",
                "mysql@8.0",
                "mysql",
                "mariadb",
            ] {
                system.push(PathBuf::from(root).join(package).join("bin"));
            }
        }
        system.push(PathBuf::from("/usr/local/mysql/bin"));
    } else if cfg!(windows) {
        for root in ["ProgramFiles", "ProgramFiles(x86)"]
            .iter()
            .filter_map(std::env::var_os)
        {
            let root = PathBuf::from(root);
            if let Ok(entries) = std::fs::read_dir(root.join("MySQL")) {
                system.extend(entries.flatten().map(|entry| entry.path().join("bin")));
            }
            if let Ok(entries) = std::fs::read_dir(root) {
                system.extend(
                    entries
                        .flatten()
                        .filter(|entry| entry.file_name().to_string_lossy().starts_with("MariaDB"))
                        .map(|entry| entry.path().join("bin")),
                );
            }
        }
    } else {
        system.extend([
            PathBuf::from("/usr/bin"),
            PathBuf::from("/usr/local/mysql/bin"),
        ]);
    }
    dirs.extend(system.into_iter().map(|dir| (dir, PgClientSource::System)));
    if let Some(path) = std::env::var_os("PATH") {
        dirs.extend(std::env::split_paths(&path).map(|dir| (dir, PgClientSource::Path)));
    }
    let mut seen = BTreeSet::new();
    dirs.retain(|(dir, _)| seen.insert(dir.clone()));
    dirs
}

fn candidates(settings: &Settings, tool: MySqlClientTool) -> Vec<(PathBuf, PgClientSource)> {
    // 旧单文件配置仍最高优先；显式错误不悄悄换成另一套客户端。
    if tool == MySqlClientTool::Dump && !settings.mysqldump_path.trim().is_empty() {
        return vec![(
            PathBuf::from(settings.mysqldump_path.trim()),
            PgClientSource::Configured,
        )];
    }
    candidate_dirs(settings)
        .into_iter()
        .filter_map(|(dir, source)| tool.find_in(&dir).map(|path| (path, source)))
        .collect()
}

pub fn mysql_client_tool_present(settings: &Settings, tool: MySqlClientTool) -> bool {
    candidates(settings, tool)
        .iter()
        .any(|(path, _)| path.is_file())
}

pub fn discover_mysql_clients(settings: &Settings) -> Vec<MySqlClientLocation> {
    candidates(settings, MySqlClientTool::Dump)
        .into_iter()
        .filter_map(|(program, source)| {
            let version = mysql_client_version(&native_client_version_output(&program)?)?;
            Some(MySqlClientLocation {
                program,
                version,
                source,
            })
        })
        .collect()
}

pub fn resolve_mysql_client_tool(
    settings: &Settings,
    tool: MySqlClientTool,
) -> Option<MySqlClientLocation> {
    candidates(settings, tool)
        .into_iter()
        .find_map(|(program, source)| {
            let version = mysql_client_version(&native_client_version_output(&program)?)?;
            Some(MySqlClientLocation {
                program,
                version,
                source,
            })
        })
}

/// 官方 ZIP 保留 mysql/mysqldump 和 bin 下 DLL，避免混入服务端、测试工具与其他目录的同名库。
fn archive_target(name: &str) -> Option<PathBuf> {
    let name = name.replace('\\', "/");
    let parts: Vec<_> = name.split('/').collect();
    if parts.iter().any(|part| *part == "..") || parts.len() != 3 || parts[1] != "bin" {
        return None;
    }
    let file = parts[2];
    (matches!(file, "mysql.exe" | "mysqldump.exe") || file.ends_with(".dll"))
        .then(|| PathBuf::from("bin").join(file))
}

pub fn download_mysql_client(
    url: &str,
    install_dir: &Path,
    cancel: &AtomicBool,
    progress: &mut dyn FnMut(PgClientDownloadProgress),
) -> fluxdb_core::Result<PathBuf> {
    std::fs::create_dir_all(install_dir)
        .map_err(|error| internal_error(format!("创建客户端目录失败：{error}")))?;
    let marker = install_dir.join(INSTALL_MARKER);
    let _ = std::fs::remove_file(&marker);
    install_native_client_files(url, install_dir, &archive_target, cancel, progress)?;
    if cancel.load(Ordering::Relaxed) {
        return Err(internal_error("安装已取消".into()));
    }
    progress(PgClientDownloadProgress {
        downloaded: 0,
        total: 0,
        stage: "verify",
    });
    let bin = install_dir.join("bin");
    // 下载源必须包含成套 MySQL 工具，不能用旧残留工具掩盖缺项。
    for name in ["mysqldump.exe", "mysql.exe"] {
        let path = bin.join(name);
        if native_client_version_output(&path)
            .and_then(|text| mysql_client_version(&text))
            .is_none()
        {
            return Err(internal_error(format!(
                "MySQL 客户端安装校验失败：{} 无法执行或识别版本",
                path.display()
            )));
        }
    }
    std::fs::write(marker, MYSQL_CLIENT_VERSION)
        .map_err(|error| internal_error(format!("写入安装完成标记失败：{error}")))?;
    tracing::info!(path = %bin.display(), "MySQL 客户端安装完成");
    Ok(bin)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{SystemTime, UNIX_EPOCH};

    #[test]
    fn archive_filter_keeps_only_mysql_client_runtime() {
        assert_eq!(
            archive_target("mysql-8.4.6-winx64/bin/mysqldump.exe"),
            Some(PathBuf::from("bin/mysqldump.exe"))
        );
        assert_eq!(
            archive_target("mysql-8.4.6-winx64/bin/libssl-3-x64.dll"),
            Some(PathBuf::from("bin/libssl-3-x64.dll"))
        );
        assert_eq!(archive_target("mysql-8.4.6-winx64/bin/mysqld.exe"), None);
        assert_eq!(
            archive_target("mysql-8.4.6-winx64/../outside/mysqldump.exe"),
            None
        );
        assert_eq!(archive_target("other/bin/mysqldump.exe/extra"), None);
    }

    #[test]
    fn install_dir_accepts_root_and_bin_without_nesting_bin() {
        let mut settings = Settings::default();
        settings.mysql_client_dir = "/tmp/mysql-client/bin".into();
        assert_eq!(
            mysql_client_install_dir(&settings),
            PathBuf::from("/tmp/mysql-client")
        );
        settings.mysql_client_dir = "/tmp/mysql-client".into();
        assert_eq!(
            mysql_client_install_dir(&settings),
            PathBuf::from("/tmp/mysql-client")
        );
    }

    #[test]
    #[cfg(unix)]
    fn legacy_dump_path_stays_highest_priority_and_is_version_checked() {
        use std::os::unix::fs::PermissionsExt;

        let root = std::env::temp_dir().join(format!(
            "fluxdb-mysql-client-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&root).unwrap();
        let program = root.join("mysqldump");
        std::fs::write(
            &program,
            "#!/bin/sh\necho 'mysqldump Ver 10.19 Distrib 10.11.8-MariaDB'\n",
        )
        .unwrap();
        std::fs::set_permissions(&program, std::fs::Permissions::from_mode(0o755)).unwrap();
        let mut settings = Settings::default();
        settings.mysqldump_path = program.display().to_string();

        let resolved = resolve_mysql_client_tool(&settings, MySqlClientTool::Dump).unwrap();

        assert_eq!(resolved.program, program);
        assert!(resolved.version.mariadb);
        assert_eq!((resolved.version.major, resolved.version.minor), (10, 11));
        std::fs::remove_dir_all(root).unwrap();
    }
}
