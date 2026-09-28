// 连接凭据后端：生产写入仅使用 SQLite AES-256-GCM 密文表。
// 系统凭据库不再被访问；旧密码需由用户重输一次。
use std::fmt::Debug;

#[cfg(any(test, feature = "test-util"))]
std::thread_local! {
    pub static TEST_OVERRIDE: std::cell::RefCell<Option<std::sync::Arc<dyn CredentialBackend>>> =
        const { std::cell::RefCell::new(None) };
}

#[derive(Debug, Clone, PartialEq, Eq)]
#[allow(dead_code)]
pub enum CredentialError {
    NotFound,
    Locked,
    Unavailable,
    Denied,
    Failure(String),
}

impl std::fmt::Display for CredentialError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NotFound => write!(f, "凭据条目不存在"),
            Self::Locked => write!(f, "凭据库已锁定"),
            Self::Unavailable => write!(f, "凭据存储不可用"),
            Self::Denied => write!(f, "凭据访问被拒绝"),
            Self::Failure(msg) => write!(f, "凭据操作失败: {msg}"),
        }
    }
}
impl std::error::Error for CredentialError {}

pub trait CredentialBackend: Debug + Send + Sync {
    fn read(&self, connection_id: u64, kind: &str) -> Result<Option<String>, CredentialError>;
    fn write(&self, connection_id: u64, kind: &str, secret: &str) -> Result<(), CredentialError>;
    fn delete(&self, connection_id: u64, kind: &str) -> Result<(), CredentialError>;
}

#[cfg(any(test, feature = "test-util"))]
pub fn set_test_backend(backend: std::sync::Arc<dyn CredentialBackend>) {
    TEST_OVERRIDE.with(|cell| *cell.borrow_mut() = Some(backend));
}
#[cfg(any(test, feature = "test-util"))]
pub fn clear_test_backend() {
    TEST_OVERRIDE.with(|cell| *cell.borrow_mut() = None);
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

/// 连接加密凭据后端；
/// 测试可用 `set_test_backend` 在当前线程注入隔离内存后端。真实运行无注入。
pub fn backend(root: &std::path::Path) -> std::sync::Arc<dyn CredentialBackend> {
    #[cfg(any(test, feature = "test-util"))]
    {
        if let Some(b) = TEST_OVERRIDE.with(|cell| cell.borrow().clone()) {
            return b;
        }
    }
    std::sync::Arc::new(crate::connection_secrets::SqliteEncryptedBackend::new(
        root.to_path_buf(),
    ))
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

#[cfg(any(test, feature = "test-util"))]
#[derive(Debug)]
pub struct UnavailableBackend;

#[cfg(any(test, feature = "test-util"))]
impl CredentialBackend for UnavailableBackend {
    fn read(&self, _: u64, _: &str) -> Result<Option<String>, CredentialError> {
        Err(CredentialError::Unavailable)
    }
    fn write(&self, _: u64, _: &str, _: &str) -> Result<(), CredentialError> {
        Err(CredentialError::Unavailable)
    }
    fn delete(&self, _: u64, _: &str) -> Result<(), CredentialError> {
        Err(CredentialError::Unavailable)
    }
}

// ---- 内存后端（测试用）----

/// 内存后端：测试用隔离凭据库，不访问任何真实用户密码；可按账号前缀注入写失败。
/// 仅测试 / test-util 下编译，生产包不携带，避免"未构造"死代码警告。
#[cfg(any(test, feature = "test-util"))]
#[derive(Debug, Default)]
pub struct InMemoryBackend {
    store: std::sync::Mutex<std::collections::BTreeMap<(u64, String), String>>,
    fail_write_prefix: std::sync::Mutex<Vec<String>>,
    fail_next_write_accounts: std::sync::Mutex<Vec<String>>,
}

#[cfg(any(test, feature = "test-util"))]
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
    pub fn peek(&self, connection_id: u64, kind: &str) -> Option<String> {
        self.store
            .lock()
            .unwrap()
            .get(&(connection_id, kind.to_string()))
            .cloned()
    }
}

#[cfg(any(test, feature = "test-util"))]
impl CredentialBackend for InMemoryBackend {
    fn read(&self, id: u64, kind: &str) -> Result<Option<String>, CredentialError> {
        Ok(self.peek(id, kind))
    }
    fn write(&self, id: u64, kind: &str, secret: &str) -> Result<(), CredentialError> {
        let account = format!("{id}:{kind}");
        if self.write_fails(&account) {
            return Err(CredentialError::Failure(format!("注入写入失败: {account}")));
        }
        self.store
            .lock()
            .unwrap()
            .insert((id, kind.to_string()), secret.to_string());
        Ok(())
    }
    fn delete(&self, id: u64, kind: &str) -> Result<(), CredentialError> {
        self.store.lock().unwrap().remove(&(id, kind.to_string()));
        Ok(())
    }
}
