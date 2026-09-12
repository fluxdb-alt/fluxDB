// 受控密钥引用模型（Redis / MySQL / PostgreSQL 共用）。
//
// 由 `redis_profile.rs` 机械迁出：原定义只依赖 `serde` 与 `std`，各档案在
// crate-root include 作用域下可见，无需在档案文件内重复定义。

/// 一条可拨号的不安全密钥引用。
///
/// - `key`：受控存储里的引用名（如 macOS Keychain 的 account），可安全落盘。
/// - `inline`：仅在内存中出现的一次性值（本次拨号使用），`#[serde(skip)]` 保证绝不写盘。
#[derive(Clone, Default, Eq, PartialEq, Serialize, Deserialize)]
pub struct SecretRef {
    /// 受控存储中的引用名；空表示没有引用。
    pub key: String,
    /// 内存中的受控值；落盘前由 storage 剥离，本字段不参与序列化。
    #[serde(skip)]
    pub inline: Option<String>,
}

// 手动 Debug：内联受控值一律打码，杜绝日志 / `{:?}` 泄漏明文密钥正文。
impl std::fmt::Debug for SecretRef {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SecretRef")
            .field("key", &self.key)
            .field("inline", &self.inline.as_ref().map(|_| "***"))
            .finish()
    }
}

impl SecretRef {
    /// 构造一个持有内联受控值的引用（仅内存）。
    pub fn inline(value: impl Into<String>) -> Self {
        Self {
            key: String::new(),
            inline: Some(value.into()),
        }
    }

    /// 构造一个指向受控存储槽位的引用。
    pub fn ref_key(key: impl Into<String>) -> Self {
        Self {
            key: key.into(),
            inline: None,
        }
    }

    /// 取用于拨号的值：优先内联受控值，其次引用名（引用通常在拨号前被 storage 解析回填）。
    pub fn value(&self) -> Option<&str> {
        self.inline.as_deref().or(if self.key.is_empty() { None } else { Some(self.key.as_str()) })
    }
}

#[cfg(test)]
mod secrets_tests {
    use super::*;

    #[test]
    fn debug_redacts_inline_secret() {
        let s = SecretRef::inline("hunter2");
        let debug = format!("{s:?}");
        assert!(!debug.contains("hunter2"), "Debug 泄漏内联密钥: {debug}");
        assert!(debug.contains("***"));
    }
}
