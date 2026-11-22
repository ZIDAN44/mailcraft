//! Kerberos sign-in for on-premises Exchange (gap 5): HTTP Negotiate (RFC 4559) with a Kerberos
//! token, for a server that takes nothing else. sspi (Devolutions, MIT / Apache-2.0) builds the
//! tokens; MailCraft talks to the KDC itself over TCP ([`Kdc`]), found as Active Directory
//! publishes it (`_kerberos._tcp.<realm>` in DNS), or at `MAILCRAFT_KDC` (`host[:port]`).

use std::io::{Read, Write};
use std::net::{TcpStream, ToSocketAddrs};
use std::time::Duration;

use sspi::{
    AuthIdentity, BufferType, ClientRequestFlags, CredentialUse, DataRepresentation, Kerberos, KerberosConfig, SecurityBuffer, SecurityStatus, Sspi,
    SspiImpl, Username,
};

use crate::{NetError, Result};

/// Largest answer a KDC may send.
const MAX_KDC_ANSWER: usize = 1 << 20;
/// How long a KDC has to connect and answer.
const KDC_TIMEOUT: Duration = Duration::from_secs(10);

/// One Kerberos sign-in in progress: the context and the credentials it uses.
pub struct Negotiation {
    kerberos: Kerberos,
    creds: <Kerberos as SspiImpl>::CredentialsHandle,
    target: String,
    /// The TLS channel's binding as Windows lays it out, for a server that checks it (issue
    /// #249: Exchange's Extended Protection).
    bindings: Option<Vec<u8>>,
}

/// The Kerberos principal for a user name as it's typed (`user@corp.example`, `CORP\user`, or a
/// bare name), with the account's address for the realm when the name doesn't give one: (user,
/// realm in capitals).
pub fn principal(user: &str, email: &str) -> Option<(String, String)> {
    let user = user.trim();
    let domain_of = |a: &str| a.rsplit_once('@').map(|(_, d)| d.trim().to_string()).filter(|d| d.contains('.'));
    let (name, realm) = if let Some((n, r)) = user.split_once('@') {
        (n.to_string(), r.to_string())
    } else if let Some((_, n)) = user.split_once('\\') {
        // A NetBIOS domain isn't a realm: the address's domain is the guess Active Directory
        // usually bears out.
        (n.to_string(), domain_of(email)?)
    } else {
        (user.to_string(), domain_of(email)?)
    };
    let ok = |s: &str| !s.is_empty() && s.len() <= 255 && !s.chars().any(|c| c.is_control() || c.is_whitespace() || c == '@' || c == '\\');
    (ok(&name) && ok(&realm)).then(|| (name, realm.to_ascii_uppercase()))
}

/// Where the realm's KDC is: `MAILCRAFT_KDC` when set, else the realm's DNS SRV record.
pub fn kdc_for(realm: &str) -> Option<String> {
    if let Some(k) = std::env::var("MAILCRAFT_KDC").ok().map(|k| k.trim().to_string()).filter(|k| !k.is_empty()) {
        return Some(if k.contains(':') { k } else { format!("{k}:88") });
    }
    crate::dns::srv(&format!("_kerberos._tcp.{}", realm.to_ascii_lowercase())).map(|(host, port)| format!("{host}:{port}"))
}

impl Negotiation {
    /// Start signing in to `host` (its service principal `HTTP/host`) as `user` (see
    /// [`principal`]): the first token to send, or why not (no realm, no KDC, refused).
    /// `binding`: the TLS channel's (`tls-server-end-point`), over HTTPS.
    pub fn start(user: &str, email: &str, password: &str, host: &str, binding: Option<&[u8]>) -> Result<(Negotiation, Vec<u8>)> {
        let bindings = binding.map(crate::stream::binding::sec_channel_bindings);
        let (name, realm) = principal(user, email).ok_or_else(|| NetError::Auth("Kerberos needs a user name with its domain".into()))?;
        let kdc = kdc_for(&realm).ok_or_else(|| NetError::No(format!("no Kerberos server (KDC) found for {realm}")))?;
        // The plain principal (name and realm), which every KDC knows; a name typed with its
        // domain (`riley@corp.example`) is asked again as Active Directory's enterprise name
        // when the KDC doesn't know the plain one (a UPN unlike the account name).
        let plain = Username::new_down_level_logon_name(&name, &realm).map_err(|e| sspi_error(e.into()))?;
        match Negotiation::with(plain, &kdc, password, host, bindings.clone()) {
            Err(f) if f.why == Why::ClientNotFound && user.contains('@') => {
                Ok(Negotiation::with(Username::parse(user.trim()).map_err(|e| sspi_error(e.into()))?, &kdc, password, host, bindings)?)
            }
            r => Ok(r?),
        }
    }

    fn with(
        username: Username,
        kdc: &str,
        password: &str,
        host: &str,
        bindings: Option<Vec<u8>>,
    ) -> std::result::Result<(Negotiation, Vec<u8>), Failure> {
        let computer = std::env::var("COMPUTERNAME").or_else(|_| std::env::var("HOSTNAME")).unwrap_or_else(|_| "mailcraft".into());
        let mut kerberos = Kerberos::new_client_from_config(KerberosConfig::new_with_kdc_url(&format!("tcp://{kdc}"), computer)).map_err(failure)?;
        let identity = AuthIdentity { username, password: password.to_string().into() };
        let creds = kerberos
            .acquire_credentials_handle()
            .with_credential_use(CredentialUse::Outbound)
            .with_auth_data(&identity.into())
            .execute(&mut kerberos)
            .map_err(failure)?
            .credentials_handle;
        let mut n = Negotiation { kerberos, creds, target: format!("HTTP/{}", host.to_ascii_lowercase()), bindings };
        let (token, _) = n.next(&[])?;
        Ok((n, token))
    }

    /// The next token for the server's answer (`input`), and whether the context is complete.
    pub fn step(&mut self, input: &[u8]) -> Result<(Vec<u8>, bool)> {
        Ok(self.next(input)?)
    }

    /// [`Negotiation::step`], telling what went wrong.
    fn next(&mut self, input: &[u8]) -> std::result::Result<(Vec<u8>, bool), Failure> {
        let mut input = vec![SecurityBuffer::new(input.to_vec(), BufferType::Token)];
        if let Some(b) = &self.bindings {
            input.push(SecurityBuffer::new(b.clone(), BufferType::ChannelBindings));
        }
        let mut output = vec![SecurityBuffer::new(Vec::new(), BufferType::Token)];
        let mut builder = self
            .kerberos
            .initialize_security_context()
            .with_credentials_handle(&mut self.creds)
            .with_context_requirements(ClientRequestFlags::MUTUAL_AUTH)
            .with_target_data_representation(DataRepresentation::Native)
            .with_target_name(&self.target)
            .with_input(&mut input)
            .with_output(&mut output);
        let result = self.kerberos.initialize_security_context_impl(&mut builder).map_err(failure)?.resolve_with_client(&Kdc).map_err(failure)?;
        let token = output.into_iter().next().map(|b| b.buffer).unwrap_or_default();
        match result.status {
            SecurityStatus::Ok => Ok((token, true)),
            SecurityStatus::ContinueNeeded => Ok((token, false)),
            s => Err(Failure { why: Why::Other, error: NetError::Auth(format!("Kerberos: {s:?}")) }),
        }
    }
}

/// What went wrong, as sspi tells it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Why {
    /// The KDC refused the password (pre-authentication failed).
    BadPassword,
    /// The KDC knows no user by this name.
    ClientNotFound,
    /// The domain won't sign this user in (revoked, disabled, expired).
    Refused,
    /// No KDC answered.
    NoKdc,
    Other,
}

/// A failure to sign in: what it was, and what the user is told.
struct Failure {
    why: Why,
    error: NetError,
}

impl From<Failure> for NetError {
    fn from(f: Failure) -> NetError {
        f.error
    }
}

/// What sspi's failure was: the KDC's refusals are known by the words sspi tells them with, a KDC
/// that can't be reached by its kind. Only here are its words read.
fn why(e: &sspi::Error) -> Why {
    let d = e.description.to_ascii_lowercase();
    if d.contains("pre-authentication") || d.contains("preauth") {
        Why::BadPassword
    } else if d.contains("client not found") {
        Why::ClientNotFound
    } else if d.contains("revoked") || d.contains("disabled") || d.contains("expired") {
        Why::Refused
    } else if e.error_type == sspi::ErrorKind::NoAuthenticatingAuthority {
        Why::NoKdc
    } else {
        Why::Other
    }
}

/// What sspi says went wrong: a wrong password or unknown user is a sign-in to correct, said
/// plainly (the KDC's own words come with encoded data).
fn failure(e: sspi::Error) -> Failure {
    let why = why(&e);
    let said = short(&e.description);
    let error = match why {
        Why::BadPassword => NetError::Auth("the domain refused the password (Kerberos)".into()),
        Why::ClientNotFound => NetError::Auth("the domain has no user by this name (Kerberos, CLIENT_NOT_FOUND)".into()),
        Why::Refused => NetError::Auth(format!("the domain won't sign this user in (Kerberos: {said})")),
        Why::NoKdc => NetError::Closed(format!("the Kerberos server (KDC) didn't answer: {said}")),
        Why::Other => NetError::Auth(format!("Kerberos: {said}")),
    };
    Failure { why, error }
}

fn sspi_error(e: sspi::Error) -> NetError {
    failure(e).error
}

/// sspi's description up to the encoded data it appends, at most a line.
fn short(d: &str) -> String {
    d.split(". Additional error").next().unwrap_or(d).chars().take(200).collect()
}

/// The KDC over TCP (RFC 4120 7.2.2: each message after its length), its answer capped.
struct Kdc;

impl sspi::network_client::NetworkClient for Kdc {
    fn send(&self, request: &sspi::generator::NetworkRequest) -> sspi::Result<Vec<u8>> {
        let fail = |m: String| sspi::Error::new(sspi::ErrorKind::NoAuthenticatingAuthority, m);
        if !matches!(request.protocol, sspi::network_client::NetworkProtocol::Tcp) {
            return Err(fail(format!("the KDC is reached over TCP only, not {:?}", request.protocol)));
        }
        let host = request.url.host_str().unwrap_or_default();
        let port = request.url.port().unwrap_or(88);
        let addrs = (host, port).to_socket_addrs().map_err(|e| fail(e.to_string()))?;
        let mut last = fail(format!("{host}:{port} has no address"));
        for addr in addrs {
            match TcpStream::connect_timeout(&addr, KDC_TIMEOUT) {
                Ok(mut s) => {
                    let mut exchange = || -> std::io::Result<Vec<u8>> {
                        s.set_read_timeout(Some(KDC_TIMEOUT))?;
                        s.set_write_timeout(Some(KDC_TIMEOUT))?;
                        s.write_all(&request.data)?;
                        let mut len = [0u8; 4];
                        s.read_exact(&mut len)?;
                        let n = usize::try_from(u32::from_be_bytes(len)).unwrap_or(usize::MAX);
                        if n > MAX_KDC_ANSWER {
                            return Err(std::io::Error::other("the KDC's answer is too large"));
                        }
                        let mut out = len.to_vec();
                        out.resize(4 + n, 0);
                        s.read_exact(out.get_mut(4..).unwrap_or_default())?;
                        Ok(out)
                    };
                    return exchange().map_err(|e| fail(e.to_string()));
                }
                Err(e) => last = fail(e.to_string()),
            }
        }
        Err(last)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn principals_from_user_names() {
        assert_eq!(principal("riley@corp.example", "x@y.example"), Some(("riley".into(), "CORP.EXAMPLE".into())));
        assert_eq!(principal("CORP\\riley", "riley@corp.example"), Some(("riley".into(), "CORP.EXAMPLE".into())));
        assert_eq!(principal("riley", "riley@contoso.example"), Some(("riley".into(), "CONTOSO.EXAMPLE".into())));
        for bad in ["", "@corp.example", "riley@", "ri ley@corp.example", "riley\r@corp.example"] {
            assert_eq!(principal(bad, "x@y.example"), None, "{bad:?}");
        }
        assert_eq!(principal("CORP\\riley", "no-domain"), None, "no realm to guess");
    }

    /// sspi's failures as the user is told them.
    #[test]
    fn sspi_failures_are_told_plainly() {
        use sspi::ErrorKind as K;
        let cases = [
            (
                K::InternalError,
                "KDC_ERR_PREAUTH_FAILED: Pre-authentication information was invalid",
                "sign-in failed: the domain refused the password (Kerberos)",
            ),
            (
                K::InternalError,
                "KDC_ERR_C_PRINCIPAL_UNKNOWN: Client not found in Kerberos database. Additional error data: 3081",
                "sign-in failed: the domain has no user by this name (Kerberos, CLIENT_NOT_FOUND)",
            ),
            (
                K::InternalError,
                "KDC_ERR_CLIENT_REVOKED: Clients credentials have been revoked. Additional error data: 3081",
                "sign-in failed: the domain won't sign this user in (Kerberos: KDC_ERR_CLIENT_REVOKED: Clients credentials have been revoked)",
            ),
            (K::LogonDenied, "denied. Additional error data: 30", "sign-in failed: Kerberos: denied"),
            (K::UnknownCredentials, "no such", "sign-in failed: Kerberos: no such"),
            (
                K::NoAuthenticatingAuthority,
                "connection refused",
                "the server closed the connection: the Kerberos server (KDC) didn't answer: connection refused",
            ),
            (K::InternalError, "something else", "sign-in failed: Kerberos: something else"),
        ];
        let whys = [Why::BadPassword, Why::ClientNotFound, Why::Refused, Why::Other, Why::Other, Why::NoKdc, Why::Other];
        for ((kind, description, told), why) in cases.into_iter().zip(whys) {
            let f = failure(sspi::Error::new(kind, description));
            assert_eq!((f.why, f.error.to_string().as_str()), (why, told), "{description}");
        }
    }
}
