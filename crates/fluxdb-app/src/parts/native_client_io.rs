use std::io::Write;
// 原生客户端共享的限时探测、HTTP 分片下载与安全解压。数据库仅提供文件筛选规则。

const NATIVE_CLIENT_VERSION_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(3);

fn native_client_version_output(program: &Path) -> Option<String> {
    let mut child = std::process::Command::new(program)
        .arg("--version")
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .ok()?;
    let deadline = std::time::Instant::now() + NATIVE_CLIENT_VERSION_TIMEOUT;
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
    let bytes = if output.stdout.is_empty() {
        &output.stderr
    } else {
        &output.stdout
    };
    let text = String::from_utf8_lossy(bytes).trim().to_string();
    (!text.is_empty()).then_some(text)
}

/// 取回并解压一轮客户端文件：优先分片下载，Range 不可用时回退整包下载。
fn install_native_client_files(
    url: &str,
    install_dir: &Path,
    select_entry: &dyn Fn(&str) -> Option<PathBuf>,
    cancel: &AtomicBool,
    progress: &mut dyn FnMut(PgClientDownloadProgress),
) -> fluxdb_core::Result<()> {
    match HttpRangeReader::open(url, cancel) {
        Ok(reader) => {
            let fetched = reader.fetched.clone();
            tracing::info!(url, "按需分片下载 数据库客户端");
            return extract_native_client_archive(
                std::io::BufReader::new(reader),
                install_dir,
                select_entry,
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
        Ok(file) => extract_native_client_archive(
            std::io::BufReader::new(file),
            install_dir,
            select_entry,
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
    tracing::info!(url, "开始整包下载 数据库客户端工具");
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
fn extract_native_client_archive<R: Read + std::io::Seek>(
    reader: R,
    install_dir: &Path,
    select_entry: &dyn Fn(&str) -> Option<PathBuf>,
    cancel: &AtomicBool,
    progress: &mut dyn FnMut(&'static str, u64, u64),
) -> fluxdb_core::Result<()> {
    let mut archive = zip::ZipArchive::new(reader)
        .map_err(|error| internal_error(format!("解析客户端压缩包失败：{error}")))?;
    // 先按中央目录挑出要解压的条目，避免对整包上万个条目逐个做 IO。
    let wanted: Vec<(usize, PathBuf)> = (0..archive.len())
        .filter_map(|index| {
            let name = archive.name_for_index(index)?;
            select_entry(name).map(|target| (index, target))
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
        // bin/lib 必须是实体目录，不能经已有链接写到安装目录之外。
        let parent = target.parent().expect("客户端条目包含 bin/lib 目录");
        if std::fs::symlink_metadata(parent)
            .map_err(|error| internal_error(format!("检查安装目录失败：{error}")))?
            .file_type()
            .is_symlink()
        {
            return Err(internal_error(format!(
                "安装子目录不能是符号链接：{}",
                parent.display()
            )));
        }
        let mode = entry.unix_mode();
        if is_symlink_mode(mode) {
            write_symlink_entry(&mut entry, &target)?;
            continue;
        }
        // 删除目录项本身再独占创建，既不跟随符号链接，也不截断已有硬链接指向的文件。
        match std::fs::remove_file(&target) {
            Ok(()) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => return Err(internal_error(format!("替换客户端文件失败：{error}"))),
        }
        let mut output = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&target)
            .map_err(|error| internal_error(format!("写入 {} 失败：{error}", target.display())))?;
        std::io::copy(&mut entry, &mut output)
            .map_err(|error| internal_error(format!("解压 {} 失败：{error}", target.display())))?;
        apply_executable_mode(&target, mode);
    }
    Ok(())
}

/// zip 条目是否为符号链接（macOS 包里 `libpq.dylib -> libpq.5.dylib` 这类）。
fn is_symlink_mode(mode: Option<u32>) -> bool {
    !cfg!(target_os = "windows") && mode.is_some_and(|mode| mode & 0xF000 == 0xA000)
}

/// 还原符号链接条目：条目内容即链接目标路径。
///
/// 返回 false 表示该条目被安全策略跳过：官方包（如 EDB macOS 版）里 pgAdmin 内嵌 Python
/// framework 的 `libpython3.13.dylib -> ../Python` 是合法的跨目录链接，客户端工具并不依赖它；
/// 这类条目不落盘、只记日志，不能让整次安装失败。非法链接一律不创建，安全边界不变。
fn write_symlink_entry(entry: &mut impl Read, target: &Path) -> fluxdb_core::Result<bool> {
    let mut link = String::new();
    entry
        .read_to_string(&mut link)
        .map_err(|error| internal_error(format!("读取符号链接条目失败：{error}")))?;
    // 官方动态库链接仅引用同目录的库文件；拒绝绝对路径、跨目录和链式逃逸。
    if link.is_empty()
        || link.contains('/')
        || link.contains('\\')
        || link == "."
        || link == ".."
        || Path::new(&link)
            .components()
            .any(|part| !matches!(part, std::path::Component::Normal(_)))
    {
        tracing::warn!(
            link = %link,
            target = %target.display(),
            "客户端压缩包含不受支持的跨目录符号链接，已跳过该条目"
        );
        return Ok(false);
    }
    let destination = target.with_file_name(&link);
    if std::fs::symlink_metadata(&destination)
        .is_ok_and(|metadata| metadata.file_type().is_symlink())
    {
        tracing::warn!(
            link = %link,
            target = %target.display(),
            "客户端压缩包符号链接指向其他链接，已跳过该条目"
        );
        return Ok(false);
    }
    match std::fs::remove_file(target) {
        Ok(()) => {}
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => return Err(internal_error(format!("替换客户端链接失败：{error}"))),
    }
    #[cfg(unix)]
    std::os::unix::fs::symlink(&link, target)
        .map_err(|error| internal_error(format!("创建符号链接失败：{error}")))?;
    Ok(true)
}

/// 还原可执行权限（zip 里的 unix mode 丢失会导致解压出来的 pg_dump 不可执行）。
fn apply_executable_mode(target: &Path, mode: Option<u32>) {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = mode.unwrap_or(0o755) & 0o777;
        let mode = if mode == 0 { 0o755 } else { mode };
        if let Err(error) = std::fs::set_permissions(target, std::fs::Permissions::from_mode(mode))
        {
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
        self.fetched.fetch_add(body.len() as u64, Ordering::Relaxed);
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
