//! Opt-in SHA-256 certificate pinning for eSCL over HTTPS.
//!
//! Most scanners present self-signed certificates that WebPKI rejects. A pin
//! trusts exactly one end-entity certificate for one named host (optionally a
//! single port). Every other connection keeps ureq's default WebPKI path.

use super::{normalize_host, parse_endpoint_port, Endpoint, MdnsLocalResolver};
use rustls::client::danger::{HandshakeSignatureValid, ServerCertVerified, ServerCertVerifier};
use rustls::crypto::hash::HashAlgorithm;
use rustls::crypto::CryptoProvider;
use rustls::pki_types::{CertificateDer, ServerName, UnixTime};
use rustls::{
    CertificateError, ClientConfig, ClientConnection, DigitallySignedStruct, OtherError,
    SignatureScheme, StreamOwned, SupportedCipherSuite,
};
use std::io::{Read, Write};
use std::net::Ipv6Addr;
use std::sync::Arc;
use std::time::Duration;
use ureq::unversioned::transport::{
    Buffers, ConnectionDetails, Connector, DefaultConnector, LazyBuffers, NextTimeout,
    TcpConnector, Transport, TransportAdapter,
};

pub(crate) const ESCL_CERT_PINS_ENV: &str = "OPEN_SCANLINE_ESCL_CERT_PINS";

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct CertificatePin {
    pub(crate) host: String,
    pub(crate) port: Option<u16>,
    pub(crate) fingerprint: [u8; 32],
}

/// Pins from `OPEN_SCANLINE_ESCL_CERT_PINS`, formatted as
/// `host[:port]=sha256hex[,host[:port]=sha256hex...]`. Parsed once per process;
/// every HTTP exchange builds an agent.
pub(crate) fn env_certificate_pins() -> Vec<CertificatePin> {
    static PINS: std::sync::OnceLock<Vec<CertificatePin>> = std::sync::OnceLock::new();
    PINS.get_or_init(|| match std::env::var(ESCL_CERT_PINS_ENV) {
        Ok(raw) => parse_certificate_pins(&raw),
        Err(_) => Vec::new(),
    })
    .clone()
}

/// Invalid entries are skipped, which leaves their host on WebPKI.
pub(crate) fn parse_certificate_pins(raw: &str) -> Vec<CertificatePin> {
    raw.split(',')
        .map(str::trim)
        .filter(|entry| !entry.is_empty())
        .filter_map(|entry| {
            let pin = parse_certificate_pin(entry);
            if pin.is_none() {
                crate::debug_log::debug_log!(
                    "escl",
                    "ignoring invalid {ESCL_CERT_PINS_ENV} entry {entry:?}"
                );
            }
            pin
        })
        .collect()
}

pub(crate) fn parse_certificate_pin(entry: &str) -> Option<CertificatePin> {
    let (authority, fingerprint) = entry.split_once('=')?;
    let (host, port) = pin_host_port(authority.trim())?;
    Some(CertificatePin {
        host,
        port,
        fingerprint: parse_fingerprint(fingerprint.trim())?,
    })
}

fn pin_host_port(authority: &str) -> Option<(String, Option<u16>)> {
    let (host, suffix) = if let Some(rest) = authority.strip_prefix('[') {
        let (host, suffix) = rest.split_once(']')?;
        (host.parse::<Ipv6Addr>().ok()?.to_string(), suffix)
    } else {
        let (host, suffix) = authority.split_at(authority.find(':').unwrap_or(authority.len()));
        (normalize_host(host)?, suffix)
    };
    let port = if suffix.is_empty() {
        None
    } else {
        Some(parse_endpoint_port(suffix, 0)?)
    };
    Some((host, port))
}

/// 64 hex digits, case-insensitive, optionally separated by `:`.
pub(crate) fn parse_fingerprint(value: &str) -> Option<[u8; 32]> {
    let digits = value
        .bytes()
        .filter(|&byte| byte != b':')
        .map(|byte| char::from(byte).to_digit(16).map(|digit| digit as u8))
        .collect::<Option<Vec<u8>>>()?;
    if digits.len() != 64 {
        return None;
    }
    let mut fingerprint = [0; 32];
    for (byte, pair) in fingerprint.iter_mut().zip(digits.chunks(2)) {
        *byte = (pair[0] << 4) | pair[1];
    }
    Some(fingerprint)
}

pub(crate) fn format_fingerprint(fingerprint: &[u8]) -> String {
    fingerprint
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

/// Host-and-port pins take precedence over a host-only pin.
pub(crate) fn pin_for<'a>(
    pins: &'a [CertificatePin],
    host: &str,
    port: u16,
) -> Option<&'a CertificatePin> {
    let host = connection_host(host)?;
    pins.iter()
        .find(|pin| pin.host == host && pin.port == Some(port))
        .or_else(|| {
            pins.iter()
                .find(|pin| pin.host == host && pin.port.is_none())
        })
}

fn connection_host(host: &str) -> Option<String> {
    match host
        .strip_prefix('[')
        .and_then(|rest| rest.strip_suffix(']'))
    {
        Some(address) => Some(address.parse::<Ipv6Addr>().ok()?.to_string()),
        None => host
            .parse::<Ipv6Addr>()
            .map(|address| address.to_string())
            .ok()
            .or_else(|| normalize_host(host)),
    }
}

fn connection_pin<'a>(
    pins: &'a [CertificatePin],
    details: &ConnectionDetails,
) -> Option<&'a CertificatePin> {
    if !details.needs_tls() {
        return None;
    }
    pin_for(
        pins,
        details.uri.host()?,
        details.uri.port_u16().unwrap_or(443),
    )
}

#[derive(Clone, PartialEq, Eq)]
pub(crate) struct CertificatePinMismatch {
    host: String,
    presented: Option<[u8; 32]>,
}

impl std::fmt::Display for CertificatePinMismatch {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let presented = self.presented.map_or_else(
            || "unavailable".to_string(),
            |hash| format_fingerprint(&hash),
        );
        write!(
            formatter,
            "certificate for {} does not match the pinned SHA-256 fingerprint (got {presented})",
            self.host
        )
    }
}

// rustls formats `CertificateError::Other` with `Debug`; keep it readable.
impl std::fmt::Debug for CertificatePinMismatch {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        std::fmt::Display::fmt(self, formatter)
    }
}

impl std::error::Error for CertificatePinMismatch {}

pub(crate) fn check_pinned_fingerprint(
    host: &str,
    pinned: &[u8; 32],
    presented: Option<[u8; 32]>,
) -> Result<(), CertificatePinMismatch> {
    if presented.as_ref() == Some(pinned) {
        return Ok(());
    }
    Err(CertificatePinMismatch {
        host: host.to_string(),
        presented,
    })
}

/// SHA-256 from the crypto provider's own cipher-suite hash.
pub(crate) fn certificate_fingerprint(
    provider: &CryptoProvider,
    certificate: &[u8],
) -> Option<[u8; 32]> {
    provider
        .cipher_suites
        .iter()
        .map(|suite| match suite {
            SupportedCipherSuite::Tls12(suite) => suite.common.hash_provider,
            SupportedCipherSuite::Tls13(suite) => suite.common.hash_provider,
        })
        .find(|hash| hash.algorithm() == HashAlgorithm::SHA256)
        .and_then(|hash| hash.hash(certificate).as_ref().try_into().ok())
}

/// Provider selection matching ureq's rustls connector.
fn crypto_provider() -> Arc<CryptoProvider> {
    CryptoProvider::get_default()
        .cloned()
        .unwrap_or_else(|| Arc::new(rustls::crypto::ring::default_provider()))
}

/// Accepts exactly the pinned end-entity certificate. Chain, expiry and name
/// are deliberately not checked; handshake signatures still are, so the peer
/// must hold the pinned certificate's private key.
#[derive(Debug)]
pub(crate) struct PinnedCertificateVerifier {
    host: String,
    fingerprint: [u8; 32],
    provider: Arc<CryptoProvider>,
}

impl ServerCertVerifier for PinnedCertificateVerifier {
    fn verify_server_cert(
        &self,
        end_entity: &CertificateDer<'_>,
        _intermediates: &[CertificateDer<'_>],
        _server_name: &ServerName<'_>,
        _ocsp_response: &[u8],
        _now: UnixTime,
    ) -> Result<ServerCertVerified, rustls::Error> {
        check_pinned_fingerprint(
            &self.host,
            &self.fingerprint,
            certificate_fingerprint(&self.provider, end_entity),
        )
        .map(|()| ServerCertVerified::assertion())
        .map_err(|mismatch| {
            rustls::Error::InvalidCertificate(CertificateError::Other(OtherError(Arc::new(
                mismatch,
            ))))
        })
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

    fn supported_verify_schemes(&self) -> Vec<SignatureScheme> {
        self.provider
            .signature_verification_algorithms
            .supported_schemes()
    }
}

/// Routes HTTPS connections to a pinned host through
/// [`PinnedCertificateVerifier`]; everything else uses ureq's
/// [`DefaultConnector`] unchanged.
#[derive(Debug)]
pub(crate) struct PinningConnector {
    default: DefaultConnector,
    tcp: TcpConnector,
    pins: Vec<CertificatePin>,
    provider: Arc<CryptoProvider>,
}

impl PinningConnector {
    pub(crate) fn new(pins: Vec<CertificatePin>) -> Self {
        Self {
            default: DefaultConnector::new(),
            tcp: TcpConnector::default(),
            pins,
            provider: crypto_provider(),
        }
    }
}

impl Connector for PinningConnector {
    type Out = Box<dyn Transport>;

    fn connect(
        &self,
        details: &ConnectionDetails,
        chained: Option<()>,
    ) -> Result<Option<Self::Out>, ureq::Error> {
        let Some(pin) = connection_pin(&self.pins, details) else {
            return self.default.connect(details, chained);
        };
        let Some(transport) = self.tcp.connect(details, chained)? else {
            return Ok(None);
        };
        let verifier = PinnedCertificateVerifier {
            host: pin.host.clone(),
            fingerprint: pin.fingerprint,
            provider: self.provider.clone(),
        };
        let transport = tls_transport(
            details,
            transport.boxed(),
            Arc::new(verifier),
            self.provider.clone(),
        )?;
        Ok(Some(Box::new(transport)))
    }
}

fn tls_transport(
    details: &ConnectionDetails,
    transport: Box<dyn Transport>,
    verifier: Arc<dyn ServerCertVerifier>,
    provider: Arc<CryptoProvider>,
) -> Result<VerifiedTlsTransport, ureq::Error> {
    let config = ClientConfig::builder_with_provider(provider)
        .with_protocol_versions(rustls::ALL_VERSIONS)?
        .dangerous()
        .with_custom_certificate_verifier(verifier)
        .with_no_client_auth();
    let host = details
        .uri
        .host()
        .ok_or(ureq::Error::Tls("uri authority for tls"))?;
    let host = host
        .strip_prefix('[')
        .and_then(|rest| rest.strip_suffix(']'))
        .unwrap_or(host);
    let name = ServerName::try_from(host.to_string())
        .map_err(|_| ureq::Error::Tls("Rustls invalid dns name error"))?;
    let conn = ClientConnection::new(Arc::new(config), name)?;
    Ok(VerifiedTlsTransport {
        buffers: LazyBuffers::new(
            details.config.input_buffer_size(),
            details.config.output_buffer_size(),
        ),
        stream: StreamOwned {
            conn,
            sock: TransportAdapter::new(transport),
        },
    })
}

/// Modelled on ureq's `RustlsTransport`.
pub(crate) struct VerifiedTlsTransport {
    buffers: LazyBuffers,
    stream: StreamOwned<ClientConnection, TransportAdapter>,
}

impl std::fmt::Debug for VerifiedTlsTransport {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.debug_struct("VerifiedTlsTransport").finish()
    }
}

impl Transport for VerifiedTlsTransport {
    fn buffers(&mut self) -> &mut dyn Buffers {
        &mut self.buffers
    }

    fn transmit_output(&mut self, amount: usize, timeout: NextTimeout) -> Result<(), ureq::Error> {
        self.stream.get_mut().set_timeout(timeout);
        let output = &self.buffers.output()[..amount];
        self.stream.write_all(output)?;
        Ok(())
    }

    fn await_input(&mut self, timeout: NextTimeout) -> Result<bool, ureq::Error> {
        self.stream.get_mut().set_timeout(timeout);
        let input = self.buffers.input_append_buf();
        let amount = self.stream.read(input)?;
        self.buffers.input_appended(amount);
        Ok(amount > 0)
    }

    fn is_open(&mut self) -> bool {
        self.stream.get_mut().get_mut().is_open()
    }

    fn is_tls(&self) -> bool {
        true
    }
}

fn rustls_error(error: &ureq::Error) -> Option<&rustls::Error> {
    match error {
        ureq::Error::Rustls(error) => Some(error),
        ureq::Error::Io(error) => error.get_ref()?.downcast_ref::<rustls::Error>(),
        _ => None,
    }
}

/// The user-facing pin mismatch message carried by a failed exchange.
pub(crate) fn certificate_pin_mismatch(error: &ureq::Error) -> Option<String> {
    let rustls::Error::InvalidCertificate(CertificateError::Other(other)) = rustls_error(error)?
    else {
        return None;
    };
    other
        .0
        .downcast_ref::<CertificatePinMismatch>()
        .map(ToString::to_string)
}

/// Under `--verbose`, log the SHA-256 of the certificate an unpinned HTTPS
/// endpoint presented after WebPKI rejected it. The probe never trusts the
/// certificate: its verifier always fails the handshake.
pub(crate) fn log_presented_certificate(
    endpoint: &Endpoint,
    error: &ureq::Error,
    timeout: Duration,
) {
    if !crate::debug_log::enabled()
        || !endpoint.secure
        || !matches!(
            rustls_error(error),
            Some(rustls::Error::InvalidCertificate(_))
        )
        || certificate_pin_mismatch(error).is_some()
    {
        return;
    }
    let config = ureq::Agent::config_builder()
        .timeout_global(Some(timeout))
        .http_status_as_error(false)
        .max_redirects(0)
        .proxy(None)
        .build();
    let agent = ureq::Agent::with_parts(
        config,
        FingerprintProbeConnector {
            tcp: TcpConnector::default(),
            provider: crypto_provider(),
        },
        MdnsLocalResolver::default(),
    );
    let _ = agent.get(&endpoint.url("/")).call();
}

#[derive(Debug)]
struct FingerprintProbeConnector {
    tcp: TcpConnector,
    provider: Arc<CryptoProvider>,
}

impl Connector for FingerprintProbeConnector {
    type Out = VerifiedTlsTransport;

    fn connect(
        &self,
        details: &ConnectionDetails,
        chained: Option<()>,
    ) -> Result<Option<Self::Out>, ureq::Error> {
        let (true, Some(host)) = (details.needs_tls(), details.uri.host()) else {
            return Ok(None);
        };
        let Some(transport) = self.tcp.connect(details, chained)? else {
            return Ok(None);
        };
        let verifier = FingerprintProbeVerifier {
            host: host.to_string(),
            provider: self.provider.clone(),
        };
        tls_transport(
            details,
            transport.boxed(),
            Arc::new(verifier),
            self.provider.clone(),
        )
        .map(Some)
    }
}

#[derive(Debug)]
struct FingerprintProbeVerifier {
    host: String,
    provider: Arc<CryptoProvider>,
}

impl ServerCertVerifier for FingerprintProbeVerifier {
    fn verify_server_cert(
        &self,
        end_entity: &CertificateDer<'_>,
        _intermediates: &[CertificateDer<'_>],
        _server_name: &ServerName<'_>,
        _ocsp_response: &[u8],
        _now: UnixTime,
    ) -> Result<ServerCertVerified, rustls::Error> {
        if let Some(fingerprint) = certificate_fingerprint(&self.provider, end_entity) {
            crate::debug_log::debug_log!(
                "escl",
                "{} presented an untrusted certificate with SHA-256 {}; set {ESCL_CERT_PINS_ENV} only after confirming it on the device",
                self.host,
                format_fingerprint(&fingerprint)
            );
        }
        Err(rustls::Error::General(
            "certificate fingerprint probe never trusts a certificate".into(),
        ))
    }

    fn verify_tls12_signature(
        &self,
        _message: &[u8],
        _cert: &CertificateDer<'_>,
        _dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, rustls::Error> {
        Err(rustls::Error::General(
            "certificate fingerprint probe never trusts a certificate".into(),
        ))
    }

    fn verify_tls13_signature(
        &self,
        _message: &[u8],
        _cert: &CertificateDer<'_>,
        _dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, rustls::Error> {
        Err(rustls::Error::General(
            "certificate fingerprint probe never trusts a certificate".into(),
        ))
    }

    fn supported_verify_schemes(&self) -> Vec<SignatureScheme> {
        self.provider
            .signature_verification_algorithms
            .supported_schemes()
    }
}

#[cfg(test)]
mod tests {
    use super::super::scanner_agent_with_pins;
    use super::*;
    use rustls::pki_types::pem::PemObject;
    use rustls::pki_types::PrivateKeyDer;
    use rustls::{ServerConfig, ServerConnection, SupportedProtocolVersion};
    use std::net::TcpListener;

    // Self-signed P-256 test certificate for `pinned-scanner.test`; the key
    // exists only for these tests.
    const TEST_CERT_PEM: &str = "-----BEGIN CERTIFICATE-----
MIIBkzCCATigAwIBAgIUbzf8tqB4iJlFRHBiAvyndT5cEDswCgYIKoZIzj0EAwIw
HjEcMBoGA1UEAwwTcGlubmVkLXNjYW5uZXIudGVzdDAgFw0yNjEwMTgwODMyMTZa
GA8yMTI2MDkyNDA4MzIxNlowHjEcMBoGA1UEAwwTcGlubmVkLXNjYW5uZXIudGVz
dDBZMBMGByqGSM49AgEGCCqGSM49AwEHA0IABG7y/V5wUSdIz5rUhNWCUUpy6HPe
M0CCdc06JdJSprN5Cpy4JBxVQjqVbrkrFwMAOYnZIooUdeTlJ3V4sxnJocyjUjBQ
MB0GA1UdDgQWBBRyX8X4j1UDSasTC9b/NbMXu+zApjAPBgNVHRMBAf8EBTADAQH/
MB4GA1UdEQQXMBWCE3Bpbm5lZC1zY2FubmVyLnRlc3QwCgYIKoZIzj0EAwIDSQAw
RgIhAJt6geONT4E+SZI08kY3+ETgo3mkJFf/Wd9v7yGmeqIAAiEAv7g7SUvU0O0S
Tw2iu9GVLB+dgdvGWE3igfyvtUTCnXo=
-----END CERTIFICATE-----
";
    const TEST_KEY_PEM: &str = "-----BEGIN PRIVATE KEY-----
MIGHAgEAMBMGByqGSM49AgEGCCqGSM49AwEHBG0wawIBAQQgjxXpd3SpYa0+sbMc
ags2UJw72lVfzQ2XydjCrC+MT+yhRANCAARu8v1ecFEnSM+a1ITVglFKcuhz3jNA
gnXNOiXSUqazeQqcuCQcVUI6lW65KxcDADmJ2SKKFHXk5Sd1eLMZyaHM
-----END PRIVATE KEY-----
";
    const TEST_CERT_SHA256: &str =
        "b653885f830e8f0055b663d55c93edbad5105e3c3174853edd8e68902ac09fc2";

    fn fingerprint() -> [u8; 32] {
        parse_fingerprint(TEST_CERT_SHA256).unwrap()
    }

    fn pin(host: &str, port: Option<u16>, fingerprint: [u8; 32]) -> CertificatePin {
        CertificatePin {
            host: host.into(),
            port,
            fingerprint,
        }
    }

    #[test]
    fn pins_parse_hosts_ports_and_separated_hex() {
        let colon_hex = TEST_CERT_SHA256
            .as_bytes()
            .chunks(2)
            .map(|pair| std::str::from_utf8(pair).unwrap().to_ascii_uppercase())
            .collect::<Vec<_>>()
            .join(":");
        let raw = format!(
            "Printer.Local={colon_hex}, 10.0.0.5:8443={TEST_CERT_SHA256},[FE80::1]:443={TEST_CERT_SHA256}"
        );
        assert_eq!(
            parse_certificate_pins(&raw),
            vec![
                pin("printer.local", None, fingerprint()),
                pin("10.0.0.5", Some(8443), fingerprint()),
                pin("fe80::1", Some(443), fingerprint()),
            ]
        );
    }

    #[test]
    fn invalid_pin_entries_are_skipped() {
        let short = &TEST_CERT_SHA256[..62];
        let long = format!("{TEST_CERT_SHA256}00");
        let non_hex = format!("{}zz", &TEST_CERT_SHA256[..62]);
        for entry in [
            "printer.local".to_string(),
            format!("printer.local={short}"),
            format!("printer.local={long}"),
            format!("printer.local={non_hex}"),
            format!("printer.local:0={TEST_CERT_SHA256}"),
            format!("printer.local:={TEST_CERT_SHA256}"),
            format!("printer.local:http={TEST_CERT_SHA256}"),
            format!("fe80::1={TEST_CERT_SHA256}"),
            format!("[fe80::1]x={TEST_CERT_SHA256}"),
            format!("https://printer.local={TEST_CERT_SHA256}"),
            format!("bad_host={TEST_CERT_SHA256}"),
            format!("={TEST_CERT_SHA256}"),
        ] {
            assert_eq!(parse_certificate_pin(&entry), None, "{entry}");
        }
        let raw = format!("broken,printer.local={TEST_CERT_SHA256},");
        assert_eq!(
            parse_certificate_pins(&raw),
            vec![pin("printer.local", None, fingerprint())]
        );
    }

    #[test]
    fn pin_lookup_matches_host_and_optional_port() {
        let other = [7; 32];
        let pins = vec![
            pin("printer.local", None, fingerprint()),
            pin("printer.local", Some(8443), other),
            pin("10.0.0.5", Some(443), fingerprint()),
            pin("fe80::1", None, fingerprint()),
        ];
        let lookup = |host, port| pin_for(&pins, host, port).map(|pin| pin.fingerprint);
        assert_eq!(lookup("printer.local", 443), Some(fingerprint()));
        assert_eq!(lookup("PRINTER.local", 9443), Some(fingerprint()));
        assert_eq!(lookup("printer.local", 8443), Some(other));
        assert_eq!(lookup("10.0.0.5", 443), Some(fingerprint()));
        assert_eq!(lookup("10.0.0.5", 8443), None);
        assert_eq!(lookup("[FE80::0001]", 443), Some(fingerprint()));
        assert_eq!(lookup("other.local", 443), None);
        assert_eq!(lookup("printer.local.evil", 443), None);
        assert_eq!(pin_for(&[], "printer.local", 443), None);
    }

    #[test]
    fn pinned_fingerprint_decision() {
        assert_eq!(
            check_pinned_fingerprint("printer.local", &fingerprint(), Some(fingerprint())),
            Ok(())
        );
        let mismatch =
            check_pinned_fingerprint("printer.local", &[0; 32], Some(fingerprint())).unwrap_err();
        assert_eq!(
            mismatch.to_string(),
            format!(
                "certificate for printer.local does not match the pinned SHA-256 fingerprint (got {TEST_CERT_SHA256})"
            )
        );
        assert!(check_pinned_fingerprint("printer.local", &fingerprint(), None).is_err());
    }

    #[test]
    fn fingerprint_uses_the_provider_sha256() {
        let cert = CertificateDer::from_pem_slice(TEST_CERT_PEM.as_bytes()).unwrap();
        let provider = rustls::crypto::ring::default_provider();
        assert_eq!(
            certificate_fingerprint(&provider, &cert),
            Some(fingerprint())
        );
        assert_eq!(format_fingerprint(&fingerprint()), TEST_CERT_SHA256);
    }

    fn serve_once(
        versions: &'static [&'static SupportedProtocolVersion],
    ) -> (u16, std::thread::JoinHandle<()>) {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let cert = CertificateDer::from_pem_slice(TEST_CERT_PEM.as_bytes()).unwrap();
        let key = PrivateKeyDer::from_pem_slice(TEST_KEY_PEM.as_bytes()).unwrap();
        let config =
            ServerConfig::builder_with_provider(Arc::new(rustls::crypto::ring::default_provider()))
                .with_protocol_versions(versions)
                .unwrap()
                .with_no_client_auth()
                .with_single_cert(vec![cert], key)
                .unwrap();
        let server = std::thread::spawn(move || {
            let Ok((socket, _)) = listener.accept() else {
                return;
            };
            let _ = socket.set_read_timeout(Some(Duration::from_secs(5)));
            let connection = ServerConnection::new(Arc::new(config)).unwrap();
            let mut stream = StreamOwned::new(connection, socket);
            let mut request = Vec::new();
            let mut buffer = [0; 1024];
            while !request.windows(4).any(|window| window == b"\r\n\r\n") {
                match stream.read(&mut buffer) {
                    Ok(0) | Err(_) => return,
                    Ok(read) => request.extend_from_slice(&buffer[..read]),
                }
            }
            let _ = stream
                .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\nConnection: close\r\n\r\nok");
            stream.conn.send_close_notify();
            let _ = stream.flush();
        });
        (port, server)
    }

    fn get(
        versions: &'static [&'static SupportedProtocolVersion],
        pins: Vec<CertificatePin>,
    ) -> Result<String, ureq::Error> {
        let (port, server) = serve_once(versions);
        let timeout = Duration::from_secs(5);
        let result = scanner_agent_with_pins(timeout, timeout, timeout, None, pins)
            .get(&format!(
                "https://127.0.0.1:{port}/eSCL/ScannerCapabilities"
            ))
            .call()
            .and_then(|response| response.into_body().read_to_string());
        server.join().unwrap();
        result
    }

    #[test]
    fn pinned_self_signed_certificate_completes_tls12_and_tls13_handshakes() {
        const TLS12_ONLY: &[&SupportedProtocolVersion] = &[&rustls::version::TLS12];
        const TLS13_ONLY: &[&SupportedProtocolVersion] = &[&rustls::version::TLS13];
        for versions in [TLS12_ONLY, TLS13_ONLY] {
            let body = get(versions, vec![pin("127.0.0.1", None, fingerprint())]).unwrap();
            assert_eq!(body, "ok");
        }
        let body = get(
            rustls::ALL_VERSIONS,
            vec![
                pin("127.0.0.1", Some(1), [0; 32]),
                pin("127.0.0.1", None, fingerprint()),
            ],
        );
        assert_eq!(body.unwrap(), "ok");
    }

    #[test]
    fn mismatched_pin_reports_the_presented_fingerprint() {
        let error = get(rustls::ALL_VERSIONS, vec![pin("127.0.0.1", None, [0; 32])]).unwrap_err();
        assert_eq!(
            certificate_pin_mismatch(&error),
            Some(format!(
                "certificate for 127.0.0.1 does not match the pinned SHA-256 fingerprint (got {TEST_CERT_SHA256})"
            ))
        );
    }

    #[test]
    fn unpinned_hosts_keep_webpki_verification() {
        for pins in [
            Vec::new(),
            vec![pin("127.0.0.2", None, fingerprint())],
            vec![pin("127.0.0.1", Some(1), fingerprint())],
        ] {
            let error = get(rustls::ALL_VERSIONS, pins).unwrap_err();
            assert!(
                matches!(
                    rustls_error(&error),
                    Some(rustls::Error::InvalidCertificate(_))
                ),
                "{error}"
            );
            assert_eq!(certificate_pin_mismatch(&error), None);
        }
    }
}
