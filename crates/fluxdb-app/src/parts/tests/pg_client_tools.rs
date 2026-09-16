// PostgreSQL 客户端工具发现/解析/下载的单测。下载相关用本地一次性 HTTP 服务，不触公网。

/// 建一个临时的假客户端 bin 目录，里面放一个会打印指定版本号的 pg_dump 脚本。
#[cfg(unix)]
fn fake_pg_client_dir(label: &str, version: &str) -> std::path::PathBuf {
    use std::os::unix::fs::PermissionsExt;

    let root = temp_pg_client_path(label);
    let bin = root.join("bin");
    std::fs::create_dir_all(&bin).unwrap();
    let program = bin.join("pg_dump");
    std::fs::write(
        &program,
        format!("#!/bin/sh\necho \"pg_dump (PostgreSQL) {version}\"\n"),
    )
    .unwrap();
    std::fs::set_permissions(&program, std::fs::Permissions::from_mode(0o755)).unwrap();
    root
}

/// 唯一临时路径（同一测试进程内不冲突）。
fn temp_pg_client_path(label: &str) -> std::path::PathBuf {
    let suffix = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    std::env::temp_dir().join(format!(
        "fluxdb-pg-client-{label}-{}-{suffix}",
        std::process::id()
    ))
}

/// 造一个结构与官方二进制包一致的压缩包：`pgsql/bin/*` + `pgsql/lib/*`，
/// 其中 pg_dump/psql 是可执行脚本，便于校验解压后的权限与「能否运行」。
/// `padding` 用来放大包体积，模拟「整包很大、需要的只有一点」的真实情况。
#[cfg(unix)]
fn fake_pg_client_archive(version: &str, padding: usize) -> Vec<u8> {
    use std::io::Write as _;
    use zip::write::SimpleFileOptions;

    let mut buffer = std::io::Cursor::new(Vec::new());
    let mut writer = zip::ZipWriter::new(&mut buffer);
    // 不压缩，保证「只取需要的字节」在测试里可度量。
    let exec = SimpleFileOptions::default()
        .compression_method(zip::CompressionMethod::Stored)
        .unix_permissions(0o755);
    let data = SimpleFileOptions::default()
        .compression_method(zip::CompressionMethod::Stored)
        .unix_permissions(0o644);
    writer.start_file("pgsql/bin/pg_dump", exec).unwrap();
    writer
        .write_all(format!("#!/bin/sh\necho \"pg_dump (PostgreSQL) {version}\"\n").as_bytes())
        .unwrap();
    writer.start_file("pgsql/bin/psql", exec).unwrap();
    writer
        .write_all(format!("#!/bin/sh\necho \"psql (PostgreSQL) {version}\"\n").as_bytes())
        .unwrap();
    // 服务端程序、头文件、可选 ICU 库都不该进精简安装。
    writer.start_file("pgsql/bin/postgres", exec).unwrap();
    writer.write_all(&vec![b'p'; padding]).unwrap();
    writer.start_file("pgsql/include/libpq-fe.h", data).unwrap();
    writer.write_all(b"// header\n").unwrap();
    writer.start_file("pgsql/lib/libpq.5.dylib", data).unwrap();
    writer.write_all(b"fake dylib\n").unwrap();
    writer
        .start_file("pgsql/lib/libicudata.68.2.dylib", data)
        .unwrap();
    writer.write_all(&vec![b'i'; padding]).unwrap();
    writer.finish().unwrap();
    buffer.into_inner()
}

/// 本地测试 HTTP 服务：`support_range=false` 时忽略 Range 一律回 200（用于验证回退整包），
/// `true` 时按 Range 回 206 并统计实际发出的字节数（用于验证「只下需要的部分」）。
#[cfg(unix)]
fn serve_archive(
    body: Vec<u8>,
    support_range: bool,
) -> (String, Arc<std::sync::atomic::AtomicU64>) {
    use std::io::{BufRead as _, BufReader, Write as _};

    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let url = format!("http://{}/client.zip", listener.local_addr().unwrap());
    let served = Arc::new(std::sync::atomic::AtomicU64::new(0));
    let served_for_thread = served.clone();
    // 线程随进程结束回收：测试不 join，避免最后一次 accept 阻塞。
    std::thread::spawn(move || {
        while let Ok((stream, _)) = listener.accept() {
            let mut reader = BufReader::new(stream);
            let mut range: Option<(u64, u64)> = None;
            loop {
                let mut line = String::new();
                if reader.read_line(&mut line).unwrap_or(0) == 0 || line == "\r\n" {
                    break;
                }
                if support_range && line.to_ascii_lowercase().starts_with("range:") {
                    let spec = line.split('=').nth(1).unwrap_or("").trim().to_string();
                    let mut bounds = spec.split('-');
                    let start: u64 = bounds.next().unwrap_or("0").parse().unwrap_or(0);
                    let end: u64 = bounds
                        .next()
                        .and_then(|value| value.parse().ok())
                        .unwrap_or(body.len() as u64 - 1);
                    range = Some((start, end.min(body.len() as u64 - 1)));
                }
            }
            let stream = reader.get_mut();
            let (status, slice, extra) = match range {
                Some((start, end)) => (
                    "206 Partial Content",
                    &body[start as usize..=end as usize],
                    format!("Content-Range: bytes {start}-{end}/{}\r\n", body.len()),
                ),
                None => ("200 OK", &body[..], String::new()),
            };
            served_for_thread.fetch_add(slice.len() as u64, Ordering::Relaxed);
            let header = format!(
                "HTTP/1.1 {status}\r\nContent-Length: {}\r\n{extra}Connection: close\r\n\r\n",
                slice.len()
            );
            let _ = stream.write_all(header.as_bytes());
            let _ = stream.write_all(slice);
            let _ = stream.flush();
        }
    });
    (url, served)
}

#[test]
#[cfg(unix)]
fn download_pg_client_fetches_only_needed_entries_over_range() {
    let install_dir = temp_pg_client_path("install-range");
    // 8MB 的「服务端程序」+ 8MB 的 ICU 数据：都不该被下载。
    let archive = fake_pg_client_archive("17.6", 8 * 1024 * 1024);
    let archive_size = archive.len() as u64;
    let (url, served) = serve_archive(archive, true);
    let cancel = std::sync::atomic::AtomicBool::new(false);
    let mut stages: Vec<&'static str> = Vec::new();

    let bin_dir = download_pg_client(&url, &install_dir, &cancel, &mut |progress| {
        if stages.last() != Some(&progress.stage) {
            stages.push(progress.stage);
        }
    })
    .unwrap();

    assert_eq!(bin_dir, install_dir.join("bin"));
    assert!(bin_dir.join("pg_dump").is_file());
    assert!(bin_dir.join("psql").is_file());
    assert!(install_dir.join("lib/libpq.5.dylib").is_file());
    // 服务端程序、头文件、ICU 可选库都没落盘。
    assert!(!bin_dir.join("postgres").exists());
    assert!(!install_dir.join("include").exists());
    assert!(!install_dir.join("lib/libicudata.68.2.dylib").exists());
    assert_eq!(pg_tool_major_at(&bin_dir.join("pg_dump")), Some(17));
    // 关键断言：实际传输字节数远小于整包（不再「下整包扔掉绝大部分」）。
    let transferred = served.load(Ordering::Relaxed);
    assert!(
        transferred < archive_size / 4,
        "分片下载应只取需要的条目，实际 {transferred} / 整包 {archive_size}"
    );
    // 分片模式不产生整包临时文件。
    assert!(!install_dir.join(".pg-client-download.part").exists());
    assert_eq!(stages, vec!["extract", "verify"]);

    std::fs::remove_dir_all(&install_dir).ok();
}

#[test]
#[cfg(unix)]
fn download_pg_client_falls_back_to_full_download_without_range() {
    let install_dir = temp_pg_client_path("install-full");
    let (url, _served) = serve_archive(fake_pg_client_archive("16.10", 4096), false);
    let cancel = std::sync::atomic::AtomicBool::new(false);
    let mut stages: Vec<&'static str> = Vec::new();

    let bin_dir = download_pg_client(&url, &install_dir, &cancel, &mut |progress| {
        if stages.last() != Some(&progress.stage) {
            stages.push(progress.stage);
        }
    })
    .unwrap();

    assert_eq!(pg_tool_major_at(&bin_dir.join("pg_dump")), Some(16));
    assert!(!bin_dir.join("postgres").exists());
    // 回退路径先整包落地再解压，结束后临时文件必须清理。
    assert!(!install_dir.join(".pg-client-download.part").exists());
    assert_eq!(stages, vec!["download", "extract", "verify"]);

    std::fs::remove_dir_all(&install_dir).ok();
}

#[test]
#[cfg(unix)]
fn download_pg_client_reports_verify_failure() {
    use std::io::Write as _;

    let install_dir = temp_pg_client_path("install-bad");
    // 压缩包里没有 pg_dump：两轮（精简 + 补全依赖）都装不出可执行工具，必须明确报错。
    let archive = {
        let mut buffer = std::io::Cursor::new(Vec::new());
        let mut writer = zip::ZipWriter::new(&mut buffer);
        writer
            .start_file("pgsql/bin/psql", zip::write::SimpleFileOptions::default())
            .unwrap();
        writer.write_all(b"#!/bin/sh\nexit 0\n").unwrap();
        writer.finish().unwrap();
        buffer.into_inner()
    };
    let (url, _served) = serve_archive(archive, true);
    let cancel = std::sync::atomic::AtomicBool::new(false);

    let error = download_pg_client(&url, &install_dir, &cancel, &mut |_| {}).unwrap_err();

    assert!(error.to_string().contains("客户端安装校验失败"));
    std::fs::remove_dir_all(&install_dir).ok();
}

#[test]
#[cfg(unix)]
fn managed_install_without_marker_is_ignored() {
    let root = temp_pg_client_path("managed-root");
    // 半成品：pg_dump 在，但没走完校验（无完成标记）——安装中途被杀就会留下这种目录。
    let broken = root.join("17.6-1").join("bin");
    std::fs::create_dir_all(&broken).unwrap();
    std::fs::write(broken.join("pg_dump"), b"#!/bin/sh\nexit 0\n").unwrap();
    // 完整：带完成标记。
    let complete = root.join("16.10-1").join("bin");
    std::fs::create_dir_all(&complete).unwrap();
    std::fs::write(complete.join("pg_dump"), b"#!/bin/sh\nexit 0\n").unwrap();
    std::fs::write(root.join("16.10-1").join(PG_CLIENT_INSTALL_MARKER), b"16").unwrap();

    let dirs: Vec<String> = pg_client_managed_dirs_in(&root)
        .into_iter()
        .map(|dir| dir.display().to_string())
        .collect();

    assert_eq!(dirs, vec![complete.display().to_string()]);

    std::fs::remove_dir_all(&root).ok();
}

#[test]
#[cfg(unix)]
fn pg_tool_major_at_gives_up_on_hanging_tool() {
    use std::os::unix::fs::PermissionsExt;

    let root = temp_pg_client_path("hanging-tool");
    std::fs::create_dir_all(&root).unwrap();
    let program = root.join("pg_dump");
    // 模拟依赖库缺失等异常下「不退出」的工具：必须被超时终止，而不是把调用方拖死。
    std::fs::write(&program, b"#!/bin/sh\nsleep 60\n").unwrap();
    std::fs::set_permissions(&program, std::fs::Permissions::from_mode(0o755)).unwrap();

    let started = std::time::Instant::now();
    let major = pg_tool_major_at(&program);

    assert_eq!(major, None);
    assert!(
        started.elapsed() < PG_CLIENT_VERSION_TIMEOUT * 3,
        "应在超时后很快返回，实际耗时 {:?}",
        started.elapsed()
    );

    std::fs::remove_dir_all(&root).ok();
}

#[test]
fn pg_client_archive_target_keeps_only_client_files() {
    // 客户端可执行文件保留，服务端程序与头文件跳过。
    assert_eq!(
        pg_client_archive_target("pgsql/bin/pg_dump", false),
        Some(std::path::PathBuf::from("bin/pg_dump"))
    );
    assert_eq!(
        pg_client_archive_target("pgsql/bin/pg_restore.exe", false),
        Some(std::path::PathBuf::from("bin/pg_restore.exe"))
    );
    assert_eq!(pg_client_archive_target("pgsql/bin/postgres", false), None);
    assert_eq!(
        pg_client_archive_target("pgsql/include/libpq-fe.h", false),
        None
    );
    // zip slip 防御：带上跳路径的条目一律丢弃。
    assert_eq!(pg_client_archive_target("../../etc/passwd", false), None);
}

#[test]
fn pg_client_archive_target_narrows_to_runtime_libraries_by_default() {
    // 精简安装只取运行时依赖闭包：常规运行库保留。
    assert_eq!(
        pg_client_archive_target("pgsql/bin/libpq.dll", false),
        Some(std::path::PathBuf::from("bin/libpq.dll"))
    );
    assert_eq!(
        pg_client_archive_target("pgsql/bin/libssl-3-x64.dll", false),
        Some(std::path::PathBuf::from("bin/libssl-3-x64.dll"))
    );
    // 有匹配的库都不是客户端依赖：跳过。
    for skipped in [
        "pgsql/lib/libxml2.16.dylib",
        "pgsql/lib/libhuge.dylib",
        "pgsql/lib/libicudata.68.2.dylib",
        "pgsql/lib/libwx_base.dylib",
    ] {
        assert_eq!(pg_client_archive_target(skipped, false), None, "{skipped}");
    }
    // 补齐轮放开限制：任何动态库都取，保证不会因过度精简装出跑不起来的工具。
    for kept in [
        "pgsql/lib/libxml2.16.dylib",
        "pgsql/lib/libhuge.dylib",
        "pgsql/lib/libicudata.68.2.dylib",
        "pgsql/lib/libwx_base.dylib",
    ] {
        assert!(
            pg_client_archive_target(kept, true).is_some(),
            "补齐轮应保留 {kept}"
        );
    }
    // pgAdmin 内嵌 Python framework 的库与客户端工具无关，且与真依赖同名
    // （libssl.3.dylib 会覆盖 pgsql/lib 里的同名库）、还带跨目录符号链接，两轮都不收。
    for skipped in [
        "pgsql/pgAdmin 4.app/Contents/Frameworks/Python.framework/Versions/3.13/lib/libssl.3.dylib",
        "pgsql/pgAdmin 4.app/Contents/Frameworks/Python.framework/Versions/3.13/lib/libpython3.13.dylib",
        "pgsql/pgAdmin 4.app/Contents/Frameworks/QtWidgets",
    ] {
        assert_eq!(pg_client_archive_target(skipped, false), None, "{skipped}");
        assert_eq!(pg_client_archive_target(skipped, true), None, "{skipped}");
    }
}

#[test]
#[cfg(not(target_os = "windows"))]
fn pg_client_archive_target_keeps_runtime_libraries_on_unix() {
    assert_eq!(
        pg_client_archive_target("pgsql/lib/libpq.5.dylib", false),
        Some(std::path::PathBuf::from("lib/libpq.5.dylib"))
    );
    // 服务端扩展在 lib 的子目录里，不属于客户端运行依赖。
    assert_eq!(
        pg_client_archive_target("pgsql/lib/postgresql/plpgsql.so", false),
        None
    );
    // 静态库不是运行依赖。
    assert_eq!(pg_client_archive_target("pgsql/lib/libpq.a", false), None);
}

#[test]
fn pg_client_download_url_follows_server_major_version() {
    let Some(platform) = pg_client_platform_slug() else {
        // Linux 无官方免安装包，只给安装引导。
        assert!(pg_client_download_url("", Some(17)).is_none());
        return;
    };
    let url = pg_client_download_url("", Some(16)).unwrap();
    assert_eq!(
        url,
        format!(
            "https://get.enterprisedb.com/postgresql/postgresql-16.10-1-{platform}-binaries.zip"
        )
    );
    // 服务端版本未知时取内置最新版本（新客户端可备份旧服务端）。
    let fallback = pg_client_download_url("", None).unwrap();
    assert!(fallback.contains(PG_CLIENT_FALLBACK_VERSION));
}

#[test]
fn pg_client_download_url_supports_custom_source() {
    if pg_client_platform_slug().is_none() {
        return;
    }
    // 自建镜像：占位符按平台和版本替换。
    let url = pg_client_download_url(
        "https://mirror.internal/pg/{version}/{platform}.zip",
        Some(17),
    )
    .unwrap();
    assert!(url.starts_with("https://mirror.internal/pg/17.6-1/"));
    // 直链：没有占位符时原样使用。
    let direct = pg_client_download_url("https://mirror.internal/pg/client.zip", Some(17)).unwrap();
    assert_eq!(direct, "https://mirror.internal/pg/client.zip");
}

#[test]
#[cfg(unix)]
fn resolve_pg_client_tool_prefers_configured_dir() {
    let root = fake_pg_client_dir("configured", "16.10");
    let mut settings = Settings::default();
    settings.pg_client_dir = root.display().to_string();

    let resolved = resolve_pg_client_tool(&settings, PgClientTool::Dump, None).unwrap();

    assert_eq!(resolved.source, PgClientSource::Configured);
    assert_eq!(resolved.major_version, Some(16));
    assert_eq!(
        resolved.program,
        root.join("bin/pg_dump").display().to_string()
    );
    std::fs::remove_dir_all(&root).ok();
}

#[test]
#[cfg(unix)]
fn resolve_pg_client_tool_keeps_legacy_pg_dump_path() {
    let legacy = fake_pg_client_dir("legacy", "15.14");
    let configured = fake_pg_client_dir("configured-ignored", "16.10");
    let mut settings = Settings::default();
    settings.pg_dump_path = legacy.join("bin/pg_dump").display().to_string();
    settings.pg_client_dir = configured.display().to_string();

    // 旧设置项仍然优先，避免升级后既有配置失效。
    let dump = resolve_pg_client_tool(&settings, PgClientTool::Dump, None).unwrap();
    assert_eq!(dump.program, settings.pg_dump_path);
    assert_eq!(dump.major_version, Some(15));

    // 但只对 pg_dump 生效：psql 不会拿到旧的 pg_dump 路径，而是走自己的解析链
    // （可能有托管下载/系统安装的客户端，也可能落到 PATH 兜底，都取决于运行机器）。
    let psql = resolve_pg_client_tool(&settings, PgClientTool::Psql, None);
    assert!(
        psql.is_none_or(|psql| psql.program != settings.pg_dump_path),
        "psql 不应复用 pg_dump_path"
    );

    std::fs::remove_dir_all(&legacy).ok();
    std::fs::remove_dir_all(&configured).ok();
}

#[test]
#[cfg(unix)]
fn discover_pg_clients_reports_version_and_source() {
    let root = fake_pg_client_dir("discover", "18.1");
    let mut settings = Settings::default();
    settings.pg_client_dir = root.display().to_string();

    let found = discover_pg_clients(&settings);

    let configured = found
        .iter()
        .find(|location| location.source == PgClientSource::Configured)
        .expect("应发现配置目录中的客户端");
    assert_eq!(configured.bin_dir, root.join("bin"));
    assert_eq!(configured.major_version, Some(18));
    std::fs::remove_dir_all(&root).ok();
}

#[test]
#[cfg(unix)]
fn pg_client_reinstall_uses_same_root_for_bin_setting() {
    let root = temp_pg_client_path("reinstall");
    let (url, _) = serve_archive(fake_pg_client_archive("17.6", 0), true);
    let cancel = AtomicBool::new(false);
    let bin = download_pg_client(&url, &root, &cancel, &mut |_| {}).unwrap();
    let mut settings = Settings::default();
    settings.pg_client_dir = bin.display().to_string();
    assert_eq!(pg_client_install_dir(&settings, "17.6-1"), root);
    let again = download_pg_client(
        &url,
        &pg_client_install_dir(&settings, "17.6-1"),
        &cancel,
        &mut |_| {},
    )
    .unwrap();
    assert_eq!(again, bin);
    assert!(!bin.join("bin").exists());
    settings.pg_client_dir = root.display().to_string();
    assert_eq!(pg_client_install_dir(&settings, "17.6-1"), root);
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
#[cfg(unix)]
fn resolve_pg_client_without_requirement_preserves_directory_order() {
    // 两个配置候选可复现多版本场景，不修改进程 PATH 或依赖本机安装。
    let root = fake_pg_client_dir("priority", "16.10");
    std::fs::copy(root.join("bin/pg_dump"), root.join("pg_dump")).unwrap();
    std::fs::write(
        root.join("pg_dump"),
        b"#!/bin/sh\necho 'pg_dump (PostgreSQL) 18.1'\n",
    )
    .unwrap();
    let mut settings = Settings::default();
    settings.pg_client_dir = root.display().to_string();
    let selected = resolve_pg_client_tool(&settings, PgClientTool::Dump, None).unwrap();
    assert_eq!(
        selected.program,
        root.join("bin/pg_dump").display().to_string()
    );
    let compatible = resolve_pg_client_tool(&settings, PgClientTool::Dump, Some(18)).unwrap();
    assert_eq!(
        compatible.program,
        root.join("pg_dump").display().to_string()
    );
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
#[cfg(unix)]
fn pg_client_extract_does_not_follow_existing_links() {
    let root = temp_pg_client_path("extract-links");
    let outside = temp_pg_client_path("outside-file");
    std::fs::create_dir_all(root.join("bin")).unwrap();
    std::fs::write(&outside, b"keep me").unwrap();
    std::os::unix::fs::symlink(&outside, root.join("bin/pg_dump")).unwrap();
    extract_pg_client_archive(
        std::io::Cursor::new(fake_pg_client_archive("17.6", 0)),
        &root,
        false,
        &AtomicBool::new(false),
        &mut |_, _, _| {},
    )
    .unwrap();
    assert_eq!(std::fs::read(&outside).unwrap(), b"keep me");
    assert!(
        !std::fs::symlink_metadata(root.join("bin/pg_dump"))
            .unwrap()
            .file_type()
            .is_symlink()
    );
    std::fs::remove_dir_all(&root).unwrap();
    std::fs::create_dir_all(&root).unwrap();
    let outside_dir = temp_pg_client_path("outside-dir");
    std::fs::create_dir_all(&outside_dir).unwrap();
    std::os::unix::fs::symlink(&outside_dir, root.join("bin")).unwrap();
    assert!(
        extract_pg_client_archive(
            std::io::Cursor::new(fake_pg_client_archive("17.6", 0)),
            &root,
            false,
            &AtomicBool::new(false),
            &mut |_, _, _| {},
        )
        .is_err()
    );
    assert!(!outside_dir.join("pg_dump").exists());
    std::fs::remove_dir_all(root).unwrap();
    std::fs::remove_dir_all(outside_dir).unwrap();
    std::fs::remove_file(outside).unwrap();
}

#[test]
#[cfg(unix)]
fn pg_client_archive_skips_escaping_symlink_and_allows_library_alias() {
    // 不安全的链接（绝对路径/跨目录/逃逸）不创建、也不让整包安装失败：
    // 官方 macOS 包里 pgAdmin Python framework 的 `libpython -> ../Python` 就是合法的跨目录链接。
    for link in [
        "../../outside",
        "/tmp/outside",
        "..\\outside",
        "../Python",
    ] {
        let root = temp_pg_client_path("archive-symlink");
        let mut archive = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
        archive
            .add_symlink(
                // 用白名单里的 libpq 前缀命名，保证条目会被选取、真正走到链接还原逻辑。
                "pgsql/lib/libpq.dylib",
                link,
                zip::write::SimpleFileOptions::default(),
            )
            .unwrap();
        let bytes = archive.finish().unwrap().into_inner();
        extract_pg_client_archive(
            std::io::Cursor::new(bytes),
            &root,
            false,
            &AtomicBool::new(false),
            &mut |_, _, _| {},
        )
        .unwrap_or_else(|error| panic!("跨目录链接 {link} 应被跳过而不是报错：{error}"));
        assert!(
            !std::fs::symlink_metadata(root.join("lib/libpq.dylib")).is_ok(),
            "链接 {link} 不应落盘"
        );
        std::fs::remove_dir_all(root).unwrap();
    }

    // 同目录的库别名链接（libpq.dylib -> libpq.5.dylib）仍正常还原。
    let root = temp_pg_client_path("archive-symlink-alias");
    let mut archive = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
    archive
        .add_symlink(
            "pgsql/lib/libpq.dylib",
            "libpq.5.dylib",
            zip::write::SimpleFileOptions::default(),
        )
        .unwrap();
    let bytes = archive.finish().unwrap().into_inner();
    extract_pg_client_archive(
        std::io::Cursor::new(bytes),
        &root,
        false,
        &AtomicBool::new(false),
        &mut |_, _, _| {},
    )
    .unwrap();
    assert_eq!(
        std::fs::read_link(root.join("lib/libpq.dylib")).unwrap(),
        PathBuf::from("libpq.5.dylib")
    );
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
#[cfg(unix)]
fn download_pg_client_tolerates_framework_symlinks_in_optional_round() {
    use std::io::Write as _;

    let install_dir = temp_pg_client_path("install-framework");
    // 还原真实场景：pgAdmin Python framework 里的 libpython3.13.dylib -> ../Python
    // 出现在补齐轮（include_optional=true）的选取范围里，不能让整次安装失败。
    // pg_dump 脚本依赖非白名单的可选库 libicudata：精简轮校验必失败，强制走进补齐轮。
    let archive = {
        let mut buffer = std::io::Cursor::new(Vec::new());
        let mut writer = zip::ZipWriter::new(&mut buffer);
        let exec = zip::write::SimpleFileOptions::default().unix_permissions(0o755);
        let data = zip::write::SimpleFileOptions::default().unix_permissions(0o644);
        writer.start_file("pgsql/bin/pg_dump", exec).unwrap();
        writer
            .write_all(
                b"#!/bin/sh\n\
                  [ -f \"$(dirname \"$0\")/../lib/libicudata.77.1.dylib\" ] || exit 1\n\
                  echo \"pg_dump (PostgreSQL) 18.1\"\n",
            )
            .unwrap();
        writer.start_file("pgsql/bin/psql", exec).unwrap();
        writer
            .write_all(b"#!/bin/sh\necho \"psql (PostgreSQL) 18.1\"\n")
            .unwrap();
        writer.start_file("pgsql/lib/libpq.5.dylib", data).unwrap();
        writer.write_all(b"fake dylib\n").unwrap();
        writer
            .start_file("pgsql/lib/libicudata.77.1.dylib", data)
            .unwrap();
        writer.write_all(b"fake icu\n").unwrap();
        writer
            .add_symlink("pgsql/lib/libpython3.13.dylib", "../Python", data)
            .unwrap();
        writer.finish().unwrap();
        buffer.into_inner()
    };
    let (url, _served) = serve_archive(archive, true);
    let cancel = AtomicBool::new(false);

    let bin_dir = download_pg_client(&url, &install_dir, &cancel, &mut |_| {}).unwrap();

    assert_eq!(pg_tool_major_at(&bin_dir.join("pg_dump")), Some(18));
    let lib_dir = install_dir.join("lib");
    assert!(!lib_dir.join("libpython3.13.dylib").exists());
    assert!(lib_dir.join("libpq.5.dylib").is_file());
    std::fs::remove_dir_all(&install_dir).ok();
}
