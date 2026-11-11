// src/executor/tls.rs
//
// سیاست TLS به‌صورت یک تصمیم مستقل و قابل‌توسعه، جدا از هر executor خاص.
// الان فقط `Insecure` پیاده‌سازی شده (رفتار فعلی RawHttpExecutor، طبق
// تصمیم صریح کاربر). `SystemCa` و `CustomCa` برای فاز بعد رزرو شده‌اند
// تا وقتی اضافه بشن، لازم نباشه RawHttpExecutor دوباره بازنویسی بشه —
// فقط یک match-arm جدید اینجا اضافه می‌شه.
use std::path::PathBuf;
use std::sync::{Arc, Once};

use tokio_rustls::rustls::{
    self,
    client::danger::{HandshakeSignatureValid, ServerCertVerified, ServerCertVerifier},
    pki_types::{CertificateDer, ServerName, UnixTime},
    DigitallySignedStruct, SignatureScheme,
};
use tokio_rustls::TlsConnector;

/// سیاست اعتبارسنجی گواهیِ هدف HTTPS.
///
/// فقط `Insecure` فعلاً پیاده‌سازی شده. بقیه عمداً `#[allow(dead_code)]`
/// دارن تا کامپایلر درباره‌شون شکایت نکنه، بدون اینکه نیاز باشه حذفشون
/// کنیم یا جلوتر از تصمیم معماری پیاده‌سازی‌شون کنیم.
#[derive(Debug, Clone)]
#[allow(dead_code)]
pub enum TlsVerification {
    /// رفتار فعلی: زنجیره و هاست‌نیم گواهی چک نمی‌شه (برای هدف‌های
    /// self-signed/داخلیِ آزمایشگاه). امضای handshake همچنان با
    /// الگوریتم‌های واقعی provider بررسی می‌شه.
    Insecure,
    /// فاز بعد: اعتماد به ریشه‌های سیستم + اعتبارسنجی کامل hostname/زنجیره.
    SystemCa,
    /// فاز بعد: اعتماد به یک CA اختصاصی (مسیر PEM)، با حفظ اعتبارسنجی
    /// hostname/زنجیره در برابر همون CA.
    CustomCa(PathBuf),
}

impl Default for TlsVerification {
    fn default() -> Self {
        // پیش‌فرض فعلی پروژه برای RawHttpExecutor. تغییر این پیش‌فرض یک
        // تصمیم معماری جداست و باید در Decision log ثبت بشه.
        TlsVerification::Insecure
    }
}

/// verifier برای `TlsVerification::Insecure`: زنجیره و هاست‌نیم گواهی رو
/// چک نمی‌کنه، چون هدف‌ها معمولاً self-signed یا internal CA هستن و این
/// ابزار علیه محیط‌های کنترل‌شده‌ی خودِ کاربر اجرا می‌شه، نه ترافیک کاربر
/// نهایی. امضای handshake (TLS12/13) همچنان با الگوریتم‌های provider
/// واقعی چک می‌شه — یعنی فقط اعتماد به CA/هاست غیرفعاله، نه کل هندشیک.
#[derive(Debug)]
struct AcceptAnyServerCert(Arc<rustls::crypto::CryptoProvider>);

impl ServerCertVerifier for AcceptAnyServerCert {
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
            &self.0.signature_verification_algorithms,
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
            &self.0.signature_verification_algorithms,
        )
    }

    fn supported_verify_schemes(&self) -> Vec<SignatureScheme> {
        self.0.signature_verification_algorithms.supported_schemes()
    }
}

static INSTALL_CRYPTO_PROVIDER: Once = Once::new();

/// نصب CryptoProvider سراسری فرایند، دقیقاً یک‌بار — صرف‌نظر از اینکه
/// چند تا RawHttpExecutor یا چند بار این تابع صدا زده بشه. قبلاً این کار
/// توی constructor هر executor تکرار می‌شد (حتی برای http که اصلاً TLS
/// لازم نداره)؛ الان هم idempotent شده هم از مسیر http جدا شده، چون این
/// تابع فقط از build_connector صدا زده می‌شه که خودش فقط lazy و فقط
/// برای https صدا زده می‌شه.
fn ensure_crypto_provider_installed() {
    INSTALL_CRYPTO_PROVIDER.call_once(|| {
        // reqwest (برای HttpTargetExecutor) هم همین provider رو نصب
        // می‌کنه؛ اگه او زودتر نصب کرده باشه، install_default خطا
        // می‌ده که این‌جا بی‌ضرر نادیده گرفته می‌شه.
        let _ = rustls::crypto::ring::default_provider().install_default();
    });
}

/// ساخت TlsConnector برای یک سیاست مشخص. این تابع فقط وقتی باید صدا زده
/// بشه که واقعاً یک اتصال https در پیش است (lazy) — نه در ساخت هر executor.
pub fn build_connector(policy: &TlsVerification) -> TlsConnector {
    ensure_crypto_provider_installed();

    match policy {
        TlsVerification::Insecure => {
            let provider = Arc::new(rustls::crypto::ring::default_provider());
            let config = rustls::ClientConfig::builder()
                .dangerous()
                .with_custom_certificate_verifier(Arc::new(AcceptAnyServerCert(provider)))
                .with_no_client_auth();
            TlsConnector::from(Arc::new(config))
        }
        TlsVerification::SystemCa | TlsVerification::CustomCa(_) => {
            // عمداً پیاده‌سازی نشده: فاز بعد. تا اون موقع هیچ executor ای
            // این variantها رو نباید بسازه.
            unimplemented!(
                "TlsVerification::{:?} is reserved for the System/Custom CA phase \
                 and is not wired into any executor yet",
                policy
            )
        }
    }
}
