// GitHub Releases 更新协议。
//
// 免费方案不要求 macOS/Windows 发布者证书，但更新清单必须由内置 Ed25519
// 公钥验证；清单中的资产再按 SHA-256 校验。安装动作只负责启动平台安装器，
// 不直接覆盖正在运行的可执行文件。

use base64::Engine as _;
use serde::{Deserialize as SerdeDeserialize, Serialize as SerdeSerialize};
use sha2::Digest as _;

const UPDATE_SCHEMA_VERSION: u32 = 1;
const UPDATE_MANIFEST_URL: &str =
    "https://github.com/fluxdb-alt/fluxDB/releases/latest/download/latest.json";
const UPDATE_SIGNATURE_URL: &str =
    "https://github.com/fluxdb-alt/fluxDB/releases/latest/download/latest.json.sig";
const MAX_MANIFEST_BYTES: u64 = 1024 * 1024;
const MAX_SIGNATURE_BYTES: u64 = 4 * 1024;
const MAX_UPDATE_BYTES: u64 = 512 * 1024 * 1024;
const DOWNLOAD_BUFFER_BYTES: usize = 256 * 1024;
/// 每次下载使用独立的临时文件，避免被取消的旧任务清理文件时误删新任务的进度。
static DOWNLOAD_SEQ: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

#[derive(Clone, Debug, SerdeDeserialize, SerdeSerialize, PartialEq, Eq)]
pub struct UpdateManifest {
    pub schema: u32,
    pub version: String,
    #[serde(default)]
    pub published_at: Option<String>,
    #[serde(default)]
    pub notes: std::collections::BTreeMap<String, String>,
    #[serde(default)]
    pub releases: Vec<UpdateRelease>,
    pub platforms: std::collections::BTreeMap<String, UpdateAsset>,
}

#[derive(Clone, Debug, SerdeDeserialize, SerdeSerialize, PartialEq, Eq)]
pub struct UpdateRelease {
    pub version: String,
    #[serde(default)]
    pub published_at: Option<String>,
    #[serde(default)]
    pub notes: std::collections::BTreeMap<String, String>,
}

#[derive(Clone, Debug, SerdeDeserialize, SerdeSerialize, PartialEq, Eq)]
pub struct UpdateAsset {
    pub url: String,
    pub sha256: String,
    pub size: u64,
    pub kind: UpdateAssetKind,
}

#[derive(Clone, Copy, Debug, SerdeDeserialize, SerdeSerialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum UpdateAssetKind {
    MacosAppTarGz,
    WindowsInno,
    LinuxDeb,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AvailableUpdate {
    pub current_version: String,
    pub version: String,
    pub published_at: Option<String>,
    pub releases: Vec<UpdateRelease>,
    pub asset: UpdateAsset,
    pub platform: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum UpdateCheck {
    UpToDate { current_version: String },
    Available(AvailableUpdate),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum UpdateErrorKind {
    Disabled,
    Network,
    InvalidManifest,
    InvalidSignature,
    UnsupportedPlatform,
    Download,
    Checksum,
    Cancelled,
    Install,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct UpdateError {
    pub kind: UpdateErrorKind,
    pub detail: String,
}

impl UpdateError {
    fn new(kind: UpdateErrorKind, detail: impl Into<String>) -> Self {
        Self {
            kind,
            detail: detail.into(),
        }
    }
}

impl std::fmt::Display for UpdateError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(&self.detail)
    }
}

impl std::error::Error for UpdateError {}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct UpdateDownloadProgress {
    pub downloaded: u64,
    pub total: u64,
}

pub fn current_update_platform() -> Option<&'static str> {
    #[cfg(all(target_os = "macos", target_arch = "aarch64"))]
    {
        return Some("macos-aarch64");
    }
    #[cfg(all(target_os = "macos", target_arch = "x86_64"))]
    {
        return Some("macos-x86_64");
    }
    #[cfg(all(target_os = "windows", target_arch = "x86_64"))]
    {
        return Some("windows-x86_64");
    }
    #[cfg(all(target_os = "linux", target_arch = "x86_64"))]
    {
        return Some("linux-x86_64");
    }
    #[allow(unreachable_code)]
    None
}

fn expected_asset_kind() -> Option<UpdateAssetKind> {
    #[cfg(target_os = "macos")]
    {
        return Some(UpdateAssetKind::MacosAppTarGz);
    }
    #[cfg(target_os = "windows")]
    {
        return Some(UpdateAssetKind::WindowsInno);
    }
    #[cfg(target_os = "linux")]
    {
        return Some(UpdateAssetKind::LinuxDeb);
    }
    #[allow(unreachable_code)]
    None
}

/// 下载并验证最新稳定版清单。自动检测与手动检测共用这一入口。
pub fn check_for_updates(current_version: &str) -> Result<UpdateCheck, UpdateError> {
    let current = semver::Version::parse(current_version).map_err(|error| {
        UpdateError::new(
            UpdateErrorKind::InvalidManifest,
            format!("当前应用版本无效：{error}"),
        )
    })?;
    let platform = current_update_platform().ok_or_else(|| {
        UpdateError::new(UpdateErrorKind::UnsupportedPlatform, "当前平台不支持应用内更新")
    })?;
    let expected_kind = expected_asset_kind().ok_or_else(|| {
        UpdateError::new(UpdateErrorKind::UnsupportedPlatform, "当前平台没有更新包类型")
    })?;
    let public_key = update_public_key()?;
    let agent = network_agent(std::time::Duration::from_secs(20));
    let manifest_url = update_manifest_url();
    let signature_url = update_signature_url();
    let manifest_bytes = fetch_bytes(
        &agent,
        &manifest_url,
        MAX_MANIFEST_BYTES,
        "读取更新清单失败",
    )?;
    let signature_text = fetch_bytes(
        &agent,
        &signature_url,
        MAX_SIGNATURE_BYTES,
        "读取更新签名失败",
    )?;
    verify_manifest_signature(&manifest_bytes, &signature_text, &public_key)?;
    let manifest: UpdateManifest = serde_json::from_slice(&manifest_bytes).map_err(|error| {
        UpdateError::new(
            UpdateErrorKind::InvalidManifest,
            format!("更新清单格式无效：{error}"),
        )
    })?;
    validate_manifest(&manifest, &current, platform, expected_kind)
}

pub fn validate_manifest(
    manifest: &UpdateManifest,
    current: &semver::Version,
    platform: &str,
    expected_kind: UpdateAssetKind,
) -> Result<UpdateCheck, UpdateError> {
    if manifest.schema != UPDATE_SCHEMA_VERSION {
        return Err(UpdateError::new(
            UpdateErrorKind::InvalidManifest,
            format!("不支持的更新清单版本：{}", manifest.schema),
        ));
    }
    let remote = semver::Version::parse(manifest.version.trim_start_matches('v')).map_err(|error| {
        UpdateError::new(
            UpdateErrorKind::InvalidManifest,
            format!("Release 版本无效：{error}"),
        )
    })?;
    if remote <= *current {
        return Ok(UpdateCheck::UpToDate {
            current_version: current.to_string(),
        });
    }

    let asset = manifest.platforms.get(platform).ok_or_else(|| {
        UpdateError::new(
            UpdateErrorKind::UnsupportedPlatform,
            format!("最新版本没有提供 {platform} 更新包"),
        )
    })?;
    if asset.kind != expected_kind {
        return Err(UpdateError::new(
            UpdateErrorKind::InvalidManifest,
            format!("{platform} 更新包类型不匹配"),
        ));
    }
    validate_asset(asset)?;

    let mut releases = manifest
        .releases
        .iter()
        .filter_map(|release| {
            let version =
                semver::Version::parse(release.version.trim_start_matches('v')).ok()?;
            (version > *current && version <= remote).then_some((version, release.clone()))
        })
        .collect::<Vec<_>>();
    releases.sort_by(|left, right| right.0.cmp(&left.0));
    let releases = if releases.is_empty() {
        vec![UpdateRelease {
            version: manifest.version.clone(),
            published_at: manifest.published_at.clone(),
            notes: manifest.notes.clone(),
        }]
    } else {
        releases
            .into_iter()
            .map(|(_, release)| release)
            .collect()
    };

    Ok(UpdateCheck::Available(AvailableUpdate {
        current_version: current.to_string(),
        version: manifest.version.clone(),
        published_at: manifest.published_at.clone(),
        releases,
        asset: asset.clone(),
        platform: platform.to_string(),
    }))
}

fn validate_asset(asset: &UpdateAsset) -> Result<(), UpdateError> {
    let url = url::Url::parse(&asset.url).map_err(|error| {
        UpdateError::new(
            UpdateErrorKind::InvalidManifest,
            format!("更新包地址无效：{error}"),
        )
    })?;
    let local_debug_url = cfg!(debug_assertions)
        && url.scheme() == "http"
        && matches!(url.host_str(), Some("127.0.0.1" | "localhost"));
    if url.scheme() != "https" && !local_debug_url {
        return Err(UpdateError::new(
            UpdateErrorKind::InvalidManifest,
            "更新包必须使用 HTTPS",
        ));
    }
    if asset.size == 0 || asset.size > MAX_UPDATE_BYTES {
        return Err(UpdateError::new(
            UpdateErrorKind::InvalidManifest,
            "更新包大小超出允许范围",
        ));
    }
    if decode_sha256(&asset.sha256).is_none() {
        return Err(UpdateError::new(
            UpdateErrorKind::InvalidManifest,
            "更新包 SHA-256 格式无效",
        ));
    }
    Ok(())
}

fn update_public_key() -> Result<Vec<u8>, UpdateError> {
    let encoded = option_env!("FLUXDB_UPDATE_PUBLIC_KEY")
        .unwrap_or_default()
        .trim();
    if encoded.is_empty() {
        return Err(UpdateError::new(
            UpdateErrorKind::Disabled,
            "此构建未配置更新签名公钥",
        ));
    }
    let key = base64::engine::general_purpose::STANDARD
        .decode(encoded)
        .map_err(|error| {
            UpdateError::new(
                UpdateErrorKind::InvalidSignature,
                format!("内置更新公钥无效：{error}"),
            )
        })?;
    if key.len() != 32 {
        return Err(UpdateError::new(
            UpdateErrorKind::InvalidSignature,
            "内置更新公钥长度无效",
        ));
    }
    Ok(key)
}

fn update_manifest_url() -> String {
    if cfg!(debug_assertions)
        && let Ok(url) = std::env::var("FLUXDB_UPDATE_MANIFEST_URL")
        && !url.trim().is_empty()
    {
        return url;
    }
    UPDATE_MANIFEST_URL.to_string()
}

fn update_signature_url() -> String {
    if cfg!(debug_assertions)
        && let Ok(url) = std::env::var("FLUXDB_UPDATE_SIGNATURE_URL")
        && !url.trim().is_empty()
    {
        return url;
    }
    UPDATE_SIGNATURE_URL.to_string()
}

pub fn verify_manifest_signature(
    manifest: &[u8],
    signature_text: &[u8],
    public_key: &[u8],
) -> Result<(), UpdateError> {
    let signature_text = std::str::from_utf8(signature_text).map_err(|error| {
        UpdateError::new(
            UpdateErrorKind::InvalidSignature,
            format!("更新签名不是 UTF-8：{error}"),
        )
    })?;
    // GNU base64 默认每 76 列换行，已发布的签名文件因此不是单行。
    let signature_encoded = signature_text
        .chars()
        .filter(|character| !character.is_ascii_whitespace())
        .collect::<String>();
    let signature = base64::engine::general_purpose::STANDARD
        .decode(signature_encoded)
        .map_err(|error| {
            UpdateError::new(
                UpdateErrorKind::InvalidSignature,
                format!("更新签名编码无效：{error}"),
            )
        })?;
    ring::signature::UnparsedPublicKey::new(&ring::signature::ED25519, public_key)
        .verify(manifest, &signature)
        .map_err(|_| {
            UpdateError::new(
                UpdateErrorKind::InvalidSignature,
                "更新清单签名验证失败",
            )
        })
}

pub fn localized_note<'a>(
    notes: &'a std::collections::BTreeMap<String, String>,
    locale: &str,
) -> Option<&'a str> {
    notes
        .get(locale)
        .or_else(|| (locale.starts_with("zh")).then(|| notes.get("zh-CN")).flatten())
        .or_else(|| notes.get("en"))
        .or_else(|| notes.values().next())
        .map(String::as_str)
}

pub fn download_update(
    asset: &UpdateAsset,
    target_dir: &std::path::Path,
    cancel: &std::sync::atomic::AtomicBool,
    progress: &mut dyn FnMut(UpdateDownloadProgress),
) -> Result<std::path::PathBuf, UpdateError> {
    validate_asset(asset)?;
    std::fs::create_dir_all(target_dir).map_err(|error| {
        UpdateError::new(
            UpdateErrorKind::Download,
            format!("创建更新缓存目录失败：{error}"),
        )
    })?;
    let part_path = target_dir.join(format!(
        "update-{}-{}.part",
        std::process::id(),
        DOWNLOAD_SEQ.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
    ));
    let final_path = target_dir.join(update_file_name(asset.kind));
    let response = network_agent(std::time::Duration::from_secs(30 * 60))
        .get(&asset.url)
        .call()
        .map_err(|error| {
            UpdateError::new(
                UpdateErrorKind::Network,
                format!("下载更新失败：{error}"),
            )
        })?;
    if response.status() != 200 {
        return Err(UpdateError::new(
            UpdateErrorKind::Download,
            format!("下载更新失败：HTTP {}", response.status()),
        ));
    }
    let total = response
        .headers()
        .get("content-length")
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.parse::<u64>().ok())
        .unwrap_or(asset.size);
    if total > MAX_UPDATE_BYTES {
        return Err(UpdateError::new(
            UpdateErrorKind::Download,
            "更新包大小超出允许范围",
        ));
    }
    let mut reader = response.into_body().into_reader();
    let mut file = std::fs::File::create(&part_path).map_err(|error| {
        UpdateError::new(
            UpdateErrorKind::Download,
            format!("创建更新临时文件失败：{error}"),
        )
    })?;
    let mut hasher = sha2::Sha256::new();
    let mut downloaded = 0u64;
    let mut buffer = vec![0u8; DOWNLOAD_BUFFER_BYTES];
    loop {
        if cancel.load(std::sync::atomic::Ordering::Relaxed) {
            let _ = std::fs::remove_file(&part_path);
            return Err(UpdateError::new(UpdateErrorKind::Cancelled, "下载已取消"));
        }
        let read = std::io::Read::read(&mut reader, &mut buffer).map_err(|error| {
            let _ = std::fs::remove_file(&part_path);
            UpdateError::new(
                UpdateErrorKind::Network,
                format!("读取更新包失败：{error}"),
            )
        })?;
        if read == 0 {
            break;
        }
        downloaded = downloaded.saturating_add(read as u64);
        if downloaded > MAX_UPDATE_BYTES || downloaded > asset.size.saturating_add(1) {
            let _ = std::fs::remove_file(&part_path);
            return Err(UpdateError::new(
                UpdateErrorKind::Download,
                "更新包大小与清单不一致",
            ));
        }
        std::io::Write::write_all(&mut file, &buffer[..read]).map_err(|error| {
            let _ = std::fs::remove_file(&part_path);
            UpdateError::new(
                UpdateErrorKind::Download,
                format!("写入更新包失败：{error}"),
            )
        })?;
        hasher.update(&buffer[..read]);
        progress(UpdateDownloadProgress {
            downloaded,
            total: total.max(downloaded),
        });
    }
    std::io::Write::flush(&mut file).map_err(|error| {
        let _ = std::fs::remove_file(&part_path);
        UpdateError::new(
            UpdateErrorKind::Download,
            format!("写入更新包失败：{error}"),
        )
    })?;
    if downloaded != asset.size {
        let _ = std::fs::remove_file(&part_path);
        return Err(UpdateError::new(
            UpdateErrorKind::Download,
            "更新包大小与清单不一致",
        ));
    }
    let actual = format!("{:x}", hasher.finalize());
    if !actual.eq_ignore_ascii_case(&asset.sha256) {
        let _ = std::fs::remove_file(&part_path);
        return Err(UpdateError::new(
            UpdateErrorKind::Checksum,
            "更新包 SHA-256 校验失败",
        ));
    }
    if final_path.exists() {
        std::fs::remove_file(&final_path).map_err(|error| {
            let _ = std::fs::remove_file(&part_path);
            UpdateError::new(
                UpdateErrorKind::Download,
                format!("替换旧更新缓存失败：{error}"),
            )
        })?;
    }
    std::fs::rename(&part_path, &final_path).map_err(|error| {
        let _ = std::fs::remove_file(&part_path);
        UpdateError::new(
            UpdateErrorKind::Download,
            format!("保存更新包失败：{error}"),
        )
    })?;
    Ok(final_path)
}

fn update_file_name(kind: UpdateAssetKind) -> &'static str {
    match kind {
        UpdateAssetKind::MacosAppTarGz => "FluxDB-update.app.tar.gz",
        UpdateAssetKind::WindowsInno => "FluxDB-update-setup.exe",
        UpdateAssetKind::LinuxDeb => "fluxdb-update.deb",
    }
}

fn decode_sha256(value: &str) -> Option<[u8; 32]> {
    if value.len() != 64 {
        return None;
    }
    let mut bytes = [0u8; 32];
    for (index, chunk) in value.as_bytes().chunks_exact(2).enumerate() {
        let text = std::str::from_utf8(chunk).ok()?;
        bytes[index] = u8::from_str_radix(text, 16).ok()?;
    }
    Some(bytes)
}

fn network_agent(body_timeout: std::time::Duration) -> ureq::Agent {
    let config = ureq::Agent::config_builder()
        .http_status_as_error(false)
        // 发布构建连同重定向一起限制为 HTTPS；debug 保留本地 HTTP 测试源。
        .https_only(!cfg!(debug_assertions))
        .max_redirects(8)
        .timeout_connect(Some(std::time::Duration::from_secs(12)))
        .timeout_recv_response(Some(std::time::Duration::from_secs(20)))
        .timeout_recv_body(Some(body_timeout))
        .build();
    ureq::Agent::new_with_config(config)
}

fn fetch_bytes(
    agent: &ureq::Agent,
    url: &str,
    limit: u64,
    context: &str,
) -> Result<Vec<u8>, UpdateError> {
    let response = agent
        .get(url)
        .call()
        .map_err(|error| UpdateError::new(UpdateErrorKind::Network, format!("{context}：{error}")))?;
    if response.status() != 200 {
        return Err(UpdateError::new(
            UpdateErrorKind::Network,
            format!("{context}：HTTP {}", response.status()),
        ));
    }
    let mut reader = response.into_body().into_reader().take(limit + 1);
    let mut bytes = Vec::new();
    std::io::Read::read_to_end(&mut reader, &mut bytes).map_err(|error| {
        UpdateError::new(UpdateErrorKind::Network, format!("{context}：{error}"))
    })?;
    if bytes.len() as u64 > limit {
        return Err(UpdateError::new(
            UpdateErrorKind::InvalidManifest,
            format!("{context}：响应超出大小上限"),
        ));
    }
    Ok(bytes)
}

pub fn install_update(
    asset: &UpdateAsset,
    downloaded_path: &std::path::Path,
    cache_dir: &std::path::Path,
) -> Result<(), UpdateError> {
    match asset.kind {
        UpdateAssetKind::WindowsInno => install_windows(downloaded_path),
        UpdateAssetKind::MacosAppTarGz => install_macos(downloaded_path, cache_dir),
        UpdateAssetKind::LinuxDeb => install_linux(downloaded_path, cache_dir),
    }
}

fn install_windows(downloaded_path: &std::path::Path) -> Result<(), UpdateError> {
    fluxdb_core::no_console(std::process::Command::new(downloaded_path))
        .args([
            "/VERYSILENT",
            "/SUPPRESSMSGBOXES",
            "/NORESTART",
            "/CLOSEAPPLICATIONS",
            "/UPDATE",
        ])
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .map_err(|error| {
            UpdateError::new(
                UpdateErrorKind::Install,
                format!("启动 Windows 更新安装器失败：{error}"),
            )
        })?;
    Ok(())
}

#[cfg(target_os = "macos")]
fn install_macos(
    downloaded_path: &std::path::Path,
    cache_dir: &std::path::Path,
) -> Result<(), UpdateError> {
    let install_dir = unique_install_dir(cache_dir)?;
    let output = std::process::Command::new("tar")
        .arg("-xzf")
        .arg(downloaded_path)
        .arg("-C")
        .arg(&install_dir)
        .output()
        .map_err(|error| {
            UpdateError::new(
                UpdateErrorKind::Install,
                format!("解压 macOS 更新包失败：{error}"),
            )
        })?;
    if !output.status.success() {
        return Err(UpdateError::new(
            UpdateErrorKind::Install,
            format!(
                "解压 macOS 更新包失败：{}",
                String::from_utf8_lossy(&output.stderr)
            ),
        ));
    }
    let staged_app = find_app_bundle(&install_dir).ok_or_else(|| {
        UpdateError::new(
            UpdateErrorKind::Install,
            "macOS 更新包缺少 .app 应用目录",
        )
    })?;
    let current_exe = std::env::current_exe().map_err(|error| {
        UpdateError::new(
            UpdateErrorKind::Install,
            format!("读取当前应用路径失败：{error}"),
        )
    })?;
    let target_app = current_exe
        .ancestors()
        .nth(3)
        .filter(|path| path.extension().is_some_and(|extension| extension == "app"))
        .ok_or_else(|| {
            UpdateError::new(
                UpdateErrorKind::UnsupportedPlatform,
                "当前进程不在标准 macOS .app 安装目录中",
            )
        })?;
    let script_path = install_dir.join("apply-update.sh");
    std::fs::write(
        &script_path,
        r#"#!/bin/sh
set -eu
pid="$1"
target="$2"
staged="$3"
while kill -0 "$pid" 2>/dev/null; do sleep 1; done
backup="${target}.fluxdb-old-$$"
rm -rf "$backup"
if [ -e "$target" ] && ! mv "$target" "$backup"; then
  open -n "$target"
  exit 1
fi
if mv "$staged" "$target"; then
  rm -rf "$backup"
  open -n "$target"
else
  if [ -e "$backup" ]; then mv "$backup" "$target"; fi
  if [ -e "$target" ]; then open -n "$target"; fi
  exit 1
fi
"#,
    )
    .map_err(|error| {
        UpdateError::new(
            UpdateErrorKind::Install,
            format!("创建 macOS 更新 helper 失败：{error}"),
        )
    })?;
    spawn_install_script(
        &script_path,
        &[
            std::process::id().to_string(),
            target_app.display().to_string(),
            staged_app.display().to_string(),
        ],
        &install_dir.join("apply-update.log"),
    )
}

#[cfg(not(target_os = "macos"))]
fn install_macos(
    _downloaded_path: &std::path::Path,
    _cache_dir: &std::path::Path,
) -> Result<(), UpdateError> {
    Err(UpdateError::new(
        UpdateErrorKind::UnsupportedPlatform,
        "当前平台不支持 macOS 更新包",
    ))
}

#[cfg(target_os = "linux")]
fn install_linux(
    downloaded_path: &std::path::Path,
    cache_dir: &std::path::Path,
) -> Result<(), UpdateError> {
    let install_dir = unique_install_dir(cache_dir)?;
    let current_exe = std::env::current_exe().map_err(|error| {
        UpdateError::new(
            UpdateErrorKind::Install,
            format!("读取当前应用路径失败：{error}"),
        )
    })?;
    let script_path = install_dir.join("apply-update.sh");
    std::fs::write(
        &script_path,
        r#"#!/bin/sh
set -eu
pid="$1"
package="$2"
exe="$3"
while kill -0 "$pid" 2>/dev/null; do sleep 1; done
if command -v pkexec >/dev/null 2>&1; then
  if pkexec dpkg -i "$package"; then
    nohup "$exe" >/dev/null 2>&1 &
  else
    nohup "$exe" >/dev/null 2>&1 &
    exit 1
  fi
elif command -v sudo >/dev/null 2>&1; then
  if sudo dpkg -i "$package"; then
    nohup "$exe" >/dev/null 2>&1 &
  else
    nohup "$exe" >/dev/null 2>&1 &
    exit 1
  fi
else
  nohup "$exe" >/dev/null 2>&1 &
  exit 1
fi
"#,
    )
    .map_err(|error| {
        UpdateError::new(
            UpdateErrorKind::Install,
            format!("创建 Linux 更新 helper 失败：{error}"),
        )
    })?;
    spawn_install_script(
        &script_path,
        &[
            std::process::id().to_string(),
            downloaded_path.display().to_string(),
            current_exe.display().to_string(),
        ],
        &install_dir.join("apply-update.log"),
    )
}

#[cfg(not(target_os = "linux"))]
fn install_linux(
    _downloaded_path: &std::path::Path,
    _cache_dir: &std::path::Path,
) -> Result<(), UpdateError> {
    Err(UpdateError::new(
        UpdateErrorKind::UnsupportedPlatform,
        "当前平台不支持 Linux 更新包",
    ))
}

#[cfg(target_os = "macos")]
fn find_app_bundle(directory: &std::path::Path) -> Option<std::path::PathBuf> {
    std::fs::read_dir(directory)
        .ok()?
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .find(|path| path.is_dir() && path.extension().is_some_and(|ext| ext == "app"))
}

fn unique_install_dir(
    cache_dir: &std::path::Path,
) -> Result<std::path::PathBuf, UpdateError> {
    let stamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_millis())
        .unwrap_or(0);
    let path = cache_dir.join(format!("install-{stamp}"));
    std::fs::create_dir_all(&path).map_err(|error| {
        UpdateError::new(
            UpdateErrorKind::Install,
            format!("创建更新安装目录失败：{error}"),
        )
    })?;
    Ok(path)
}

#[cfg(unix)]
fn spawn_install_script(
    script: &std::path::Path,
    args: &[String],
    log_path: &std::path::Path,
) -> Result<(), UpdateError> {
    let log = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(log_path)
        .map_err(|error| {
            UpdateError::new(
                UpdateErrorKind::Install,
                format!("打开更新日志失败：{error}"),
            )
        })?;
    let log_err = log.try_clone().map_err(|error| {
        UpdateError::new(
            UpdateErrorKind::Install,
            format!("复制更新日志句柄失败：{error}"),
        )
    })?;
    std::process::Command::new("/bin/sh")
        .arg(script)
        .args(args)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::from(log))
        .stderr(std::process::Stdio::from(log_err))
        .spawn()
        .map_err(|error| {
            UpdateError::new(
                UpdateErrorKind::Install,
                format!("启动更新 helper 失败：{error}"),
            )
        })?;
    Ok(())
}

#[cfg(not(unix))]
fn spawn_install_script(
    _script: &std::path::Path,
    _args: &[String],
    _log_path: &std::path::Path,
) -> Result<(), UpdateError> {
    Err(UpdateError::new(
        UpdateErrorKind::UnsupportedPlatform,
        "当前平台不支持脚本安装器",
    ))
}

#[cfg(test)]
mod updater_tests {
    use super::*;

    fn release(version: &str, zh: &str, en: &str) -> UpdateRelease {
        UpdateRelease {
            version: version.to_string(),
            published_at: None,
            notes: std::collections::BTreeMap::from([
                ("zh-CN".to_string(), zh.to_string()),
                ("en".to_string(), en.to_string()),
            ]),
        }
    }

    fn asset(kind: UpdateAssetKind) -> UpdateAsset {
        UpdateAsset {
            url: "https://example.com/update.bin".to_string(),
            sha256: "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"
                .to_string(),
            size: 128,
            kind,
        }
    }

    #[test]
    fn newer_release_selects_localized_notes_in_descending_order() {
        let manifest = UpdateManifest {
            schema: 1,
            version: "0.2.0".to_string(),
            published_at: None,
            notes: std::collections::BTreeMap::new(),
            releases: vec![
                release("0.1.10", "旧", "old"),
                release("0.2.0", "新", "new"),
                release("0.1.9", "当前", "current"),
            ],
            platforms: std::collections::BTreeMap::from([(
                "macos-aarch64".to_string(),
                asset(UpdateAssetKind::MacosAppTarGz),
            )]),
        };
        let check = validate_manifest(
            &manifest,
            &semver::Version::parse("0.1.9").unwrap(),
            "macos-aarch64",
            UpdateAssetKind::MacosAppTarGz,
        )
        .unwrap();
        let UpdateCheck::Available(update) = check else {
            panic!("expected update");
        };
        assert_eq!(update.releases.len(), 2);
        assert_eq!(update.releases[0].version, "0.2.0");
        assert_eq!(localized_note(&update.releases[0].notes, "zh-CN"), Some("新"));
        assert_eq!(localized_note(&update.releases[0].notes, "en"), Some("new"));
    }

    #[test]
    fn same_or_older_remote_version_is_up_to_date() {
        let manifest = UpdateManifest {
            schema: 1,
            version: "0.1.9".to_string(),
            published_at: None,
            notes: std::collections::BTreeMap::new(),
            releases: Vec::new(),
            platforms: std::collections::BTreeMap::from([(
                "linux-x86_64".to_string(),
                asset(UpdateAssetKind::LinuxDeb),
            )]),
        };
        assert_eq!(
            validate_manifest(
                &manifest,
                &semver::Version::parse("0.1.9").unwrap(),
                "linux-x86_64",
                UpdateAssetKind::LinuxDeb,
            )
            .unwrap(),
            UpdateCheck::UpToDate {
                current_version: "0.1.9".to_string()
            }
        );
    }

    #[test]
    fn ed25519_signature_rejects_tampered_manifest() {
        let public_key = base64::engine::general_purpose::STANDARD
            .decode("FM1FcW0Y45iNLlCEJnyreOe/rw+eqvuIypLaeJ+1ji0=")
            .unwrap();
        let signature = base64::engine::general_purpose::STANDARD
            .decode("aLZ5zPge4XET0lvNcNJK2xyzZrY8shsclkWbhATHuTPy+WfPOkLBqIvUutsWKGHjFkqmaPCvNY28ymQTdCjeBQ==")
            .unwrap();
        let signature_text = base64::engine::general_purpose::STANDARD.encode(signature);
        assert!(verify_manifest_signature(
            b"fluxdb-update-test-manifest",
            signature_text.as_bytes(),
            &public_key,
        )
        .is_ok());
        assert!(
            verify_manifest_signature(
                b"fluxdb-update-test-manifest-tampered",
                signature_text.as_bytes(),
                &public_key,
            )
            .is_err()
        );
    }

    #[test]
    fn ed25519_signature_accepts_base64_line_wrapping() {
        let public_key = base64::engine::general_purpose::STANDARD
            .decode("FM1FcW0Y45iNLlCEJnyreOe/rw+eqvuIypLaeJ+1ji0=")
            .unwrap();
        let single_line = "aLZ5zPge4XET0lvNcNJK2xyzZrY8shsclkWbhATHuTPy+WfPOkLBqIvUutsWKGHjFkqmaPCvNY28ymQTdCjeBQ==";
        let wrapped = format!("{}\r\n{}\n", &single_line[..76], &single_line[76..]);

        verify_manifest_signature(
            b"fluxdb-update-test-manifest",
            wrapped.as_bytes(),
            &public_key,
        )
        .expect("base64 line wrapping must be ignored");
    }
}
