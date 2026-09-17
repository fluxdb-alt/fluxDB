// 系统凭据后端（方案 §4.2 跨平台）。
//
// 统一抽象读/写/删系统凭据，按平台提供实现：
// - macOS：保留既有 `security` CLI（Keychain），保持历史条目兼容与 credential_ref 所有权语义。
// - Windows：Credential Manager（advapi32 CredWriteW/CredReadW/CredDeleteW/CredFree）。
// - Linux（仅编译此文件时为该平台）：Secret Service（GNOME Keyring / KWallet 等），
//   经 keyring 4 默认 feature(v1) 里的 zbus-secret-service 实现（纯 Rust D-Bus，无 C 原生依赖）。
// - 内存后端：仅测试用（InMemoryBackend），隔离、不访问真实用户密码库，可注入故障。
//
// 移除非 macOS 的"写入假成功"：write 失败必须返回错误，不得 Ok(())。

use std::fmt::Debug;

// 测试注入覆盖：一个共享的 thread_local（set/clear/backend 同源，避免并行污染）。
#[cfg(any(test, feature = "test-util"))]
std::thread_local! {
    pub static TEST_OVERRIDE: std::cell::RefCell<Option<std::sync::Arc<dyn CredentialBackend>>> =
        const { std::cell::RefCell::new(None) };
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CredentialError {
    /// 目标条目不存在（读）。
    NotFound,
    /// 凭据库被锁定（如 Keychain 未解锁 / Secret Service 锁定）。
    Locked,
    /// 凭据服务进程不可用/未启动（如 Linux 无 Secret Service 提供方）。
    Unavailable,
    /// 权限拒绝 / 拒绝授权。
    Denied,
    /// 其他写入/平台失败。
    Failure(String),
}

impl std::fmt::Display for CredentialError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NotFound => write!(f, "凭据条目不存在"),
            Self::Locked => write!(f, "系统凭据库已锁定"),
            Self::Unavailable => write!(f, "系统凭据服务不可用"),
            Self::Denied => write!(f, "访问系统凭据被拒绝"),
            Self::Failure(msg) => write!(f, "系统凭据操作失败: {msg}"),
        }
    }
}

impl std::error::Error for CredentialError {}

/// 凭据后端的统一接口。错误细分用于 UI 反馈区分。
pub trait CredentialBackend: Debug + Send + Sync {
    /// 读取凭据；条目不存在返回 Ok(None)。
    fn read(&self, account: &str) -> std::result::Result<Option<String>, CredentialError>;
    /// 写入/更新凭据；失败必须返回错误（不得假成功）。
    fn write(&self, account: &str, secret: &str) -> std::result::Result<(), CredentialError>;
    /// 删除凭据；条目不存在视为成功（幂等）。
    fn delete(&self, account: &str) -> std::result::Result<(), CredentialError>;
}

/// 平台后端工厂：按当前编译平台选择真实后端（无状态，可安全单例化）。
/// 测试经 `set_test_backend` 在当前线程覆盖为隔离内存后端，不使用全局函数替换。
fn platform_backend() -> std::sync::Arc<dyn CredentialBackend> {
    #[cfg(target_os = "macos")]
    return std::sync::Arc::new(MacKeychainBackend);
    #[cfg(target_os = "windows")]
    return std::sync::Arc::new(WindowsCredManagerBackend);
    #[cfg(target_os = "linux")]
    return std::sync::Arc::new(LinuxSecretServiceBackend);

    // 非目标平台兜底：明确不可用，不让调用方误以为凭据已保存。
    #[allow(unreachable_code)]
    std::sync::Arc::new(UnavailableBackend)
}

/// 测试注入的后端覆盖（thread_local，每测试线程独立，避免并行测试互相污染）。
/// 仅在测试或 test-util feature 下编译；使用隔离的 InMemoryBackend，不访问真实密码库。
#[cfg(any(test, feature = "test-util"))]
pub fn set_test_backend(backend: std::sync::Arc<dyn CredentialBackend>) {
    TEST_OVERRIDE.with(|cell| *cell.borrow_mut() = Some(backend));
}

#[cfg(any(test, feature = "test-util"))]
pub fn clear_test_backend() {
    TEST_OVERRIDE.with(|cell| *cell.borrow_mut() = None);
}

/// 当前线程是否有测试 override（供 FileStorage 决定是否绕过"非系统 root 短路"）。
#[cfg(any(test, feature = "test-util"))]
pub fn test_override_active() -> bool {
    TEST_OVERRIDE.with(|cell| cell.borrow().is_some())
}

#[cfg(not(any(test, feature = "test-util")))]
pub fn test_override_active() -> bool {
    false
}

// ---- 配置提交失败注入（测试用，小范围可控）----
// 让"凭据全部写入成功、但 SQLite 配置提交失败"可复现，验证跨层补偿不会误删旧凭据。

#[cfg(any(test, feature = "test-util"))]
std::thread_local! {
    pub static FAIL_NEXT_COMMIT: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

/// 触发下一次配置提交失败（当前测试线程）。
#[cfg(any(test, feature = "test-util"))]
pub fn fail_next_commit() {
    FAIL_NEXT_COMMIT.with(|c| c.set(true));
}

/// 配置提交处消费一次失败标记；返回 true 表示本次应失败。生产编译为空实现。
pub fn consume_commit_failure() -> bool {
    #[cfg(any(test, feature = "test-util"))]
    {
        FAIL_NEXT_COMMIT.with(|c| {
            if c.get() {
                c.set(false);
                true
            } else {
                false
            }
        })
    }
    #[cfg(not(any(test, feature = "test-util")))]
    {
        false
    }
}

/// 进程级凭据后端。默认返回平台后端（不可变、无状态，进程内单例安全）；
/// 测试可用 `set_test_backend` 在当前线程注入隔离内存后端。真实运行无注入。
pub fn backend() -> std::sync::Arc<dyn CredentialBackend> {
    #[cfg(any(test, feature = "test-util"))]
    {
        if let Some(b) = TEST_OVERRIDE.with(|cell| cell.borrow().clone()) {
            return b;
        }
    }
    use std::sync::OnceLock;
    static BACKEND: OnceLock<std::sync::Arc<dyn CredentialBackend>> = OnceLock::new();
    BACKEND.get_or_init(platform_backend).clone()
}

/// 将凭据错误映射为 storage 统一 Error（供上层传播给 UI/日志）：
/// Locked/Denied -> Permission；NotFound / Unavailable / Failure -> Internal。
pub fn to_storage_error(err: &CredentialError) -> crate::Error {
    let kind = match err {
        CredentialError::Locked | CredentialError::Denied => crate::ErrorKind::Permission,
        CredentialError::NotFound | CredentialError::Unavailable | CredentialError::Failure(_) => {
            crate::ErrorKind::Internal
        }
    };
    crate::Error::new(kind, err.to_string())
}

// ---------------------------------------------------------------------------
// 平台实现
// ---------------------------------------------------------------------------

#[cfg(target_os = "macos")]
#[derive(Debug)]
pub struct MacKeychainBackend;

#[cfg(target_os = "windows")]
#[derive(Debug)]
pub struct WindowsCredManagerBackend;

#[cfg(target_os = "linux")]
#[derive(Debug)]
pub struct LinuxSecretServiceBackend;

/// 非三目标平台的后端（或平台 fn 底部兜底）：明确不可用，不让调用方误以为凭据已保存。
#[derive(Debug)]
pub struct UnavailableBackend;

impl CredentialBackend for UnavailableBackend {
    fn read(&self, _: &str) -> std::result::Result<Option<String>, CredentialError> {
        Err(CredentialError::Unavailable)
    }
    fn write(&self, _: &str, _: &str) -> std::result::Result<(), CredentialError> {
        Err(CredentialError::Unavailable)
    }
    fn delete(&self, _: &str) -> std::result::Result<(), CredentialError> {
        Err(CredentialError::Unavailable)
    }
}

// ---- 内存后端（测试用）----

/// 内存后端：测试用隔离凭据库，不访问任何真实用户密码；可按账号前缀注入写失败。
#[derive(Debug, Default)]
pub struct InMemoryBackend {
    store: std::sync::Mutex<std::collections::BTreeMap<String, String>>,
    fail_write_prefix: std::sync::Mutex<Vec<String>>,
    fail_next_write_accounts: std::sync::Mutex<Vec<String>>,
}

impl InMemoryBackend {
    pub fn new() -> Self {
        Self::default()
    }
    /// 注入：写入账号前缀匹配时失败（用于"多槽位部分失败"测试）。
    pub fn fail_writes_with_prefix(&self, prefix: &str) {
        self.fail_write_prefix
            .lock()
            .unwrap()
            .push(prefix.to_string());
    }
    /// 注入：指定账号的下一次写入失败一次，随后自动恢复（用于验证正式键写入失败后的回滚）。
    pub fn fail_next_write(&self, account: &str) {
        self.fail_next_write_accounts
            .lock()
            .unwrap()
            .push(account.to_string());
    }
    fn write_fails(&self, account: &str) -> bool {
        if self
            .fail_write_prefix
            .lock()
            .unwrap()
            .iter()
            .any(|p| account.starts_with(p))
        {
            return true;
        }
        let mut accounts = self.fail_next_write_accounts.lock().unwrap();
        if let Some(index) = accounts.iter().position(|item| item == account) {
            accounts.remove(index);
            return true;
        }
        false
    }
    /// 测试断言辅助：直接读取存储内容。
    pub fn peek(&self, account: &str) -> Option<String> {
        self.store.lock().unwrap().get(account).cloned()
    }
}

impl CredentialBackend for InMemoryBackend {
    fn read(&self, account: &str) -> std::result::Result<Option<String>, CredentialError> {
        Ok(self.store.lock().unwrap().get(account).cloned())
    }
    fn write(&self, account: &str, secret: &str) -> std::result::Result<(), CredentialError> {
        if self.write_fails(account) {
            return Err(CredentialError::Failure(format!("注入写入失败: {account}")));
        }
        self.store
            .lock()
            .unwrap()
            .insert(account.to_string(), secret.to_string());
        Ok(())
    }
    fn delete(&self, account: &str) -> std::result::Result<(), CredentialError> {
        self.store.lock().unwrap().remove(account);
        Ok(())
    }
}

// 平台实现放在同模块：mac（security CLI）、windows（CredManager）、linux（keyring）。
#[cfg(target_os = "linux")]
mod impl_linux;
#[cfg(target_os = "macos")]
mod impl_macos;
#[cfg(target_os = "windows")]
mod impl_windows;
