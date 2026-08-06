//! TLS 材料：HTTPS(HTTP/1.1+2) 与 HTTP/3 共用证书，ALPN 各自配置。

use std::fs;
use std::path::Path;
use std::sync::Arc;

use rustls::ServerConfig;
use rustls::pki_types::{CertificateDer, PrivateKeyDer, PrivatePkcs8KeyDer};

#[derive(Clone)]
pub struct TlsConfig {
    certs: Vec<CertificateDer<'static>>,
    key: Arc<PrivateKeyDer<'static>>,
}

impl TlsConfig {
    /// 从 PEM 证书 / 私钥文件加载。
    pub fn from_pem_files(cert_path: impl AsRef<Path>, key_path: impl AsRef<Path>) -> Self {
        let cert_pem = fs::read(cert_path.as_ref()).unwrap_or_else(|e| {
            panic!("读取证书失败 {}: {e}", cert_path.as_ref().display());
        });
        let key_pem = fs::read(key_path.as_ref()).unwrap_or_else(|e| {
            panic!("读取私钥失败 {}: {e}", key_path.as_ref().display());
        });
        Self::from_pem_bytes(&cert_pem, &key_pem)
    }

    pub fn from_pem_bytes(cert_pem: &[u8], key_pem: &[u8]) -> Self {
        let certs = rustls_pemfile::certs(&mut &*cert_pem)
            .collect::<Result<Vec<_>, _>>()
            .unwrap_or_else(|e| panic!("解析证书 PEM 失败: {e}"));
        if certs.is_empty() {
            panic!("证书 PEM 中没有 certificate");
        }

        let key = rustls_pemfile::private_key(&mut &*key_pem)
            .unwrap_or_else(|e| panic!("解析私钥 PEM 失败: {e}"))
            .unwrap_or_else(|| panic!("私钥 PEM 中没有 private key"));

        Self {
            certs,
            key: Arc::new(key),
        }
    }

    /// 开发用自签证书（localhost 等）。
    pub fn self_signed(hostnames: &[&str]) -> Self {
        let names: Vec<String> = if hostnames.is_empty() {
            vec!["localhost".into(), "127.0.0.1".into()]
        } else {
            hostnames.iter().map(|s| (*s).to_string()).collect()
        };

        let certified = rcgen::generate_simple_self_signed(names)
            .unwrap_or_else(|e| panic!("生成自签证书失败: {e}"));

        let cert = CertificateDer::from(certified.cert);
        let key = PrivateKeyDer::Pkcs8(PrivatePkcs8KeyDer::from(
            certified.signing_key.serialize_der(),
        ));

        Self {
            certs: vec![cert],
            key: Arc::new(key),
        }
    }

    pub(crate) fn rustls_https(&self) -> Arc<ServerConfig> {
        let key = self.key.as_ref().clone_key();
        let mut config = ServerConfig::builder()
            .with_no_client_auth()
            .with_single_cert(self.certs.clone(), key)
            .unwrap_or_else(|e| panic!("构建 HTTPS rustls 配置失败: {e}"));
        config.alpn_protocols = vec![b"h2".to_vec(), b"http/1.1".to_vec()];
        Arc::new(config)
    }

    pub(crate) fn rustls_http3(&self) -> Arc<ServerConfig> {
        let key = self.key.as_ref().clone_key();
        let mut config = ServerConfig::builder()
            .with_no_client_auth()
            .with_single_cert(self.certs.clone(), key)
            .unwrap_or_else(|e| panic!("构建 HTTP/3 rustls 配置失败: {e}"));
        config.alpn_protocols = vec![b"h3".to_vec()];
        config.max_early_data_size = u32::MAX;
        Arc::new(config)
    }
}
