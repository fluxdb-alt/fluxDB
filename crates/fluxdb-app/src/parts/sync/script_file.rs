//! 「导出迁移 SQL」落盘：把同步实际执行的修改语句逐条追加到本地文件。
//!
//! 只在语句执行成功后写入，因此失败/跳过的行不会出现在脚本里；写入失败按同步
//! 失败处理，避免用户拿到看似完整、实际缺语句的脚本。

use std::io::Write;
use std::path::{Path, PathBuf};

use fluxdb_core::{Error, ErrorKind};

pub struct MigrationScriptFile {
    file: std::fs::File,
    path: PathBuf,
}

impl MigrationScriptFile {
    /// 以追加方式打开（不存在则创建），每写一条立即落盘：
    /// 同步中途失败或进程退出时，已执行成功的语句仍完整可见。
    ///
    /// 只有新建的空文件才写文件头，追加到已有脚本时直接续写语句。
    pub fn create(path: &Path) -> fluxdb_core::Result<Self> {
        let file = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(path)
            .map_err(|error| script_io_error(path, error))?;
        let is_empty = file
            .metadata()
            .map(|metadata| metadata.len() == 0)
            .map_err(|error| script_io_error(path, error))?;
        let mut script = Self {
            file,
            path: path.to_path_buf(),
        };
        if is_empty {
            script.write_line("-- fluxDB 数据同步迁移脚本")?;
            script.write_line("-- 仅追加已执行成功的语句；请按原顺序在目标库执行。")?;
        }
        Ok(script)
    }

    /// 追加一条已执行成功的 SQL 语句。
    pub fn write(&mut self, sql: &str) -> fluxdb_core::Result<()> {
        self.write_line(sql)
    }

    pub fn write_line(&mut self, line: &str) -> fluxdb_core::Result<()> {
        // 每条语句写完即刷盘：追加模式下不允许把整份脚本攒在内存里。
        writeln!(self.file, "{line}")
            .and_then(|()| self.file.flush())
            .map_err(|error| script_io_error(&self.path, error))
    }

    /// 兼容旧调用点；写入已是直写，这里只做最终兜底刷新。
    pub fn finish(&mut self) -> fluxdb_core::Result<()> {
        self.file
            .flush()
            .map_err(|error| script_io_error(&self.path, error))
    }
}

fn script_io_error(path: &Path, error: std::io::Error) -> Error {
    Error::new(
        ErrorKind::Internal,
        format!("写入迁移脚本失败（{}）：{error}", path.display()),
    )
}

#[cfg(test)]
mod tests {
    use super::MigrationScriptFile;

    /// 追加模式：已有脚本续写、文件头只出现一次、每条语句立即落盘。
    #[test]
    fn appends_without_duplicating_header() {
        let path =
            std::env::temp_dir().join(format!("fluxdb-script-append-{}.sql", std::process::id()));
        let _ = std::fs::remove_file(&path);

        let mut first = MigrationScriptFile::create(&path).expect("create");
        first.write("INSERT INTO t VALUES (1);").expect("write");
        drop(first);

        let mut second = MigrationScriptFile::create(&path).expect("reopen");
        second.write("UPDATE t SET id = 2;").expect("write");
        second.finish().expect("finish");

        let content = std::fs::read_to_string(&path).expect("read");
        assert_eq!(content.matches("-- fluxDB 数据同步迁移脚本").count(), 1);
        assert!(content.contains("INSERT INTO t VALUES (1);"));
        assert!(content.contains("UPDATE t SET id = 2;"));
        let _ = std::fs::remove_file(&path);
    }
}
