//! 连接凭据：以 (connection_id, secret_kind) 为主键保存 AES-256-GCM 密文。
//! 默认密钥内置，可只在 config.toml 中覆盖；不读取旧系统凭据或旧加密文件。

use std::path::PathBuf;

use aes_gcm::aead::{Aead, Payload};
use aes_gcm::{Aes256Gcm, Nonce};
use rand::RngCore;
use rusqlite::{Connection, OptionalExtension, params};

use crate::credential::{CredentialBackend, CredentialError};
use crate::sqlite::sqlite_error;

pub(crate) const DATABASE_PASSWORD: &str = "database_password";
pub(crate) const SSH_PASSWORD: &str = "ssh_password";
pub(crate) const SSH_PASSPHRASE: &str = "ssh_passphrase";
pub(crate) const PROXY_PASSWORD: &str = "proxy_password";
/// TLS 客户端私钥口令（加密私钥）。
pub(crate) const TLS_CLIENT_KEY_PASSPHRASE: &str = "tls_client_key_passphrase";
pub(crate) const ENDPOINT_URI: &str = "endpoint_uri";
pub(crate) const URL_PARAMS: &str = "url_params";
/// Oracle Wallet 口令：与登录密码分开存，改密码不会误删 Wallet 凭据。
pub(crate) const ORACLE_WALLET_PASSWORD: &str = "oracle_wallet_password";

const CREATE_SQL: &str = "CREATE TABLE IF NOT EXISTS connection_secrets (
    connection_id INTEGER NOT NULL,
    secret_kind TEXT NOT NULL,
    version INTEGER NOT NULL,
    nonce BLOB NOT NULL,
    ciphertext BLOB NOT NULL,
    PRIMARY KEY(connection_id, secret_kind)
);";
const VERSION: i64 = 1;

pub(crate) fn ensure_schema(conn: &Connection) -> fluxdb_core::Result<()> {
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

    /// 与模型 Key 共享同一把数据密钥。
    fn cipher(&self) -> Result<Aes256Gcm, CredentialError> {
        crate::secret_crypto::load_cipher(&self.root).map(|(cipher, _)| cipher)
    }

    fn aad(connection_id: u64, secret_kind: &str) -> Vec<u8> {
        crate::secret_crypto::connection_aad(connection_id, secret_kind)
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
        let cipher = self.cipher()?;
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
        let cipher = self.cipher()?;
        // config.toml 改密钥后先认证现有密文，拒绝用错误密钥覆盖。
        let sample: Option<(u64, String, Vec<u8>, Vec<u8>)> = conn.query_row(
            "SELECT connection_id, secret_kind, nonce, ciphertext FROM connection_secrets WHERE version = 1 LIMIT 1", [],
            |r| Ok((r.get::<_, i64>(0)? as u64, r.get(1)?, r.get(2)?, r.get(3)?)),
        ).optional().map_err(db_error)?;
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

fn db_error(error: rusqlite::Error) -> CredentialError {
    CredentialError::Failure(format!("连接凭据数据库操作失败: {error}"))
}
fn storage_error(error: fluxdb_core::Error) -> CredentialError {
    CredentialError::Failure(error.to_string())
}
