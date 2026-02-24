use std::fs;
use std::io::BufReader;
use std::sync::Arc;

pub struct TlsConfigOptions<'a> {
    pub cert_path: &'a str,
    pub key_path: &'a str,
    pub alpn_protocols: Vec<Vec<u8>>,
}

#[derive(Debug, thiserror::Error)]
pub enum TlsError {
    #[error("failed to read file '{0}': {1}")]
    ReadFile(String, std::io::Error),

    #[error("no certificates found in '{0}'")]
    NoCertificates(String),

    #[error("no private key found in '{0}'")]
    NoPrivateKey(String),

    #[error("invalid TLS configuration: {0}")]
    InvalidConfig(String),
}

pub fn build_tls_server_config(
    options: TlsConfigOptions<'_>,
) -> Result<Arc<rustls::ServerConfig>, TlsError> {
    let cert_data = fs::read(options.cert_path)
        .map_err(|e| TlsError::ReadFile(options.cert_path.to_string(), e))?;
    let key_data = fs::read(options.key_path)
        .map_err(|e| TlsError::ReadFile(options.key_path.to_string(), e))?;

    let certs: Vec<_> = rustls_pemfile::certs(&mut BufReader::new(cert_data.as_slice()))
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| TlsError::ReadFile(options.cert_path.to_string(), e))?;

    if certs.is_empty() {
        return Err(TlsError::NoCertificates(options.cert_path.to_string()));
    }

    let key = rustls_pemfile::private_key(&mut BufReader::new(key_data.as_slice()))
        .map_err(|e| TlsError::ReadFile(options.key_path.to_string(), e))?
        .ok_or_else(|| TlsError::NoPrivateKey(options.key_path.to_string()))?;

    let provider = rustls::crypto::ring::default_provider();

    let mut config = rustls::ServerConfig::builder_with_provider(provider.into())
        .with_protocol_versions(&[&rustls::version::TLS13])
        .map_err(|e| TlsError::InvalidConfig(e.to_string()))?
        .with_no_client_auth()
        .with_single_cert(certs, key)
        .map_err(|e| TlsError::InvalidConfig(e.to_string()))?;

    config.alpn_protocols = options.alpn_protocols;

    Ok(Arc::new(config))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn generate_test_cert() -> (String, String) {
        let key_pair = rcgen::KeyPair::generate().unwrap();
        let cert_params = rcgen::CertificateParams::new(vec!["localhost".to_string()]).unwrap();
        let cert = cert_params.self_signed(&key_pair).unwrap();

        let cert_pem = cert.pem();
        let key_pem = key_pair.serialize_pem();

        let dir = tempfile::tempdir().unwrap();
        let cert_path = dir.path().join("cert.pem");
        let key_path = dir.path().join("key.pem");

        fs::write(&cert_path, &cert_pem).unwrap();
        fs::write(&key_path, &key_pem).unwrap();

        // Leak the tempdir so files persist for the duration of the test
        std::mem::forget(dir);

        (
            cert_path.to_string_lossy().to_string(),
            key_path.to_string_lossy().to_string(),
        )
    }

    #[test]
    fn build_tls_config_from_valid_pem() {
        let (cert_path, key_path) = generate_test_cert();

        let result = build_tls_server_config(TlsConfigOptions {
            cert_path: &cert_path,
            key_path: &key_path,
            alpn_protocols: vec![b"h2".to_vec()],
        });

        assert!(result.is_ok(), "expected Ok, got {result:?}");
        let config = result.unwrap();
        assert_eq!(config.alpn_protocols, vec![b"h2".to_vec()]);
    }

    #[test]
    fn build_tls_config_rejects_invalid_cert() {
        let dir = tempfile::tempdir().unwrap();
        let cert_path = dir.path().join("bad_cert.pem");

        fs::write(&cert_path, b"not a valid certificate").unwrap();

        let (_, valid_key_path) = generate_test_cert();

        let result = build_tls_server_config(TlsConfigOptions {
            cert_path: cert_path.to_str().unwrap(),
            key_path: &valid_key_path,
            alpn_protocols: vec![],
        });

        assert!(
            matches!(result, Err(TlsError::NoCertificates(_))),
            "expected NoCertificates, got {result:?}"
        );
    }

    #[test]
    fn build_tls_config_rejects_missing_key() {
        let dir = tempfile::tempdir().unwrap();
        let cert_path = dir.path().join("cert.pem");
        let key_path = dir.path().join("no_key.pem");

        let key_pair = rcgen::KeyPair::generate().unwrap();
        let cert_params = rcgen::CertificateParams::new(vec!["localhost".to_string()]).unwrap();
        let cert = cert_params.self_signed(&key_pair).unwrap();
        fs::write(&cert_path, cert.pem()).unwrap();

        // Write cert content instead of key — no private key present
        fs::write(&key_path, cert.pem()).unwrap();

        let result = build_tls_server_config(TlsConfigOptions {
            cert_path: cert_path.to_str().unwrap(),
            key_path: key_path.to_str().unwrap(),
            alpn_protocols: vec![],
        });

        assert!(
            matches!(result, Err(TlsError::NoPrivateKey(_))),
            "expected NoPrivateKey, got {result:?}"
        );
    }

    #[test]
    fn build_tls_config_rejects_nonexistent_file() {
        let result = build_tls_server_config(TlsConfigOptions {
            cert_path: "/nonexistent/cert.pem",
            key_path: "/nonexistent/key.pem",
            alpn_protocols: vec![],
        });

        assert!(
            matches!(result, Err(TlsError::ReadFile(_, _))),
            "expected ReadFile, got {result:?}"
        );
    }

    #[test]
    fn build_tls_config_sets_alpn_protocols() {
        let (cert_path, key_path) = generate_test_cert();

        let config = build_tls_server_config(TlsConfigOptions {
            cert_path: &cert_path,
            key_path: &key_path,
            alpn_protocols: vec![b"h2".to_vec(), b"http/1.1".to_vec()],
        })
        .unwrap();

        assert_eq!(
            config.alpn_protocols,
            vec![b"h2".to_vec(), b"http/1.1".to_vec()]
        );
    }
}
