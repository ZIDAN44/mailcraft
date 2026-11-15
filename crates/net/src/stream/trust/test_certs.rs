//! Certificates for the trust tests: an internal CA as a company's IT runs one, an issuing CA
//! under it, servers they signed (with their keys, so a test can stand in for the server), and
//! bytes no store can use. Made up for MailCraft's tests, valid until 2126, for `mail.corp.test`
//! and 127.0.0.1; the CAs' keys were thrown away. Then two servers' own certificates (their keys
//! thrown away too): one out of date, one made by hand; and one for an address to write to.

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

/// A server's own certificate that ran out on 1 January 2021, for `old.corp.test`.
const EXPIRED: &str = "-----BEGIN CERTIFICATE-----
MIIBoDCCAUWgAwIBAgIUATJUu6Qv7Cal3qkdTtwO6E+xfNcwCgYIKoZIzj0EAwIw
GDEWMBQGA1UEAwwNb2xkLmNvcnAudGVzdDAeFw0yMDAxMDEwMDAwMDBaFw0yMTAx
MDEwMDAwMDBaMBgxFjAUBgNVBAMMDW9sZC5jb3JwLnRlc3QwWTATBgcqhkjOPQIB
BggqhkjOPQMBBwNCAARd1XQya38XKtC0QFaF3Qa7A7j7rnjBGf0k1sSZN/XYq2SB
L186jZ9hFqYLa3Z6caluOJwLwGntNZ0c/jP2qDrho20wazAdBgNVHQ4EFgQUhj7v
dI2SoNY66s/baTzOJS+TBTMwHwYDVR0jBBgwFoAUhj7vdI2SoNY66s/baTzOJS+T
BTMwDwYDVR0TAQH/BAUwAwEB/zAYBgNVHREEETAPgg1vbGQuY29ycC50ZXN0MAoG
CCqGSM49BAMCA0kAMEYCIQC2FQ7GbxJ8+gmAysZe63u990sjkigMYtdKZb7aQrNj
zAIhANeIckwrfk3gf5A787HXI1Kjjy+TT+i9hlkPlOTc9mtH
-----END CERTIFICATE-----";

/// A server's own certificate as `openssl req -x509` makes one by hand: for `home.lan` in its
/// common name alone, saying it's an authority, valid until 2126.
const HAND_MADE: &str = "-----BEGIN CERTIFICATE-----
MIIBfTCCASOgAwIBAgIUPU1r+e+KwYCaKp01AKtQukNtuT0wCgYIKoZIzj0EAwIw
EzERMA8GA1UEAwwIaG9tZS5sYW4wIBcNMjYxMTE1MjExNTM5WhgPMjEyNjEwMjIy
MTE1MzlaMBMxETAPBgNVBAMMCGhvbWUubGFuMFkwEwYHKoZIzj0CAQYIKoZIzj0D
AQcDQgAEjb5bZifajgwQzypNdk/R5yVNilawdn8VkEVHLTVtmYVWWkg/mOPfdwBW
/+QN300tNlW6m28zmjSy6FtG+yPMlqNTMFEwHQYDVR0OBBYEFJOK5+J1UnapI8C2
txuPEFIP8yVOMB8GA1UdIwQYMBaAFJOK5+J1UnapI8C2txuPEFIP8yVOMA8GA1Ud
EwEB/wQFMAMBAf8wCgYIKoZIzj0EAwIDSAAwRQIhAO8WcO4D8DmWac9XDvbS+IhX
LIYgYnSA4eRWoFxUcL+sAiAMpfW2k21IPyQJY/wKLkIZzIJnZdDt00jrpFpKUPaR
hA==
-----END CERTIFICATE-----";

/// A certificate whose only alternative name is an address to write to, its common name a
/// server's: for no server.
const EMAIL_ONLY: &str = "-----BEGIN CERTIFICATE-----
MIIBoTCCAUigAwIBAgIUb/mY+SzNJlB3WKFfSDRTThEyvxAwCgYIKoZIzj0EAwIw
GTEXMBUGA1UEAwwObWFpbC5jb3JwLnRlc3QwIBcNMjYxMTE1MjE0NzA4WhgPMjEy
NjEwMjIyMTQ3MDhaMBkxFzAVBgNVBAMMDm1haWwuY29ycC50ZXN0MFkwEwYHKoZI
zj0CAQYIKoZIzj0DAQcDQgAEstXCpTjdYemhPPJU+LBiz9Asin4JLcnMTNd5TX2f
yBOALB1e0o0JZMQs7RPTPKN8xd/UZTaTndCXtbt6ofWcYaNsMGowHQYDVR0OBBYE
FL3mtuXvqBw37hp1DaN0DsVDS5Q5MB8GA1UdIwQYMBaAFL3mtuXvqBw37hp1DaN0
DsVDS5Q5MA8GA1UdEwEB/wQFMAMBAf8wFwYDVR0RBBAwDoEMaXRAY29ycC50ZXN0
MAoGCCqGSM49BAMCA0cAMEQCIGR82E34tl7N0Ea8TGg31zL/vnr0i89gn8NLVAhE
PNdUAiAapW+hHdUu+pgb5VHLDp9yvZxF7KpFmz46vYTvPC6iVA==
-----END CERTIFICATE-----";

pub(crate) fn email_only() -> CertificateDer<'static> {
    cert(EMAIL_ONLY)
}

pub(crate) fn expired() -> CertificateDer<'static> {
    cert(EXPIRED)
}

pub(crate) fn hand_made() -> CertificateDer<'static> {
    cert(HAND_MADE)
}

/// What a damaged entry in a system store reads as.
pub(crate) fn unreadable() -> CertificateDer<'static> {
    CertificateDer::from(vec![0u8, 0, 0])
}
