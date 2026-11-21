// MongoDB 原生备份工具（mongodump / mongorestore）的发现与版本探测。
//
// 只处理「工具在哪里」，不负责执行；MongoDB 没有官方免安装二进制包的一键下载，
// 缺失时给出安装引导，不做半成品逻辑备份。

/// MongoDB 数据库工具种类。
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum MongoClientTool {
    Dump,
    Restore,
}

impl MongoClientTool {
    /// 工具名（不含平台后缀），用于展示与错误提示。
    pub fn display_name(self) -> &'static str {
        match self {
            Self::Dump => "mongodump",
            Self::Restore => "mongorestore",
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

/// 缺少 MongoDB 工具时的安装引导（按平台给出可执行的下一步）。
pub fn mongo_client_install_hint() -> &'static str {
    if cfg!(target_os = "macos") {
        "未找到 MongoDB 数据库工具。请执行 brew install mongodb-database-tools，\
         或用 MongoDB 官方 Database Tools 安装包安装后确保 mongodump/mongorestore 在 PATH 中。"
    } else if cfg!(target_os = "windows") {
        "未找到 MongoDB 数据库工具。请安装 MongoDB Database Tools，\
         并把其 bin 目录（通常为 C:\\Program Files\\MongoDB\\Tools\\<版本>\\bin）加入 PATH。"
    } else {
        "未找到 MongoDB 数据库工具。请安装 mongodb-database-tools 发行包，\
         并确保 mongodump/mongorestore 在 PATH 中。"
    }
}

/// 候选目录：PATH + 常见安装位置。MongoDB 工具没有设置项，发现顺序只按目录优先级。
fn mongo_client_candidate_dirs() -> Vec<PathBuf> {
    let mut dirs: Vec<PathBuf> = Vec::new();
    if cfg!(target_os = "macos") {
        dirs.extend([
            PathBuf::from("/opt/homebrew/bin"),
            PathBuf::from("/usr/local/bin"),
        ]);
    } else if cfg!(target_os = "windows") {
        for base in ["ProgramFiles", "ProgramFiles(x86)"] {
            let Some(root) = std::env::var_os(base) else {
                continue;
            };
            let root = PathBuf::from(root);
            // Database Tools 安装器按版本建目录（Tools\<版本>\bin）；旧版随服务器安装。
            if let Ok(entries) = std::fs::read_dir(root.join("MongoDB").join("Tools")) {
                dirs.extend(
                    entries
                        .flatten()
                        .map(|entry| entry.path().join("bin"))
                        .collect::<Vec<_>>(),
                );
            }
            if let Ok(entries) = std::fs::read_dir(root.join("MongoDB").join("Server")) {
                dirs.extend(
                    entries
                        .flatten()
                        .map(|entry| entry.path().join("bin"))
                        .collect::<Vec<_>>(),
                );
            }
        }
    } else {
        dirs.extend([
            PathBuf::from("/usr/bin"),
            PathBuf::from("/usr/local/bin"),
            PathBuf::from("/opt/mongodb-database-tools/bin"),
        ]);
    }
    if let Some(path) = std::env::var_os("PATH") {
        dirs.extend(std::env::split_paths(&path));
    }
    dirs
}

/// 解析工具可执行路径；找不到时返回 None，由调用方给出安装引导。
pub fn resolve_mongo_client_tool(tool: MongoClientTool) -> Option<PathBuf> {
    let binary = tool.binary_name();
    if let Some(dir) = mongo_client_candidate_dirs()
        .into_iter()
        .find(|dir| dir.join(&binary).is_file())
    {
        return Some(dir.join(binary));
    }
    // 兜底：PATH 中可能有同名工具，仅当 `--version` 可执行时才算可用。
    let program = PathBuf::from(&binary);
    native_client_version_output(&program).map(|_| program)
}

/// 读 `--version` 首行作为可展示的版本；探测失败返回 None，不阻断备份。
pub fn mongo_tool_version(program: &Path) -> Option<String> {
    native_client_version_output(program)?
        .lines()
        .next()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .map(str::to_string)
}
