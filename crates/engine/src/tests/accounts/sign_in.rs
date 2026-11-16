//! How an account's servers are signed in to (issue #123): chosen as it's added or changed, kept
//! with it, and refused where there's nothing to choose.

use super::*;
use mailcraft_model::SignInMethod;

/// NTLM for the incoming server and no sign-in for the outgoing one, as a company's own Exchange
/// may want: kept with the account (in its settings file, never "automatic"), a change
/// reconnecting with it, and left as it was when not given.
#[test]
fn sign_in_methods_are_chosen_kept_and_reconnected_with() {
    let mut s = sample_session("sign-in-methods");
    let add = json!({"email": "riley@corp.example", "password": "pw", "username": "CORP\\riley", "imapHost": "mail.corp.example",
        "smtpHost": "mail.corp.example", "signIn": "ntlm", "smtpSignIn": "none"});
    let id = s.run("account.add", &add).unwrap()["account"].as_str().unwrap().to_string();
    let methods = |s: &Session| s.account(&id).map(|a| (a.config.sign_in, a.config.smtp_sign_in));
    assert_eq!(methods(&s), Some((SignInMethod::Ntlm, SignInMethod::None)));
    assert!(matches!(s.take_jobs().as_slice(), [Job::Start { account, .. }] if account.sign_in == SignInMethod::Ntlm));
    let file_path = s.store.as_ref().unwrap().root().join("accounts.json");
    let saved = || std::fs::read_to_string(&file_path).unwrap();
    let file = saved();
    assert!(file.contains("\"ntlm\"") && file.contains("\"none\""), "{file}");
    // Changed: the account reconnects with it.
    s.run("account.update", &json!({"account": id, "smtpSignIn": "password"})).unwrap();
    assert_eq!(methods(&s), Some((SignInMethod::Ntlm, SignInMethod::Password)));
    assert!(matches!(s.take_jobs().as_slice(), [Job::Start { account, .. }] if account.smtp_sign_in == SignInMethod::Password));
    // Not given: as it was.
    s.run("account.update", &json!({"account": id, "name": "Riley"})).unwrap();
    assert_eq!(methods(&s), Some((SignInMethod::Ntlm, SignInMethod::Password)));
    // Unknown, or no sign-in for the incoming server: refused, nothing changed.
    for bad in [json!({"account": id, "signIn": "kerberos"}), json!({"account": id, "signIn": "none"}), json!({"account": id, "smtpSignIn": 3})] {
        let refused = s.run("account.update", &bad);
        assert!(refused.is_err() || bad["smtpSignIn"] == json!(3), "{bad}");
        assert_eq!(methods(&s), Some((SignInMethod::Ntlm, SignInMethod::Password)), "{bad}");
    }
    // Back to automatic: nothing written for it.
    s.run("account.update", &json!({"account": id, "signIn": "automatic", "smtpSignIn": "automatic"})).unwrap();
    assert_eq!(methods(&s), Some((SignInMethod::Automatic, SignInMethod::Automatic)));
    assert!(!saved().contains("sign_in"), "{}", saved());
}

/// Accounts that don't sign in to IMAP, POP or SMTP with a password have no method to choose: an
/// Exchange server through EWS (NTLM already, by itself), JMAP, one signing in with Microsoft.
#[test]
fn only_password_accounts_choose_how_they_sign_in() {
    let mut s = sample_session("sign-in-methods-not");
    s.oauth_available = vec!["microsoft".into()];
    for add in [
        json!({"email": "riley@contoso.example", "password": "pw", "ews": "mail.contoso.example", "signIn": "ntlm"}),
        json!({"email": "riley@jmap.example", "password": "pw", "jmap": "https://jmap.example/session", "smtpSignIn": "none"}),
        json!({"email": "riley@contoso.onmicrosoft.com", "oauth": "microsoft", "signIn": "password"}),
    ] {
        let e = s.run("account.add", &add).unwrap_err();
        assert!(e.to_string().contains("choose how"), "{add}: {e}");
    }
    // Automatic is what every account has: given, it's no choice.
    let ews = json!({"email": "riley@contoso.example", "password": "pw", "ews": "mail.contoso.example", "signIn": "automatic"});
    assert!(s.run("account.add", &ews).is_ok());
}
