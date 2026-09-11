// PostgreSQL TLS 策略（T05）：把 profile 的 ssl_mode / CA / 客户端证书映射为 rustls ClientConfig。
//
// ssl_mode 差异（与 PG 语义一致）：
// - `Require`：加密但不校验证书（accept-all）；
// - `VerifyCa`：校验证书链到受信 CA，但不校验主机名；
// - `VerifyFull`：校验证书链 + 主机名。
// 证书正文、私钥不落日志；文件路径来自 profile 的 `SecretRef`（本地文件引用）。

use rustls::client::danger::{HandshakeSignatureValid, ServerCertVerified, ServerCertVerifier};
use rustls::pki_types::{CertificateDer, PrivateKeyDer, ServerName, UnixTime};
use rustls::server::ParsedCertificate;
use rustls::DigitallySignedStruct;

/// 构建 PG TLS 拨号器。
///
/// 返回 `None` 表示不启用 TLS（调用方走 NoTls）。
fn pg_tls_connect(
    profile: &fluxdb_core::PostgresConnectionProfile,
) -> fluxdb_core::Result<Option<tokio_postgres_rustls::MakeRustlsConnect>> {
    if !profile.tls.enabled {
        return Ok(None);
    }
    let client_config = pg_tls_client_config(profile)?;
    Ok(Some(tokio_postgres_rustls::MakeRustlsConnect::new(client_config)))
}

/// TLS 校验所使用的主机名（独立 `tls.server_name` 优先，否则连接主机名）。
fn pg_server_name(profile: &fluxdb_core::PostgresConnectionProfile) -> &str {
    let cfg = profile.tls.server_name.trim();
    if cfg.is_empty() {
        profile.basic.host.trim()
    } else {
        cfg
    }
}

/// 进程内安装一次 ring crypto provider（`ClientConfig::builder()` 依赖默认 provider）。
/// 幂等；若他处已安装其他 provider 则忽略。
fn pg_tls_install_ring_provider() {
    static ONCE: std::sync::Once = std::sync::Once::new();
    ONCE.call_once(|| {
        let _ = rustls::crypto::ring::default_provider().install_default();
    });
}

fn pg_tls_client_config(
    profile: &fluxdb_core::PostgresConnectionProfile,
) -> fluxdb_core::Result<rustls::ClientConfig> {
    pg_tls_install_ring_provider();
    let provider = std::sync::Arc::new(rustls::crypto::ring::default_provider());

    // require：仅加密不做任何证书校验。
    let accepts_invalid_certs =
        matches!(profile.tls.ssl_mode, fluxdb_core::PostgresSslMode::Require);

    if accepts_invalid_certs {
        let builder = rustls::ClientConfig::builder()
            .dangerous()
            .with_custom_certificate_verifier(std::sync::Arc::new(PgNoCertVerification {
                provider: provider.clone(),
            }));
        return pg_tls_client_auth(builder, profile);
    }

    let root_store = pg_root_cert_store(profile)?;
    let builder = if matches!(profile.tls.ssl_mode, fluxdb_core::PostgresSslMode::VerifyFull) {
        // verify-full：链 + 主机名校验。
        rustls::ClientConfig::builder().with_root_certificates(root_store)
    } else {
        // verify-ca：仅链到 CA，不校验主机名。
        rustls::ClientConfig::builder()
            .dangerous()
            .with_custom_certificate_verifier(std::sync::Arc::new(PgCaOnlyVerifier {
                provider: provider.clone(),
                roots: std::sync::Arc::new(root_store),
            }))
    };
    pg_tls_client_auth(builder, profile)
}

/// 从 profile 的 CA 文件（或内置信任根）构建 RootCertStore。
fn pg_root_cert_store(
    profile: &fluxdb_core::PostgresConnectionProfile,
) -> fluxdb_core::Result<rustls::RootCertStore> {
    let mut root_store = rustls::RootCertStore::empty();
    let ca_path = profile.tls.ca.value().map(str::trim).filter(|p| !p.is_empty());
    match ca_path {
        Some(path) => {
            let certs = pg_read_pem_certs("CA 证书", path)?;
            let (valid_count, _) = root_store.add_parsable_certificates(certs);
            if valid_count == 0 {
                return Err(fluxdb_core::Error::new(
                    fluxdb_core::ErrorKind::Connection,
                    format!("CA 证书文件无有效证书 {path}"),
                ));
            }
        }
        None => {
            root_store.extend(webpki_roots::TLS_SERVER_ROOTS.iter().cloned());
        }
    }
    Ok(root_store)
}

/// 配置客户端证书（mTLS）；无证书给`with_no_client_auth`。
fn pg_tls_client_auth(
    builder: rustls::ConfigBuilder<rustls::ClientConfig, rustls::client::WantsClientCert>,
    profile: &fluxdb_core::PostgresConnectionProfile,
) -> fluxdb_core::Result<rustls::ClientConfig> {
    let cert_path = profile.tls.client_cert.value().map(str::trim).filter(|p| !p.is_empty());
    let key_path = profile.tls.client_key.value().map(str::trim).filter(|p| !p.is_empty());
    match (cert_path, key_path) {
        (Some(cert_path), Some(key_path)) => {
            let certs = pg_read_pem_certs("客户端证书", cert_path)?;
            if certs.is_empty() {
                return Err(fluxdb_core::Error::new(
                    fluxdb_core::ErrorKind::Connection,
                    format!("客户端证书文件无证书 {cert_path}"),
                ));
            }
            let private_key = pg_read_private_key(key_path)?;
            builder.with_client_auth_cert(certs, private_key).map_err(|e| {
                fluxdb_core::Error::new(
                    fluxdb_core::ErrorKind::Connection,
                    format!("客户端证书/私钥不匹配或无效: {e}"),
                )
            })
        }
        (Some(_), None) => Err(fluxdb_core::Error::new(
            fluxdb_core::ErrorKind::Connection,
            "已配置客户端证书但缺少客户端私钥",
        )),
        (None, Some(_)) => Err(fluxdb_core::Error::new(
            fluxdb_core::ErrorKind::Connection,
            "已配置客户端私钥但缺少客户端证书",
        )),
        (None, None) => Ok(builder.with_no_client_auth()),
    }
}

fn pg_read_pem_certs(
    label: &str,
    path: &str,
) -> fluxdb_core::Result<Vec<CertificateDer<'static>>> {
    let file = std::fs::File::open(path).map_err(|e| {
        fluxdb_core::Error::new(
            fluxdb_core::ErrorKind::Connection,
            format!("{label}读取失败 {path}: {e}"),
        )
    })?;
    let mut reader = std::io::BufReader::new(file);
    rustls_pemfile::certs(&mut reader)
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| {
            fluxdb_core::Error::new(
                fluxdb_core::ErrorKind::Connection,
                format!("{label}解析失败 {path}: {e}"),
            )
        })
}

fn pg_read_private_key(path: &str) -> fluxdb_core::Result<PrivateKeyDer<'static>> {
    let file = std::fs::File::open(path).map_err(|e| {
        fluxdb_core::Error::new(
            fluxdb_core::ErrorKind::Connection,
            format!("客户端私钥读取失败 {path}: {e}"),
        )
    })?;
    let mut reader = std::io::BufReader::new(file);
    rustls_pemfile::private_key(&mut reader)
        .map_err(|e| {
            fluxdb_core::Error::new(
                fluxdb_core::ErrorKind::Connection,
                format!("客户端私钥解析失败 {path}: {e}"),
            )
        })?
        .ok_or_else(|| {
            fluxdb_core::Error::new(
                fluxdb_core::ErrorKind::Connection,
                format!("客户端私钥文件无密钥 {path}"),
            )
        })
}

/// require：不校验服务器证书，仅加密。
#[derive(Debug)]
struct PgNoCertVerification {
    provider: std::sync::Arc<rustls::crypto::CryptoProvider>,
}

impl ServerCertVerifier for PgNoCertVerification {
    fn verify_server_cert(
        &self,
        _end_entity: &CertificateDer<'_>,
        _intermediates: &[CertificateDer<'_>],
        _server_name: &ServerName<'_>,
        _ocsp_response: &[u8],
        _now: UnixTime,
    ) -> Result<ServerCertVerified, rustls::Error> {
        Ok(ServerCertVerified::assertion())
    }

    fn verify_tls12_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, rustls::Error> {
        rustls::crypto::verify_tls12_signature(
            message,
            cert,
            dss,
            &self.provider.signature_verification_algorithms,
        )
    }

    fn verify_tls13_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, rustls::Error> {
        rustls::crypto::verify_tls13_signature(
            message,
            cert,
            dss,
            &self.provider.signature_verification_algorithms,
        )
    }

    fn supported_verify_schemes(&self) -> Vec<rustls::SignatureScheme> {
        self.provider.signature_verification_algorithms.supported_schemes()
    }
}

/// verify-ca：校验证书链到 CA，不校验主机名。
#[derive(Debug)]
struct PgCaOnlyVerifier {
    provider: std::sync::Arc<rustls::crypto::CryptoProvider>,
    roots: std::sync::Arc<rustls::RootCertStore>,
}

impl ServerCertVerifier for PgCaOnlyVerifier {
    fn verify_server_cert(
        &self,
        end_entity: &CertificateDer<'_>,
        intermediates: &[CertificateDer<'_>],
        _server_name: &ServerName<'_>,
        _ocsp_response: &[u8],
        now: UnixTime,
    ) -> Result<ServerCertVerified, rustls::Error> {
        let cert = ParsedCertificate::try_from(end_entity)?;
        rustls::client::verify_server_cert_signed_by_trust_anchor(
            &cert,
            &self.roots,
            intermediates,
            now,
            self.provider.signature_verification_algorithms.all,
        )?;
        Ok(ServerCertVerified::assertion())
    }

    fn verify_tls12_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, rustls::Error> {
        rustls::crypto::verify_tls12_signature(
            message,
            cert,
            dss,
            &self.provider.signature_verification_algorithms,
        )
    }

    fn verify_tls13_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, rustls::Error> {
        rustls::crypto::verify_tls13_signature(
            message,
            cert,
            dss,
            &self.provider.signature_verification_algorithms,
        )
    }

    fn supported_verify_schemes(&self) -> Vec<rustls::SignatureScheme> {
        self.provider.signature_verification_algorithms.supported_schemes()
    }
}
