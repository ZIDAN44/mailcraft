//! Certificates for the trust tests: an internal CA as a company's IT runs one, an issuing CA
//! under it, servers they signed (with their keys, so a test can stand in for the server), and
//! bytes no store can use. Made up for MailCraft's tests, valid until 2126, for `mail.corp.test`
//! and 127.0.0.1; the CAs' keys were thrown away.

use rustls::pki_types::pem::PemObject;
use rustls::pki_types::{CertificateDer, PrivateKeyDer};

const INTERNAL_CA: &str = "-----BEGIN CERTIFICATE-----
MIIBsDCCAVegAwIBAgIUWtdF1YzkvG0MciS3AbXMuleseWcwCgYIKoZIzj0EAwIw
JTEjMCEGA1UEAwwaTWFpbENyYWZ0IFRlc3QgSW50ZXJuYWwgQ0EwIBcNMjYxMTA0
MjAwNjU3WhgPMjEyNjEwMTEyMDA2NTdaMCUxIzAhBgNVBAMMGk1haWxDcmFmdCBU
ZXN0IEludGVybmFsIENBMFkwEwYHKoZIzj0CAQYIKoZIzj0DAQcDQgAE51BeS15b
1cjK0hRCAhWrVfTQ5VZLW5tmZ7fP+cyQMuuflHYb2Wxl+JYlmFNV1JwVVBrSi/3/
rOSlYQNQLzpO7KNjMGEwHQYDVR0OBBYEFIP4fiWnxPcuij9xrsQhdlpxi/ABMB8G
A1UdIwQYMBaAFIP4fiWnxPcuij9xrsQhdlpxi/ABMA8GA1UdEwEB/wQFMAMBAf8w
DgYDVR0PAQH/BAQDAgIEMAoGCCqGSM49BAMCA0cAMEQCIHrpZf/NDu6lDDEpmWrd
lE4owBPA/KRXrV5IK/KDrNYSAiAfXXrCofoIUXVYSb3XOrQALvtYyCn5BAs6gtaJ
V78zWQ==
-----END CERTIFICATE-----";

/// An intermediate CA, signed by [`INTERNAL_CA`].
const ISSUING_CA: &str = "-----BEGIN CERTIFICATE-----
MIIBszCCAVmgAwIBAgIUf5bwuftXZCbaBNoiGLz7OGVoG+EwCgYIKoZIzj0EAwIw
JTEjMCEGA1UEAwwaTWFpbENyYWZ0IFRlc3QgSW50ZXJuYWwgQ0EwIBcNMjYxMTA0
MjE0NjMxWhgPMjEyNjEwMTEyMTQ2MzFaMCQxIjAgBgNVBAMMGU1haWxDcmFmdCBU
ZXN0IElzc3VpbmcgQ0EwWTATBgcqhkjOPQIBBggqhkjOPQMBBwNCAAS2cJRYMPj1
OGZ9NUAu8R1ppt6SoITMwjQbvqzP94kbEMhZ/hrnu2SdlOzXdedHDzDHZPXxiUqv
pNzi3SpMlG2co2YwZDASBgNVHRMBAf8ECDAGAQH/AgEAMA4GA1UdDwEB/wQEAwIC
BDAdBgNVHQ4EFgQUmzSgrERDqrBZhbadIyBIgLbwhgEwHwYDVR0jBBgwFoAUg/h+
JafE9y6KP3GuxCF2WnGL8AEwCgYIKoZIzj0EAwIDSAAwRQIgLKiJbYHM3mqQrno8
9J181W3nAgjIQmh5LgdylEBueOwCIQD+xqmQw8ML2ZtDzgmVdRhZhX1SU40jA57G
fnf75hmxfw==
-----END CERTIFICATE-----";

/// A server signed by [`INTERNAL_CA`] itself.
const SERVER: &str = "-----BEGIN CERTIFICATE-----
MIIBxzCCAW2gAwIBAgIUf5bwuftXZCbaBNoiGLz7OGVoG+AwCgYIKoZIzj0EAwIw
JTEjMCEGA1UEAwwaTWFpbENyYWZ0IFRlc3QgSW50ZXJuYWwgQ0EwIBcNMjYxMTA0
MjE0NjMxWhgPMjEyNjEwMTEyMTQ2MzFaMBkxFzAVBgNVBAMMDm1haWwuY29ycC50
ZXN0MFkwEwYHKoZIzj0CAQYIKoZIzj0DAQcDQgAERkKccnBHzmiaNK4VmA5c70Eq
lYdlBBnDico2u3Cd5SCOe7k/Z5vHfG8Tjwo01HXHk8HIVwensfdorB5jqEVZxKOB
hDCBgTAfBgNVHREEGDAWgg5tYWlsLmNvcnAudGVzdIcEfwAAATAJBgNVHRMEAjAA
MBMGA1UdJQQMMAoGCCsGAQUFBwMBMB0GA1UdDgQWBBSk3Lv352Bf15e0ueTwKTuR
TbicYzAfBgNVHSMEGDAWgBSD+H4lp8T3Loo/ca7EIXZacYvwATAKBggqhkjOPQQD
AgNIADBFAiEAxAXXM+u3lYEyRRpAOl7USZ41FvXCqiQe62bAGZYzIZsCIB97dB/n
XMV8b7tRRyfemzhZ7TopnCqMwau6RQA70wcf
-----END CERTIFICATE-----";

/// [`SERVER`]'s key: made up for these tests, never used anywhere else.
const SERVER_KEY: &str = "-----BEGIN PRIVATE KEY-----
MIGHAgEAMBMGByqGSM49AgEGCCqGSM49AwEHBG0wawIBAQQgQ6kGmxgXG4n8rqu1
8ZdB+RpFgCeDpjTX4mIb6WUr1RehRANCAARGQpxycEfOaJo0rhWYDlzvQSqVh2UE
GcOJyja7cJ3lII57uT9nm8d8bxOPCjTUdceTwchXB6ex92isHmOoRVnE
-----END PRIVATE KEY-----";

/// A server signed by [`ISSUING_CA`]: it sends that CA along with its own certificate.
const VIA_ISSUING: &str = "-----BEGIN CERTIFICATE-----
MIIBxzCCAWygAwIBAgIUKWpor1/20HaBIUG2joGnKLPaOhAwCgYIKoZIzj0EAwIw
JDEiMCAGA1UEAwwZTWFpbENyYWZ0IFRlc3QgSXNzdWluZyBDQTAgFw0yNjExMDQy
MTQ2MzFaGA8yMTI2MTAxMTIxNDYzMVowGTEXMBUGA1UEAwwObWFpbC5jb3JwLnRl
c3QwWTATBgcqhkjOPQIBBggqhkjOPQMBBwNCAARkrfuXGAfFnx/TAw5JBzTlN6wz
SmpYeFSatcbil/4c5CUeW/hjayP2kf76Hvb6JtZjF1FLL4gdxFKkX++ZL58Go4GE
MIGBMB8GA1UdEQQYMBaCDm1haWwuY29ycC50ZXN0hwR/AAABMAkGA1UdEwQCMAAw
EwYDVR0lBAwwCgYIKwYBBQUHAwEwHQYDVR0OBBYEFHMzk4DsSAOX5z4HyGjSWs76
EdN5MB8GA1UdIwQYMBaAFJs0oKxEQ6qwWYW2nSMgSIC28IYBMAoGCCqGSM49BAMC
A0kAMEYCIQDofJzQd7N02eUsNnIQqLx6wzEZ23NeuhMjWdTCvms1MQIhAKuHWNqk
+MmUutRO5AuzhTn+GfUUVXY5vklHQx/UQwD6
-----END CERTIFICATE-----";

/// [`VIA_ISSUING`]'s key: made up for these tests, never used anywhere else.
const VIA_ISSUING_KEY: &str = "-----BEGIN PRIVATE KEY-----
MIGHAgEAMBMGByqGSM49AgEGCCqGSM49AwEHBG0wawIBAQQgA+63eFdrxGaUUHeU
mUmPBk5yDS+ignC97owD8k92PrShRANCAARkrfuXGAfFnx/TAw5JBzTlN6wzSmpY
eFSatcbil/4c5CUeW/hjayP2kf76Hvb6JtZjF1FLL4gdxFKkX++ZL58G
-----END PRIVATE KEY-----";

fn cert(pem: &str) -> CertificateDer<'static> {
    CertificateDer::from_pem_slice(pem.as_bytes()).unwrap()
}

pub(crate) fn internal_ca() -> CertificateDer<'static> {
    cert(INTERNAL_CA)
}

pub(crate) fn issuing_ca() -> CertificateDer<'static> {
    cert(ISSUING_CA)
}

pub(crate) fn server() -> CertificateDer<'static> {
    cert(SERVER)
}

/// [`server`]'s chain and key, as a TLS server presents them.
pub(crate) fn server_identity() -> (Vec<CertificateDer<'static>>, PrivateKeyDer<'static>) {
    (vec![server()], PrivateKeyDer::from_pem_slice(SERVER_KEY.as_bytes()).unwrap())
}

pub(crate) fn via_issuing() -> CertificateDer<'static> {
    cert(VIA_ISSUING)
}

/// The server under the issuing CA: its certificate, then the issuing CA's, and its key.
pub(crate) fn via_issuing_identity() -> (Vec<CertificateDer<'static>>, PrivateKeyDer<'static>) {
    (vec![via_issuing(), issuing_ca()], PrivateKeyDer::from_pem_slice(VIA_ISSUING_KEY.as_bytes()).unwrap())
}

/// What a damaged entry in a system store reads as.
pub(crate) fn unreadable() -> CertificateDer<'static> {
    CertificateDer::from(vec![0u8, 0, 0])
}
