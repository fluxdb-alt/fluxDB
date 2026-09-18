// Windows Credential Manager 后端（方案 §4.2：Windows 使用系统凭据存储）。
// 用 generic credential（CRED_TYPE_GENERIC），persistence=LOCAL_MACHINE。
// 错误码区分：ERROR_NOT_FOUND(1168) -> NotFound；ACCESS_DENIED/LOGON_FAILURE -> Denied。
use super::{CredentialBackend, CredentialError, WindowsCredManagerBackend};

const NOT_FOUND: u32 = 1168; // ERROR_NOT_FOUND

impl CredentialBackend for WindowsCredManagerBackend {
    fn read(&self, account: &str) -> Result<Option<String>, CredentialError> {
        unsafe { cred_read(account) }
    }
    fn write(&self, account: &str, secret: &str) -> Result<(), CredentialError> {
        unsafe { cred_write(account, secret) }
    }
    fn delete(&self, account: &str) -> Result<(), CredentialError> {
        unsafe { cred_delete(account) }
    }
}

use windows::Win32::Foundation::GetLastError;
use windows::Win32::Security::Credentials::{
    CRED_PERSIST_LOCAL_MACHINE, CRED_TYPE_GENERIC, CREDENTIALW, CredDeleteW, CredFree, CredReadW,
    CredWriteW,
};
use windows::core::PCWSTR;

fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
}

fn last_error_code() -> u32 {
    unsafe { GetLastError() }.0 as u32
}

unsafe fn cred_read(account: &str) -> Result<Option<String>, CredentialError> {
    let target = wide(account);
    let mut cred_ptr: *mut CREDENTIALW = std::ptr::null_mut();
    // 显式 unsafe 块：读取系统凭据（unsafe_op_in_unsafe_fn，edition 2024）。
    let result = unsafe { CredReadW(PCWSTR(target.as_ptr()), CRED_TYPE_GENERIC, 0, &mut cred_ptr) };
    if result.is_ok() {
        let out = if cred_ptr.is_null() {
            Ok(None)
        } else {
            // 解引用凭据结构体并从其原始字节切片解码（都在显式块内）。
            let (blob, blob_len) = unsafe {
                let cred = &*cred_ptr;
                (cred.CredentialBlob, cred.CredentialBlobSize as usize)
            };
            let blob_bytes = unsafe { std::slice::from_raw_parts(blob, blob_len) };
            // Credential Manager 保存调用方写入的原始字节（本项目写 UTF-8），按 UTF-8 解码。
            Ok(Some(String::from_utf8_lossy(blob_bytes).into_owned()))
        };
        if !cred_ptr.is_null() {
            unsafe { CredFree(cred_ptr as *const core::ffi::c_void) };
        }
        return out;
    }
    let code = last_error_code();
    if code == NOT_FOUND {
        Ok(None)
    } else {
        Err(classify(code))
    }
}

unsafe fn cred_write(account: &str, secret: &str) -> Result<(), CredentialError> {
    let mut target = wide(account);
    let mut blob_owned = secret.as_bytes().to_vec();
    let mut cred: CREDENTIALW = CREDENTIALW::default();
    // 以下写入凭据结构体的指针字段并调用 CredWriteW，均需显式 unsafe 块。
    unsafe {
        cred.Type = CRED_TYPE_GENERIC;
        cred.TargetName = windows::core::PWSTR(target.as_mut_ptr());
        cred.CredentialBlobSize = blob_owned.len() as u32;
        cred.CredentialBlob = blob_owned.as_mut_ptr();
        cred.Persist = CRED_PERSIST_LOCAL_MACHINE;
        if CredWriteW(&cred, 0).is_ok() {
            Ok(())
        } else {
            Err(classify(last_error_code()))
        }
    }
}

unsafe fn cred_delete(account: &str) -> Result<(), CredentialError> {
    let target = wide(account);
    // 显式 unsafe 块：调用系统凭据删除 API。
    let result = unsafe { CredDeleteW(PCWSTR(target.as_ptr()), CRED_TYPE_GENERIC, 0) };
    if result.is_ok() {
        return Ok(());
    }
    let code = last_error_code();
    if code == NOT_FOUND {
        Ok(()) // 幂等删除
    } else {
        Err(classify(code))
    }
}

fn classify(code: u32) -> CredentialError {
    match code {
        5 | 1300 | 1314 | 1326 => CredentialError::Denied, // ACCESS_DENIED/NOT_ALLOWED/PRIVILEGE_NOT_HELD/LOGON_FAILURE
        _ => CredentialError::Failure(format!("Win32 错误码 {code}")),
    }
}
