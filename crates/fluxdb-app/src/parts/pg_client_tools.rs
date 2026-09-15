// PostgreSQL 原生客户端工具（pg_dump / pg_restore / psql / pg_dumpall）的发现、解析与下载安装。
//
// 作为 `fluxdb_app` 的 crate-root 子文件被 include：只处理「工具在哪里、够不够新、缺了怎么补」，
// 不负责真正执行备份/恢复（执行仍在调用方）。参考 DBeaver 的客户端管理思路：
//   1. 先在本机标准安装路径与 PATH 中自动发现客户端；
//   2. 用户可在设置里显式指定客户端 bin 目录（最高优先级）；
//   3. 本机没有时按平台下载官方二进制包（Windows / macOS），Linux 给出包管理器安装引导。
//
// 下载目标目录：设置里配置了客户端目录就装到那里，否则装到应用数据目录下的托管目录
// （`<app-data>/clients/postgresql/<版本>`），避免污染系统目录，也便于卸载。

use std::io::{Read, Write};
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};

/// 内置下载源：EnterpriseDB 官方 PostgreSQL 二进制包（无需安装，解压即用）。
/// `{version}` 为 EDB 版本号（如 `17.6-1`），`{platform}` 为 `windows-x64` / `osx`。
pub const PG_CLIENT_DEFAULT_SOURCE: &str =
    "https://get.enterprisedb.com/postgresql/postgresql-{version}-{platform}-binaries.zip";

/// 各主版本对应的 EDB 发行版本号。按服务端主版本选择，保证 pg_dump 不低于服务端。
const PG_CLIENT_VERSIONS: &[(u32, &str)] = &[
    (18, "18.1-1"),
    (17, "17.6-1"),
    (16, "16.10-1"),
    (15, "15.14-1"),
    (14, "14.19-1"),
];

/// 服务端版本未知或超出上表范围时使用的版本（取最新，向下兼容旧服务端）。
const PG_CLIENT_FALLBACK_VERSION: &str = "18.1-1";

/// 下载安装时从压缩包里保留的可执行文件（其余 initdb/postgres 等服务端程序不需要）。
const PG_CLIENT_KEPT_BINARIES: &[&str] = &["pg_dump", "pg_dumpall", "pg_restore", "psql"];

/// 安装完成标记：只有完整通过校验的托管安装目录才会有它（见 `pg_client_managed_dirs`）。
const PG_CLIENT_INSTALL_MARKER: &str = ".fluxdb-install-complete";

/// 探测 `--version` 的等待上限。客户端工具在依赖库缺失等异常情况下可能长时间不退出，
/// 无上限等待会把客户端发现和下载校验一起拖死。
const PG_CLIENT_VERSION_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(3);

/// 客户端运行时依赖库的文件名前缀白名单（精简安装只取这些）。
///
/// 来自对官方包里 `pg_dump` 的实际链接闭包：libpq → OpenSSL / krb5 / gettext / iconv，
/// 外加 pg_dump 自身用到的压缩库。ICU、libxml2、wxWidgets 等是服务端与 pgAdmin 的依赖，不在其中。
/// 名单只按前缀匹配，兼容 `libssl.3.dylib` / `libssl-3-x64.dll` / `libssl.so.3` 等各平台命名；
/// 万一某个版本的依赖超出名单，安装校验不过时会自动改用「全部动态库」重试一轮兜底。
const PG_CLIENT_RUNTIME_PREFIXES: &[&str] = &[
    "libpq",
    "libssl",
    "libcrypto",
    "libz",
    "libzstd",
    "liblz4",
    "libintl",
    "libiconv",
    "libgssapi",
    "libgssrpc",
    "libkrb5",
    "libk5crypto",
    "libcom_err",
    "libwinpthread",
    "zlib",
];

/// PostgreSQL 原生客户端工具种类。
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PgClientTool {
    /// 逻辑备份工具。
    Dump,
    /// 全库（含角色/表空间）备份工具。
    DumpAll,
    /// 自定义/目录格式备份的恢复工具。
    Restore,
    /// 交互式客户端，用于执行含 `COPY ... FROM STDIN` 或元命令的脚本。
    Psql,
}

impl PgClientTool {
    /// 工具名（不含平台后缀），用于展示与错误提示。
    pub fn display_name(self) -> &'static str {
        match self {
            PgClientTool::Dump => "pg_dump",
            PgClientTool::DumpAll => "pg_dumpall",
            PgClientTool::Restore => "pg_restore",
            PgClientTool::Psql => "psql",
        }
    }

    /// 可执行文件名，Windows 上带 `.exe`。
    pub fn binary_name(self) -> String {
        if cfg!(target_os = "windows") {
            format!("{}.exe", self.display_name())
        } else {
            self.display_name().to_string()
        }
    }
}

/// 一处可用的 PostgreSQL 客户端安装（以 bin 目录为单位）。
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PgClientLocation {
    /// 包含 pg_dump 等可执行文件的目录。
    pub bin_dir: PathBuf,
    /// `pg_dump --version` 解析出的主版本号；探测失败为 None。
    pub major_version: Option<u32>,
    /// 该位置的来源，用于设置页展示与优先级排序。
    pub source: PgClientSource,
}

/// 客户端位置来源。顺序即优先级：显式配置 > 应用下载 > 系统安装 > PATH。
#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd)]
pub enum PgClientSource {
    /// 设置里显式指定的客户端目录。
    Configured,
    /// 应用自己下载安装的托管目录。
    Managed,
    /// 本机 PostgreSQL 安装的标准路径。
    System,
    /// 依赖系统 PATH 解析（没有具体目录）。
    Path,
}

/// 解析结果：可直接交给 `Command::new` 的程序路径 + 已知的主版本号。
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PgClientToolPath {
    /// 可执行文件绝对路径，或在 PATH 兜底时为裸名（如 `pg_dump`）。
    pub program: String,
    /// 工具主版本号；未探测出来为 None（调用方不据此拦截）。
    pub major_version: Option<u32>,
    /// 来源，便于日志与设置页展示。
    pub source: PgClientSource,
}

/// 应用托管的客户端安装根目录：`<应用数据目录>/clients/postgresql`。
pub fn pg_client_managed_root() -> PathBuf {
    fluxdb_storage::FileStorage::default_root()
        .unwrap_or_else(|_| PathBuf::from(".fluxdb"))
        .join("clients")
        .join("postgresql")
}

/// 下载安装目录：设置里配置了客户端目录就用它，否则用托管目录下的版本子目录。
/// 与用户诉求一致——「下载到设置的路径，没有设置就下载到默认安装路径」。
pub fn pg_client_install_dir(settings: &Settings, version: &str) -> PathBuf {
    let configured = settings.pg_client_dir.trim();
    if !configured.is_empty() {
        return PathBuf::from(configured);
    }
    pg_client_managed_root().join(version)
}

/// 当前平台在下载源模板里的标识；Linux 没有官方免安装二进制包，返回 None（改走安装引导）。
pub fn pg_client_platform_slug() -> Option<&'static str> {
    if cfg!(target_os = "windows") {
        Some("windows-x64")
    } else if cfg!(target_os = "macos") {
        Some("osx")
    } else {
        None
    }
}

/// 当前平台是否支持应用内下载客户端工具。
pub fn pg_client_download_supported() -> bool {
    pg_client_platform_slug().is_some()
}

/// 按服务端主版本选择要下载的客户端版本号；未知或超表范围时取内置最新版本。
pub fn pg_client_version_for_server(server_major: Option<u32>) -> &'static str {
    server_major
        .and_then(|major| {
            PG_CLIENT_VERSIONS
                .iter()
                .find(|(table_major, _)| *table_major == major)
                .map(|(_, version)| *version)
        })
        .unwrap_or(PG_CLIENT_FALLBACK_VERSION)
}

/// 构造下载 URL。`source` 为空时使用内置 EDB 源；模板不含占位符时按直链原样使用。
/// 返回 None 表示当前平台没有可下载的官方包（Linux）。
pub fn pg_client_download_url(source: &str, server_major: Option<u32>) -> Option<String> {
    let platform = pg_client_platform_slug()?;
    let template = {
        let trimmed = source.trim();
        if trimmed.is_empty() {
            PG_CLIENT_DEFAULT_SOURCE
        } else {
            trimmed
        }
    };
    let version = pg_client_version_for_server(server_major);
    Some(
        template
            .replace("{version}", version)
            .replace("{platform}", platform),
    )
}

/// 缺少客户端工具时的安装引导文案（按平台给出可执行的下一步）。
pub fn pg_client_install_hint() -> &'static str {
    if cfg!(target_os = "linux") {
        "未找到 PostgreSQL 客户端工具。请用包管理器安装（Debian/Ubuntu：sudo apt install postgresql-client；\
         RHEL/Fedora：sudo dnf install postgresql），或在「设置 → 数据 → 备份」中指定客户端目录。"
    } else if cfg!(target_os = "macos") {
        "未找到 PostgreSQL 客户端工具。可在「设置 → 数据 → 备份」中一键下载官方客户端，\
         也可先执行 brew install libpq 再在设置中指定其 bin 目录。"
    } else {
        "未找到 PostgreSQL 客户端工具。可在「设置 → 数据 → 备份」中一键下载官方客户端，\
         也可安装 PostgreSQL 后在设置中指定其 bin 目录。"
    }
}

/// 候选 bin 目录（按优先级去重排序）：设置目录 → 托管下载目录 → 系统标准安装路径。
pub fn pg_client_candidate_dirs(settings: &Settings) -> Vec<(PathBuf, PgClientSource)> {
    let mut dirs: Vec<(PathBuf, PgClientSource)> = Vec::new();
    let configured = settings.pg_client_dir.trim();
    if !configured.is_empty() {
        let base = PathBuf::from(configured);
        // 用户可能选的是安装根目录（含 bin 子目录），也可能直接选的 bin 目录，两种都接受。
        dirs.push((base.join("bin"), PgClientSource::Configured));
        dirs.push((base, PgClientSource::Configured));
    }
    for dir in pg_client_managed_dirs() {
        dirs.push((dir, PgClientSource::Managed));
    }
    for dir in pg_client_system_dirs() {
        dirs.push((dir, PgClientSource::System));
    }
    let mut seen: BTreeSet<PathBuf> = BTreeSet::new();
    dirs.retain(|(dir, _)| seen.insert(dir.clone()));
    dirs
}

/// 托管目录下已安装的版本（新版本在前）。
///
/// 只认带完成标记的目录：安装是「边下边写」，中途被杀会留下有 pg_dump 却缺依赖库的半成品，
/// 这种目录若被当成可用客户端，备份时才会以 dyld 报错暴露，所以宁可不列出来。
fn pg_client_managed_dirs() -> Vec<PathBuf> {
    pg_client_managed_dirs_in(&pg_client_managed_root())
}

/// `pg_client_managed_dirs` 的可注入版本（根目录作为参数，便于单测）。
fn pg_client_managed_dirs_in(root: &Path) -> Vec<PathBuf> {
    let Ok(entries) = std::fs::read_dir(root) else {
        return Vec::new();
    };
    let mut versions: Vec<PathBuf> = entries
        .flatten()
        .map(|entry| entry.path())
        .filter(|path| path.is_dir() && path.join(PG_CLIENT_INSTALL_MARKER).is_file())
        .collect();
    // 目录名即版本号（如 17.6-1），按字符串倒序近似「新版本优先」。
    versions.sort_by(|left, right| right.file_name().cmp(&left.file_name()));
    versions.into_iter().map(|path| path.join("bin")).collect()
}

/// 本机 PostgreSQL 客户端的标准安装路径（含按版本展开的多版本目录）与 PATH 中的目录。
fn pg_client_system_dirs() -> Vec<PathBuf> {
    let mut dirs: Vec<PathBuf> = Vec::new();
    if cfg!(target_os = "macos") {
        dirs.extend(pg_client_versioned_dirs("/Applications/Postgres.app/Contents/Versions"));
        dirs.extend(pg_client_versioned_dirs("/Library/PostgreSQL"));
        dirs.push(PathBuf::from("/opt/homebrew/opt/libpq/bin"));
        dirs.push(PathBuf::from("/usr/local/opt/libpq/bin"));
        dirs.push(PathBuf::from("/opt/homebrew/bin"));
        dirs.push(PathBuf::from("/usr/local/bin"));
    } else if cfg!(target_os = "linux") {
        dirs.extend(pg_client_versioned_dirs("/usr/lib/postgresql"));
        dirs.push(PathBuf::from("/usr/bin"));
        dirs.push(PathBuf::from("/usr/local/bin"));
    } else {
        for base in ["ProgramFiles", "ProgramFiles(x86)"] {
            if let Some(root) = std::env::var_os(base) {
                let mut root = PathBuf::from(root);
                root.push("PostgreSQL");
                dirs.extend(pg_client_versioned_dirs(root));
            }
        }
    }
    if let Some(path) = std::env::var_os("PATH") {
        dirs.extend(std::env::split_paths(&path));
    }
    dirs
}

/// 展开「按版本分目录」的安装根（如 `/usr/lib/postgresql/16/bin`），新版本排前面。
fn pg_client_versioned_dirs(root: impl AsRef<Path>) -> Vec<PathBuf> {
    let Ok(entries) = std::fs::read_dir(root.as_ref()) else {
        return Vec::new();
    };
    let mut versions: Vec<PathBuf> = entries
        .flatten()
        .map(|entry| entry.path())
        .filter(|path| path.is_dir())
        .collect();
    versions.sort_by(|left, right| right.file_name().cmp(&left.file_name()));
    versions.into_iter().map(|path| path.join("bin")).collect()
}

/// 扫描所有候选目录，返回实际存在 pg_dump 的客户端位置（已探测主版本号）。
pub fn discover_pg_clients(settings: &Settings) -> Vec<PgClientLocation> {
    let binary = PgClientTool::Dump.binary_name();
    pg_client_candidate_dirs(settings)
        .into_iter()
        .filter(|(dir, _)| dir.join(&binary).is_file())
        .map(|(dir, source)| {
            let program = dir.join(&binary);
            PgClientLocation {
                major_version: pg_tool_major_at(&program),
                bin_dir: dir,
                source,
            }
        })
        .collect()
}

/// 工具是否存在（只查文件，不执行 `--version`），供打开对话框等 UI 路径做快速预检，
/// 避免在渲染/交互路径上拉起子进程。真正执行前仍会走 `resolve_pg_client_tool` 拿具体路径。
pub fn pg_client_tool_present(settings: &Settings, tool: PgClientTool) -> bool {
    if tool == PgClientTool::Dump && !settings.pg_dump_path.trim().is_empty() {
        return Path::new(settings.pg_dump_path.trim()).is_file();
    }
    let binary = tool.binary_name();
    pg_client_candidate_dirs(settings)
        .into_iter()
        .any(|(dir, _)| dir.join(&binary).is_file())
}

/// 解析某个客户端工具的可执行路径。
///
/// 优先级：`pg_dump_path`（旧设置，仅 pg_dump 生效，保持兼容） → 设置的客户端目录 → 托管下载目录
/// → 系统安装路径 → PATH 裸名兜底。`required_major` 非空时优先挑选不低于该主版本的安装
/// （PostgreSQL 禁止用更旧的 pg_dump 备份更新的服务端），都不满足时退回版本最高的一处，
/// 由调用方按版本校验给出明确错误，而不是在这里直接失败。
pub fn resolve_pg_client_tool(
    settings: &Settings,
    tool: PgClientTool,
    required_major: Option<u32>,
) -> Option<PgClientToolPath> {
    if tool == PgClientTool::Dump {
        let legacy = settings.pg_dump_path.trim();
        if !legacy.is_empty() {
            return Some(PgClientToolPath {
                major_version: pg_tool_major_at(Path::new(legacy)),
                program: legacy.to_string(),
                source: PgClientSource::Configured,
            });
        }
    }
    let binary = tool.binary_name();
    let mut candidates: Vec<PgClientToolPath> = pg_client_candidate_dirs(settings)
        .into_iter()
        .filter(|(dir, _)| dir.join(&binary).is_file())
        .map(|(dir, source)| {
            let program = dir.join(&binary);
            PgClientToolPath {
                major_version: pg_tool_major_at(&program),
                program: program.display().to_string(),
                source,
            }
        })
        .collect();
    if let Some(required) = required_major
        && let Some(index) = candidates
            .iter()
            .position(|candidate| candidate.major_version.is_some_and(|major| major >= required))
    {
        return Some(candidates.remove(index));
    }
    if !candidates.is_empty() {
        // 没有满足版本要求的：挑主版本最高的一处，版本未知视为 0 排最后。
        candidates.sort_by_key(|candidate| std::cmp::Reverse(candidate.major_version.unwrap_or(0)));
        return Some(candidates.remove(0));
    }
    // 最后兜底：PATH 里可能有同名工具（如通过 shell 环境注入的路径）。
    let major = pg_tool_major_at(Path::new(&binary))?;
    Some(PgClientToolPath {
        program: binary,
        major_version: Some(major),
        source: PgClientSource::Path,
    })
}

/// 运行 `<program> --version` 并解析主版本号；无法执行、超时或无法解析返回 None。
///
/// 带超时：依赖库缺失等异常的工具会卡住不退出（实测 macOS 上 dyld 报错后进程仍不回收），
/// 无上限等待会让调用方（客户端发现、下载后校验）一起挂死，必须主动 kill 回收。
pub fn pg_tool_major_at(program: &Path) -> Option<u32> {
    let mut child = std::process::Command::new(program)
        .arg("--version")
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .ok()?;
    let deadline = std::time::Instant::now() + PG_CLIENT_VERSION_TIMEOUT;
    loop {
        match child.try_wait() {
            Ok(Some(_)) => break,
            Ok(None) if std::time::Instant::now() < deadline => {
                std::thread::sleep(std::time::Duration::from_millis(50));
            }
            _ => {
                let _ = child.kill();
                let _ = child.wait();
                tracing::warn!(
                    program = %program.display(),
                    "客户端工具 --version 超时未退出，已终止并忽略该安装"
                );
                return None;
            }
        }
    }
    let output = child.wait_with_output().ok()?;
    if !output.status.success() {
        return None;
    }
    pg_tool_major_version(&String::from_utf8_lossy(&output.stdout))
}

/// 下载过程回调，用于把进度透传到 UI。
#[derive(Clone, Copy, Debug)]
pub struct PgClientDownloadProgress {
    /// 已下载字节数。
    pub downloaded: u64,
    /// 总字节数；服务端未给 Content-Length 时为 0。
    pub total: u64,
    /// 当前阶段：`download` / `extract` / `verify`。
    pub stage: &'static str,
}

/// 下载并安装 PostgreSQL 客户端工具，返回安装后的 bin 目录。
///
/// 官方二进制包是完整服务端发行版（Windows 约 330MB、macOS 约 359MB），其中 97% 是备份/恢复用不到的
/// 服务端程序、share 数据与 pgAdmin 依赖。因此优先走 **分片下载**：用 HTTP Range 把压缩包当作可随机
/// 访问的文件读取，只抓取客户端工具与其依赖库的字节范围（实测降到数 MB）。服务端不支持 Range 时
/// 回退成整包下载，保证可用性。
///
/// 安装后必须能跑通 `pg_dump --version`；若精简文件集缺了依赖，会自动带上被跳过的可选库（ICU 等）
/// 重试一次，仍失败才报错，避免留下一个跑不起来的半成品目录。
pub fn download_pg_client(
    url: &str,
    install_dir: &Path,
    cancel: &AtomicBool,
    progress: &mut dyn FnMut(PgClientDownloadProgress),
) -> fluxdb_core::Result<PathBuf> {
    std::fs::create_dir_all(install_dir).map_err(|error| {
        internal_error(format!(
            "创建客户端安装目录失败：{}：{error}",
            install_dir.display()
        ))
    })?;
    let bin_dir = install_dir.join("bin");
    let dump = bin_dir.join(PgClientTool::Dump.binary_name());
    let psql = bin_dir.join(PgClientTool::Psql.binary_name());

    // 开始写入前先撤掉完成标记：中途被杀（或校验失败）时留下的半成品目录不能被当成可用安装。
    let marker = install_dir.join(PG_CLIENT_INSTALL_MARKER);
    let _ = std::fs::remove_file(&marker);

    // 第一轮只取精简文件集；校验不过再带上全部动态库与可选库重试一轮。
    // 备份（pg_dump）与原生脚本/恢复（psql）都必须真的能跑起来才算安装成功。
    for include_optional in [false, true] {
        install_pg_client_files(url, install_dir, include_optional, cancel, progress)?;
        progress(PgClientDownloadProgress {
            downloaded: 0,
            total: 0,
            stage: "verify",
        });
        let dump_major = pg_tool_major_at(&dump);
        if dump_major.is_some() && pg_tool_major_at(&psql).is_some() {
            // 校验通过才落完成标记（写入内容即本次安装到的客户端主版本，便于排查）。
            let version = dump_major.unwrap_or_default().to_string();
            if let Err(error) = std::fs::write(&marker, version) {
                tracing::warn!(?error, "写入客户端安装完成标记失败");
            }
            tracing::info!(
                target = %bin_dir.display(),
                include_optional,
                "PostgreSQL 客户端工具下载安装完成"
            );
            return Ok(bin_dir);
        }
        if !include_optional {
            tracing::warn!("精简文件集校验未通过，补取全部依赖库后重试");
        }
    }
    Err(internal_error(format!(
        "客户端安装校验失败：{} 或 {} 无法执行，请确认下载源与平台匹配",
        dump.display(),
        psql.display()
    )))
}

/// 取回并解压一轮客户端文件：优先分片下载，Range 不可用时回退整包下载。
fn install_pg_client_files(
    url: &str,
    install_dir: &Path,
    include_optional: bool,
    cancel: &AtomicBool,
    progress: &mut dyn FnMut(PgClientDownloadProgress),
) -> fluxdb_core::Result<()> {
    match HttpRangeReader::open(url, cancel) {
        Ok(reader) => {
            let fetched = reader.fetched.clone();
            tracing::info!(url, "按需分片下载 PostgreSQL 客户端");
            return extract_pg_client_archive(
                std::io::BufReader::new(reader),
                install_dir,
                include_optional,
                cancel,
                &mut |stage, index, total| {
                    progress(PgClientDownloadProgress {
                        // 分片模式下总量事先未知，按已取字节数展示。
                        downloaded: if stage == "extract" {
                            fetched.load(Ordering::Relaxed)
                        } else {
                            index
                        },
                        total: if stage == "extract" { 0 } else { total },
                        stage,
                    });
                },
            );
        }
        Err(RangeOpenError::Unsupported) => {
            tracing::info!(url, "下载源不支持 Range，回退整包下载");
        }
        Err(RangeOpenError::Failed(error)) => return Err(error),
    }

    let archive_path = install_dir.join(".pg-client-download.part");
    let download_result = download_to_file(url, &archive_path, cancel, progress);
    if let Err(error) = download_result {
        let _ = std::fs::remove_file(&archive_path);
        return Err(error);
    }
    let file = std::fs::File::open(&archive_path)
        .map_err(|error| internal_error(format!("打开下载文件失败：{error}")));
    let extract_result = match file {
        Ok(file) => extract_pg_client_archive(
            std::io::BufReader::new(file),
            install_dir,
            include_optional,
            cancel,
            &mut |stage, index, total| {
                progress(PgClientDownloadProgress {
                    downloaded: index,
                    total,
                    stage,
                });
            },
        ),
        Err(error) => Err(error),
    };
    let _ = std::fs::remove_file(&archive_path);
    extract_result
}

/// 流式下载整包到本地文件，按 256KB 分片回报进度并检查取消（Range 不可用时的回退路径）。
fn download_to_file(
    url: &str,
    target: &Path,
    cancel: &AtomicBool,
    progress: &mut dyn FnMut(PgClientDownloadProgress),
) -> fluxdb_core::Result<()> {
    tracing::info!(url, "开始整包下载 PostgreSQL 客户端工具");
    let response = ureq::get(url)
        .call()
        .map_err(|error| internal_error(format!("下载客户端失败：{error}")))?;
    let total = content_length(response.headers());
    let mut reader = response.into_body().into_reader();
    let mut file = std::fs::File::create(target)
        .map_err(|error| internal_error(format!("创建下载临时文件失败：{error}")))?;
    let mut buffer = vec![0u8; 256 * 1024];
    let mut downloaded = 0u64;
    loop {
        if cancel.load(Ordering::Relaxed) {
            return Err(internal_error("下载已取消".to_string()));
        }
        let read = reader
            .read(&mut buffer)
            .map_err(|error| internal_error(format!("下载客户端失败：{error}")))?;
        if read == 0 {
            break;
        }
        file.write_all(&buffer[..read])
            .map_err(|error| internal_error(format!("写入下载文件失败：{error}")))?;
        downloaded += read as u64;
        progress(PgClientDownloadProgress {
            downloaded,
            total,
            stage: "download",
        });
    }
    file.flush()
        .map_err(|error| internal_error(format!("写入下载文件失败：{error}")))?;
    Ok(())
}

/// 解压压缩包中需要的条目（客户端可执行文件 + 运行库），其余（服务端程序、头文件、文档）跳过。
///
/// 读取源既可以是本地文件，也可以是 `HttpRangeReader`（远端按需取字节）；zip 的中央目录解析与
/// 条目定位都走 `Seek`，因此分片模式下只有被选中的条目会真正产生网络流量。
fn extract_pg_client_archive<R: Read + std::io::Seek>(
    reader: R,
    install_dir: &Path,
    include_optional: bool,
    cancel: &AtomicBool,
    progress: &mut dyn FnMut(&'static str, u64, u64),
) -> fluxdb_core::Result<()> {
    let mut archive = zip::ZipArchive::new(reader)
        .map_err(|error| internal_error(format!("解析客户端压缩包失败：{error}")))?;
    // 先按中央目录挑出要解压的条目，避免对整包上万个条目逐个做 IO。
    let wanted: Vec<(usize, PathBuf)> = (0..archive.len())
        .filter_map(|index| {
            let name = archive.name_for_index(index)?;
            pg_client_archive_target(name, include_optional).map(|target| (index, target))
        })
        .collect();
    let total = wanted.len() as u64;
    for (position, (index, relative)) in wanted.into_iter().enumerate() {
        if cancel.load(Ordering::Relaxed) {
            return Err(internal_error("安装已取消".to_string()));
        }
        progress("extract", position as u64, total);
        let mut entry = archive
            .by_index(index)
            .map_err(|error| internal_error(format!("读取压缩包条目失败：{error}")))?;
        if entry.is_dir() {
            continue;
        }
        let target = install_dir.join(&relative);
        if let Some(parent) = target.parent() {
            std::fs::create_dir_all(parent)
                .map_err(|error| internal_error(format!("创建目录失败：{error}")))?;
        }
        let mode = entry.unix_mode();
        if is_symlink_mode(mode) {
            write_symlink_entry(&mut entry, &target)?;
            continue;
        }
        let mut output = std::fs::File::create(&target)
            .map_err(|error| internal_error(format!("写入 {} 失败：{error}", target.display())))?;
        std::io::copy(&mut entry, &mut output)
            .map_err(|error| internal_error(format!("解压 {} 失败：{error}", target.display())))?;
        apply_executable_mode(&target, mode);
    }
    Ok(())
}

/// 压缩包条目 → 安装目录下的相对路径；只保留客户端可执行文件与动态库，其余返回 None。
///
/// 只看路径的最后两段（`bin/<name>`、`lib/<name>`），避免依赖压缩包顶层目录名。
/// `include_optional=false` 时跳过体积大且客户端用不到的可选库（ICU 数据、pgAdmin 的 wxWidgets）：
/// 实测 macOS 的 `libicudata` 占 23MB、Windows 的 `icudt*.dll` 占 11MB，都是服务端排序规则/GUI 依赖。
/// 校验不过时调用方会以 `include_optional=true` 再来一轮，保证不会因为过度精简而装出跑不起来的工具。
fn pg_client_archive_target(entry_name: &str, include_optional: bool) -> Option<PathBuf> {
    let normalized = entry_name.replace('\\', "/");
    if normalized.contains("..") {
        // 防御 zip slip：带上跳路径的条目一律丢弃。
        return None;
    }
    let mut parts = normalized.rsplit('/');
    let name = parts.next()?;
    let parent = parts.next()?;
    if name.is_empty() {
        return None;
    }
    let optional = pg_client_optional_library(name);
    match parent {
        "bin" => {
            let stem = name.strip_suffix(".exe").unwrap_or(name);
            if PG_CLIENT_KEPT_BINARIES.contains(&stem) {
                return Some(PathBuf::from("bin").join(name));
            }
            // Windows 的运行库与可执行文件同在 bin/。
            let keep = name.ends_with(".dll")
                && (include_optional || (!optional && pg_client_runtime_library(name)));
            keep.then(|| PathBuf::from("bin").join(name))
        }
        // 非 Windows 上客户端依赖 `../lib` 里的动态库（macOS 的 libpq/libssl 等）。
        "lib" if !cfg!(target_os = "windows") => {
            let dynamic = name.contains(".dylib") || name.contains(".so");
            let keep = dynamic
                && (include_optional || (!optional && pg_client_runtime_library(name)));
            keep.then(|| PathBuf::from("lib").join(name))
        }
        _ => None,
    }
}

/// 是否属于客户端运行时依赖白名单（精简安装只取这些动态库）。
fn pg_client_runtime_library(name: &str) -> bool {
    let lower = name.to_ascii_lowercase();
    PG_CLIENT_RUNTIME_PREFIXES
        .iter()
        .any(|prefix| lower.starts_with(prefix))
}

/// 是否属于「默认跳过」的可选库：ICU（服务端排序规则数据）与 wxWidgets（pgAdmin GUI）。
fn pg_client_optional_library(name: &str) -> bool {
    let lower = name.to_ascii_lowercase();
    lower.starts_with("icu")
        || lower.starts_with("libicu")
        || lower.starts_with("wx")
        || lower.starts_with("libwx")
}

/// zip 条目是否为符号链接（macOS 包里 `libpq.dylib -> libpq.5.dylib` 这类）。
fn is_symlink_mode(mode: Option<u32>) -> bool {
    !cfg!(target_os = "windows") && mode.is_some_and(|mode| mode & 0xF000 == 0xA000)
}

/// 还原符号链接条目：条目内容即链接目标路径。
fn write_symlink_entry(entry: &mut impl Read, target: &Path) -> fluxdb_core::Result<()> {
    let mut link = String::new();
    entry
        .read_to_string(&mut link)
        .map_err(|error| internal_error(format!("读取符号链接条目失败：{error}")))?;
    let _ = std::fs::remove_file(target);
    #[cfg(unix)]
    std::os::unix::fs::symlink(&link, target)
        .map_err(|error| internal_error(format!("创建符号链接失败：{error}")))?;
    Ok(())
}

/// 还原可执行权限（zip 里的 unix mode 丢失会导致解压出来的 pg_dump 不可执行）。
fn apply_executable_mode(target: &Path, mode: Option<u32>) {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = mode.unwrap_or(0o755) & 0o777;
        let mode = if mode == 0 { 0o755 } else { mode };
        if let Err(error) = std::fs::set_permissions(target, std::fs::Permissions::from_mode(mode)) {
            tracing::warn!(?error, path = %target.display(), "设置客户端文件权限失败");
        }
    }
    #[cfg(not(unix))]
    {
        let _ = (target, mode);
    }
}

/// 分片读取的起始块大小：小块避免为几 KB 的条目多取无用字节。
const PG_CLIENT_RANGE_CHUNK: u64 = 256 * 1024;

/// 顺序读取时块大小逐次翻倍的上限：中央目录和大动态库都是顺序读，翻倍能把请求数压下来。
const PG_CLIENT_RANGE_CHUNK_MAX: u64 = 1024 * 1024;

/// 打开分片读取失败的原因：`Unsupported` 表示服务端不认 Range，调用方回退整包下载。
enum RangeOpenError {
    /// 服务端未返回 206 / 未给出总长度。
    Unsupported,
    /// 网络或协议错误，不应静默回退。
    Failed(fluxdb_core::Error),
}

/// 把远端 HTTP 资源包装成可 `Read + Seek` 的字节流：按 1MB 块发 Range 请求并缓存当前块。
///
/// 让 zip 直接在远端压缩包上做随机访问（中央目录 + 目标条目定位），从而只下载需要的条目，
/// 而不是先拉完 330MB 再丢掉 97%。缓存只保留最近一块：zip 的访问是「中央目录顺序扫 + 条目顺序读」，
/// 单块缓存已经足够，不需要为此维护多块 LRU。
struct HttpRangeReader<'a> {
    agent: ureq::Agent,
    url: String,
    /// 资源总长度（来自首个 Range 响应的 Content-Range）。
    total: u64,
    /// 当前读取位置。
    position: u64,
    /// 已缓存块的起始偏移与内容。
    chunk_start: u64,
    chunk: Vec<u8>,
    /// 下一次取块的大小：顺序读时翻倍（上限 `PG_CLIENT_RANGE_CHUNK_MAX`），跳读时复位。
    chunk_size: u64,
    /// 实际从网络取到的字节数，供 UI 展示真实下载量。
    fetched: Arc<std::sync::atomic::AtomicU64>,
    cancel: &'a AtomicBool,
}

impl<'a> HttpRangeReader<'a> {
    /// 探测 Range 支持并取得资源总长度；服务端返回 200（忽略 Range）时判定为不支持。
    fn open(url: &str, cancel: &'a AtomicBool) -> Result<Self, RangeOpenError> {
        let agent = ureq::agent();
        let response = agent
            .get(url)
            .header("Range", "bytes=0-0")
            .call()
            .map_err(|error| {
                RangeOpenError::Failed(internal_error(format!("下载客户端失败：{error}")))
            })?;
        if response.status() != 206 {
            return Err(RangeOpenError::Unsupported);
        }
        let Some(total) = content_range_total(response.headers()) else {
            return Err(RangeOpenError::Unsupported);
        };
        Ok(Self {
            agent,
            url: url.to_string(),
            total,
            position: 0,
            chunk_start: 0,
            chunk: Vec::new(),
            chunk_size: PG_CLIENT_RANGE_CHUNK,
            fetched: Arc::new(std::sync::atomic::AtomicU64::new(0)),
            cancel,
        })
    }

    /// 确保缓存块覆盖当前位置，必要时发一次 Range 请求。
    fn ensure_chunk(&mut self) -> std::io::Result<()> {
        let covered = self.position >= self.chunk_start
            && self.position < self.chunk_start + self.chunk.len() as u64;
        if covered {
            return Ok(());
        }
        if self.cancel.load(Ordering::Relaxed) {
            return Err(std::io::Error::other("下载已取消"));
        }
        // 顺序读（上一块的紧邻位置）说明还在扫中央目录或读大文件，块翻倍摊薄请求数；
        // 跳读说明是定位新条目，复位成小块，避免为小条目多取字节。
        let start = self.position;
        let sequential = start == self.chunk_start + self.chunk.len() as u64;
        self.chunk_size = if sequential {
            (self.chunk_size * 2).min(PG_CLIENT_RANGE_CHUNK_MAX)
        } else {
            PG_CLIENT_RANGE_CHUNK
        };
        let end = (start + self.chunk_size - 1).min(self.total.saturating_sub(1));
        let response = self
            .agent
            .get(&self.url)
            .header("Range", format!("bytes={start}-{end}"))
            .call()
            .map_err(|error| std::io::Error::other(format!("分片下载失败：{error}")))?;
        if response.status() != 206 {
            return Err(std::io::Error::other("分片下载失败：服务端未返回 206"));
        }
        let body = response
            .into_body()
            .read_to_vec()
            .map_err(|error| std::io::Error::other(format!("分片下载失败：{error}")))?;
        self.fetched
            .fetch_add(body.len() as u64, Ordering::Relaxed);
        self.chunk_start = start;
        self.chunk = body;
        Ok(())
    }
}

impl Read for HttpRangeReader<'_> {
    fn read(&mut self, buffer: &mut [u8]) -> std::io::Result<usize> {
        if buffer.is_empty() || self.position >= self.total {
            return Ok(0);
        }
        self.ensure_chunk()?;
        let offset = (self.position - self.chunk_start) as usize;
        let available = self.chunk.len().saturating_sub(offset);
        if available == 0 {
            return Ok(0);
        }
        let take = available.min(buffer.len());
        buffer[..take].copy_from_slice(&self.chunk[offset..offset + take]);
        self.position += take as u64;
        Ok(take)
    }
}

impl std::io::Seek for HttpRangeReader<'_> {
    fn seek(&mut self, position: std::io::SeekFrom) -> std::io::Result<u64> {
        let target = match position {
            std::io::SeekFrom::Start(offset) => offset as i128,
            std::io::SeekFrom::End(offset) => self.total as i128 + offset as i128,
            std::io::SeekFrom::Current(offset) => self.position as i128 + offset as i128,
        };
        if target < 0 {
            return Err(std::io::Error::other("seek 到负偏移"));
        }
        self.position = target as u64;
        Ok(self.position)
    }
}

/// 从 `Content-Range: bytes 0-0/359123456` 解析资源总长度。
fn content_range_total(headers: &ureq::http::HeaderMap) -> Option<u64> {
    headers
        .get("content-range")?
        .to_str()
        .ok()?
        .rsplit('/')
        .next()?
        .trim()
        .parse()
        .ok()
}

/// 从响应头取 Content-Length；缺失时返回 0（UI 展示为「不确定进度」）。
fn content_length(headers: &ureq::http::HeaderMap) -> u64 {
    headers
        .get("content-length")
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.parse().ok())
        .unwrap_or(0)
}

/// 统一构造内部错误，避免每处重复写 ErrorKind。
fn internal_error(message: String) -> fluxdb_core::Error {
    fluxdb_core::Error::new(fluxdb_core::ErrorKind::Internal, message)
}
