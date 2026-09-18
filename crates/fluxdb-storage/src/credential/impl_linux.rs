// Linux Secret Service 后端（方案 §4.2：Linux 使用可持久化的 Secret Service）。
//
// 经 keyring crate 的 sync-secret-service feature（Secret Service API over D-Bus，
// zbus 纯 Rust，无 C 原生依赖）。服务提供方为 GNOME Keyring / KWallet 等。
// 错误映射：NoEntry -> NotFound；NoStorageAccess -> Unavailable/Locked；其他 -> Failure。
// 注意：本后端仅在 Linux 存在 Secret Service 提供方时可用；缺失时返回 Unavailable，
// 不得返回假成功。
use super::{CredentialBackend, CredentialError, LinuxSecretServiceBackend};

// keyring 的默认 credential 类型名（等同于 macOS 的 "default" 或空 service 名）。
const KEYRING_SERVICE: &str = "fluxdb";

impl CredentialBackend for LinuxSecretServiceBackend {
    fn read(&self, account: &str) -> Result<Option<String>, CredentialError> {
        let entry = match keyring::Entry::new(KEYRING_SERVICE, account) {
            Ok(e) => e,
            Err(e) => return Err(map_err(e)),
        };
        entry.get_password().map(Some).map_err(|e| {
            if matches!(e, keyring::Error::NoEntry) {
                CredentialError::NotFound
            } else {
                map_err(e)
            }
        })
    }

    fn write(&self, account: &str, secret: &str) -> Result<(), CredentialError> {
        let entry = keyring::Entry::new(KEYRING_SERVICE, account).map_err(map_err)?;
        entry.set_password(secret).map_err(map_err)
    }

    fn delete(&self, account: &str) -> Result<(), CredentialError> {
        let entry = match keyring::Entry::new(KEYRING_SERVICE, account) {
            Ok(e) => e,
            Err(e) => return Err(map_err(e)),
        };
        match entry.delete_credential() {
            Ok(()) => Ok(()),
            Err(keyring::Error::NoEntry) => Ok(()), // 幂等删除
            Err(e) => Err(map_err(e)),
        }
    }
}

fn map_err(e: keyring::Error) -> CredentialError {
    use keyring::Error;
    match e {
        Error::NoEntry => CredentialError::NotFound,
        // 锁库 / Secret Service 不可访问（未解锁 / 无提供方 / 无默认 store）。
        Error::NoStorageAccess(_) | Error::NoDefaultStore => CredentialError::Unavailable,
        _ => CredentialError::Failure(e.to_string()),
    }
}
