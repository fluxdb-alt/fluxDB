//! 连接凭据：以 (connection_id, secret_kind) 为主键保存 AES-256-GCM 密文。
//! 默认密钥内置，可只在 config.toml 中覆盖；不读取旧系统凭据或旧加密文件。

use std::fs;
use std::path::PathBuf;

use aes_gcm::aead::{Aead, KeyInit, Payload};
use aes_gcm::{Aes256Gcm, Nonce};
use rand::RngCore;
use rusqlite::{Connection, OptionalExtension, params};

use crate::credential::{CredentialBackend, CredentialError};
use crate::sqlite::sqlite_error;

pub(crate) const DATABASE_PASSWORD: &str = "database_password";
pub(crate) const SSH_PASSWORD: &str = "ssh_password";
pub(crate) const SSH_PASSPHRASE: &str = "ssh_passphrase";
pub(crate) const PROXY_PASSWORD: &str = "proxy_password";
pub(crate) const ENDPOINT_URI: &str = "endpoint_uri";
pub(crate) const URL_PARAMS: &str = "url_params";

const CREATE_SQL: &str = "CREATE TABLE IF NOT EXISTS connection_secrets (
    connection_id INTEGER NOT NULL,
    secret_kind TEXT NOT NULL,
    version INTEGER NOT NULL,
    nonce BLOB NOT NULL,
    ciphertext BLOB NOT NULL,
    PRIMARY KEY(connection_id, secret_kind)
);";
// 内置密钥只避免 SQLite 明文，不能抵御同时取得程序与数据库的攻击者。
const DEFAULT_KEY: [u8; 32] = *b"fluxdb-default-aes256-key-v1-001";
const CONFIG_KEY: &str = "connection_secret_key";
const VERSION: i64 = 1;

pub(crate) fn ensure_schema(conn: &Connection) -> fluxdb_core::Result<()> {
    let mut stmt = conn
        .prepare("PRAGMA table_info(connection_secrets)")
        .map_err(sqlite_error)?;
    let columns = stmt
        .query_map([], |row| row.get::<_, String>(1))
        .map_err(sqlite_error)?;
    let mut old_account_column = false;
    for col in columns {
        old_account_column |= col.map_err(sqlite_error)? == "account";
    }
    if old_account_column {
        // 不迁移旧密文；仅改名留存，防止自动升级时不可逆删除用户数据。
        conn.execute_batch("ALTER TABLE connection_secrets RENAME TO connection_secrets_obsolete")
            .map_err(sqlite_error)?;
    }
    conn.execute_batch(CREATE_SQL).map_err(sqlite_error)
}

#[derive(Debug)]
pub(crate) struct SqliteEncryptedBackend {
    root: PathBuf,
}

impl SqliteEncryptedBackend {
    pub(crate) fn new(root: PathBuf) -> Self {
        Self { root }
    }

    fn open(&self) -> Result<Connection, CredentialError> {
        let conn = crate::sqlite::open(&self.root).map_err(storage_error)?;
        crate::sqlite::create_schema(&conn).map_err(storage_error)?;
        Ok(conn)
    }

    fn key(&self) -> Result<[u8; 32], CredentialError> {
        let text = match fs::read_to_string(self.root.join("config.toml")) {
            Ok(text) => text,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(DEFAULT_KEY),
            Err(e) => return Err(CredentialError::Failure(format!("读取密钥配置失败: {e}"))),
        };
        let config: toml::Value = toml::from_str(&text)
            .map_err(|_| CredentialError::Failure("config.toml 格式无效".into()))?;
        let Some(value) = config.get(CONFIG_KEY) else {
            return Ok(DEFAULT_KEY);
        };
        let hex = value.as_str().ok_or_else(invalid_key)?;
        decode_hex(hex)?.try_into().map_err(|_| invalid_key())
    }

    fn aad(connection_id: u64, secret_kind: &str) -> Vec<u8> {
        format!("fluxdb-connection-secret\0v1\0{connection_id}\0{secret_kind}").into_bytes()
    }

    fn read_new(
        &self,
        conn: &Connection,
        connection_id: u64,
        secret_kind: &str,
    ) -> Result<Option<String>, CredentialError> {
        let row: Option<(i64, Vec<u8>, Vec<u8>)> = conn.query_row(
            "SELECT version, nonce, ciphertext FROM connection_secrets WHERE connection_id = ?1 AND secret_kind = ?2",
            params![connection_id as i64, secret_kind],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        ).optional().map_err(db_error)?;
        let Some((version, nonce, ciphertext)) = row else {
            return Ok(None);
        };
        if version != VERSION || nonce.len() != 12 {
            return Err(CredentialError::Failure("连接凭据密文格式不识别".into()));
        }
        let key = self.key()?;
        let cipher = Aes256Gcm::new_from_slice(&key).map_err(|_| invalid_key())?;
        let plaintext = cipher
            .decrypt(
                Nonce::from_slice(&nonce),
                Payload {
                    msg: &ciphertext,
                    aad: &Self::aad(connection_id, secret_kind),
                },
            )
            .map_err(|_| {
                CredentialError::Failure("连接凭据认证失败（密钥错误或密文损坏）".into())
            })?;
        String::from_utf8(plaintext)
            .map(Some)
            .map_err(|_| CredentialError::Failure("连接凭据解密结果不是 UTF-8".into()))
    }
}

impl CredentialBackend for SqliteEncryptedBackend {
    fn read(
        &self,
        connection_id: u64,
        secret_kind: &str,
    ) -> Result<Option<String>, CredentialError> {
        self.read_new(&self.open()?, connection_id, secret_kind)
    }

    fn write(
        &self,
        connection_id: u64,
        secret_kind: &str,
        secret: &str,
    ) -> Result<(), CredentialError> {
        let conn = self.open()?;
        let key = self.key()?;
        // config.toml 改密钥后先认证现有密文，拒绝用错误密钥覆盖。
        let sample: Option<(u64, String, Vec<u8>, Vec<u8>)> = conn.query_row(
            "SELECT connection_id, secret_kind, nonce, ciphertext FROM connection_secrets WHERE version = 1 LIMIT 1", [],
            |r| Ok((r.get::<_, i64>(0)? as u64, r.get(1)?, r.get(2)?, r.get(3)?)),
        ).optional().map_err(db_error)?;
        let cipher = Aes256Gcm::new_from_slice(&key).map_err(|_| invalid_key())?;
        if let Some((id, kind, nonce, ciphertext)) = sample {
            if nonce.len() != 12
                || cipher
                    .decrypt(
                        Nonce::from_slice(&nonce),
                        Payload {
                            msg: &ciphertext,
                            aad: &Self::aad(id, &kind),
                        },
                    )
                    .is_err()
            {
                return Err(CredentialError::Failure(
                    "当前 config.toml 密钥无法解密已有凭据，已拒绝覆盖".into(),
                ));
            }
        }
        let mut nonce = [0; 12];
        rand::thread_rng().fill_bytes(&mut nonce);
        let ciphertext = cipher
            .encrypt(
                Nonce::from_slice(&nonce),
                Payload {
                    msg: secret.as_bytes(),
                    aad: &Self::aad(connection_id, secret_kind),
                },
            )
            .map_err(|_| CredentialError::Failure("连接凭据加密失败".into()))?;
        conn.execute("INSERT INTO connection_secrets (connection_id, secret_kind, version, nonce, ciphertext)
            VALUES (?1, ?2, ?3, ?4, ?5)
            ON CONFLICT(connection_id, secret_kind) DO UPDATE SET version=excluded.version,
                nonce=excluded.nonce, ciphertext=excluded.ciphertext",
            params![connection_id as i64, secret_kind, VERSION, nonce.as_slice(), ciphertext],
        ).map_err(db_error)?;
        Ok(())
    }

    fn delete(&self, connection_id: u64, secret_kind: &str) -> Result<(), CredentialError> {
        self.open()?
            .execute(
                "DELETE FROM connection_secrets WHERE connection_id=?1 AND secret_kind=?2",
                params![connection_id as i64, secret_kind],
            )
            .map_err(db_error)?;
        Ok(())
    }
}

fn decode_hex(text: &str) -> Result<Vec<u8>, CredentialError> {
    let chunks = text.as_bytes().chunks_exact(2);
    if !chunks.remainder().is_empty() {
        return Err(CredentialError::Failure("凭据 hex 格式无效".into()));
    }
    chunks
        .map(|pair| {
            let pair = std::str::from_utf8(pair)
                .map_err(|_| CredentialError::Failure("凭据 hex 格式无效".into()))?;
            u8::from_str_radix(pair, 16)
                .map_err(|_| CredentialError::Failure("凭据 hex 格式无效".into()))
        })
        .collect()
}
fn invalid_key() -> CredentialError {
    CredentialError::Failure("config.toml 中 connection_secret_key 必须为 64 位十六进制字符".into())
}
fn db_error(error: rusqlite::Error) -> CredentialError {
    CredentialError::Failure(format!("连接凭据数据库操作失败: {error}"))
}
fn storage_error(error: fluxdb_core::Error) -> CredentialError {
    CredentialError::Failure(error.to_string())
}
#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn root(tag: &str) -> PathBuf {
        let n = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        std::env::temp_dir().join(format!("fluxdb-secret-{tag}-{}-{n}", std::process::id()))
    }
    #[test]
    fn rows_use_two_part_key_and_random_nonce() {
        let root = root("roundtrip");
        let backend = SqliteEncryptedBackend::new(root.clone());
        backend
            .write(2, DATABASE_PASSWORD, "sensitive-pass-123")
            .unwrap();
        backend.write(2, SSH_PASSWORD, "ssh-secret").unwrap();
        let conn = backend.open().unwrap();
        let (nonce, ciphertext): (Vec<u8>, Vec<u8>) = conn.query_row(
            "SELECT nonce, ciphertext FROM connection_secrets WHERE connection_id=2 AND secret_kind='database_password'", [],
            |r| Ok((r.get(0)?, r.get(1)?)),
        ).unwrap();
        assert_eq!(nonce.len(), 12);
        assert!(!ciphertext.windows(18).any(|w| w == b"sensitive-pass-123"));
        backend
            .write(2, DATABASE_PASSWORD, "sensitive-pass-123")
            .unwrap();
        let new_nonce: Vec<u8> = conn.query_row("SELECT nonce FROM connection_secrets WHERE connection_id=2 AND secret_kind='database_password'", [], |r| r.get(0)).unwrap();
        assert_ne!(nonce, new_nonce);
        assert_eq!(
            SqliteEncryptedBackend::new(root.clone())
                .read(2, DATABASE_PASSWORD)
                .unwrap()
                .as_deref(),
            Some("sensitive-pass-123")
        );
        assert_eq!(
            backend.read(2, SSH_PASSWORD).unwrap().as_deref(),
            Some("ssh-secret")
        );
        assert!(!root.join("connection_secrets.key").exists());
    }

    #[test]
    fn config_key_change_cannot_overwrite_existing_ciphertext() {
        let root = root("custom-key");
        fs::create_dir_all(&root).unwrap();
        let key1 = "11".repeat(32);
        let key2 = "22".repeat(32);
        fs::write(
            root.join("config.toml"),
            format!("connection_secret_key = \"{key1}\"\n"),
        )
        .unwrap();
        let backend = SqliteEncryptedBackend::new(root.clone());
        backend.write(3, DATABASE_PASSWORD, "secret").unwrap();
        fs::write(
            root.join("config.toml"),
            format!("connection_secret_key = \"{key2}\"\n"),
        )
        .unwrap();
        assert!(backend.read(3, DATABASE_PASSWORD).is_err());
        assert!(backend.write(3, DATABASE_PASSWORD, "replacement").is_err());
        fs::write(
            root.join("config.toml"),
            format!("connection_secret_key = \"{key1}\"\n"),
        )
        .unwrap();
        assert_eq!(
            backend.read(3, DATABASE_PASSWORD).unwrap().as_deref(),
            Some("secret")
        );
    }

    #[test]
    fn old_account_table_is_ignored_and_connection_ref_column_removed() {
        let root = root("old-schema");
        let conn = crate::sqlite::open(&root).unwrap();
        conn.execute_batch("CREATE TABLE connections (id INTEGER PRIMARY KEY, credential_ref TEXT);
            INSERT INTO connections VALUES (7, 'old.account');
            CREATE TABLE connection_secrets (account TEXT PRIMARY KEY, version INTEGER NOT NULL, nonce BLOB NOT NULL, ciphertext BLOB NOT NULL);
            INSERT INTO connection_secrets VALUES ('old.account', 1, X'000000000000000000000000', X'1234');").unwrap();
        drop(conn);
        let backend = SqliteEncryptedBackend::new(root);
        assert!(backend.read(7, DATABASE_PASSWORD).unwrap().is_none());
        backend.write(7, DATABASE_PASSWORD, "new-secret").unwrap();
        assert_eq!(
            backend.read(7, DATABASE_PASSWORD).unwrap().as_deref(),
            Some("new-secret")
        );
        let conn = backend.open().unwrap();
        let old_ref_columns: i64 = conn
            .query_row(
                "SELECT count(*) FROM pragma_table_info('connections') WHERE name='credential_ref'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(old_ref_columns, 0);
        let old_table: i64 = conn
            .query_row(
                "SELECT count(*) FROM sqlite_master WHERE name='connection_secrets_obsolete'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(old_table, 1);
    }

    #[test]
    fn malformed_unicode_key_fails_without_panicking() {
        let root = root("bad-key");
        fs::create_dir_all(&root).unwrap();
        fs::write(root.join("config.toml"), "connection_secret_key = \"🔑\"\n").unwrap();
        assert!(
            SqliteEncryptedBackend::new(root)
                .write(1, DATABASE_PASSWORD, "pw")
                .is_err()
        );
    }
}
