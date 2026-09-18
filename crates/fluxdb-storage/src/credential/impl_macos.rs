// macOS Keychain 后端：沿用既有 `security` CLI（generic password），
// 保持历史条目兼容（service=com.fluxdb.connection, account=<ref>）。
use std::process::Command;

use super::{CredentialBackend, CredentialError, MacKeychainBackend};

const KEYCHAIN_SERVICE: &str = "com.fluxdb.connection";

impl CredentialBackend for MacKeychainBackend {
    fn read(&self, account: &str) -> std::result::Result<Option<String>, CredentialError> {
        let output = Command::new("security")
            .args([
                "find-generic-password",
                "-s",
                KEYCHAIN_SERVICE,
                "-a",
                account,
                "-w",
            ])
            .output()
            .map_err(|_| CredentialError::Unavailable)?;
        if output.status.success() {
            let pw = String::from_utf8_lossy(&output.stdout).trim().to_string();
            return Ok(if pw.is_empty() { None } else { Some(pw) });
        }
        // security 失败码分类：errSecItemNotFound -> 不存在。
        let msg = String::from_utf8_lossy(&output.stderr);
        if msg.contains("errSecItemNotFound") {
            Ok(None)
        } else {
            Err(classify_security_err(&msg))
        }
    }

    fn write(&self, account: &str, secret: &str) -> std::result::Result<(), CredentialError> {
        let out = Command::new("security")
            .args([
                "add-generic-password",
                "-s",
                KEYCHAIN_SERVICE,
                "-a",
                account,
                "-w",
                secret,
                "-U",
            ])
            .output()
            .map_err(|_| CredentialError::Unavailable)?;
        if out.status.success() {
            Ok(())
        } else {
            Err(classify_security_err(&String::from_utf8_lossy(&out.stderr)))
        }
    }

    fn delete(&self, account: &str) -> std::result::Result<(), CredentialError> {
        let out = Command::new("security")
            .args([
                "delete-generic-password",
                "-s",
                KEYCHAIN_SERVICE,
                "-a",
                account,
            ])
            .output()
            .map_err(|_| CredentialError::Unavailable)?;
        if out.status.success() {
            Ok(())
        } else {
            let msg = String::from_utf8_lossy(&out.stderr);
            if msg.contains("errSecItemNotFound") {
                Ok(()) // 幂等删除
            } else {
                Err(classify_security_err(&msg))
            }
        }
    }
}

fn classify_security_err(msg: &str) -> CredentialError {
    if msg.contains("errSecAuthFailed") || msg.contains("User interaction is not allowed") {
        CredentialError::Denied
    } else if msg.contains("errSecInteractionNotAllowed") || msg.contains("Error -25293") {
        CredentialError::Locked
    } else {
        CredentialError::Failure(msg.to_string())
    }
}
