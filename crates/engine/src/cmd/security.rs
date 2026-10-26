//! Commands: S/MIME (File › Info › Security, and the message being written).

use super::*;

pub(super) fn commands() -> Vec<CommandSpec> {
    vec![
        CommandSpec::new("security.import", "Import certificate", "File › Info › Security", |s, v| {
            let account = account_param(s, v)?;
            let password = p::str(v, "password").unwrap_or_default().to_string();
            if p::bool(v, "kept") == Some(true) {
                return s.import_identity_data(&account, Vec::new(), &password);
            }
            if let Some(data) = v.get("data").filter(|d| d.is_array()) {
                let data: Vec<u8> = serde_json::from_value(data.clone()).map_err(|_| CmdError::Params("`data`: the file's bytes".into()))?;
                return s.import_identity_data(&account, data, &password);
            }
            let path = p::req_str(v, "path")?.trim().to_string();
            s.import_identity(&account, &path, &password)
        })
        .params(
            r#"{"path": string (a .p12 or .pfx file), "data"?: [bytes] (the file itself, in place of path), "kept"?: bool (the one the browser keeps, unlocked), "password"?: string, "account"?: string}"#,
        ),
        CommandSpec::new("security.findKeys", "Use a smart card", "File › Info › Security", |s, v| {
            let module = if p::bool(v, "keychain") == Some(true) { None } else { Some(p::req_str(v, "module")?.to_string()) };
            s.find_held_keys(module, p::str(v, "pin").map(str::to_string))
        })
        .params(r#"{"module"?: string (the card's PKCS #11 module: a .dll, .so or .dylib), "keychain"?: bool (the system's keychain instead: macOS), "pin"?: string}"#),
        CommandSpec::new("security.useKey", "Use this certificate", "File › Info › Security", |s, v| {
            let account = account_param(s, v)?;
            let index = p::u64(v, "index").ok_or_else(|| CmdError::Params("`index`: which certificate security.findKeys found".into()))?;
            s.use_held_key(&account, usize::try_from(index).unwrap_or(usize::MAX))
        })
        .params(r#"{"index": number (of what security.findKeys found), "account"?: string}"#),
        CommandSpec::new("security.forget", "Remove certificate", "File › Info › Security", |s, v| {
            let account = account_param(s, v)?;
            match p::str(v, "older") {
                Some(fingerprint) => s.forget_older_identity(&account, fingerprint),
                None => s.forget_identity(&account),
            }
        })
        .params(r#"{"older"?: string (an older certificate's SHA-256, AB:CD:…, from security.certificates; else the one in use), "account"?: string}"#),
        CommandSpec::new("security.useOlder", "Use for sending", "File › Info › Security", |s, v| {
            let account = account_param(s, v)?;
            s.use_older_identity(&account, p::req_str(v, "older")?)
        })
        .params(r#"{"older": string (an older certificate's SHA-256, from security.certificates), "account"?: string}"#),
        CommandSpec::new("security.certificates", "Certificates", "File › Info › Security", |s, v| {
            let account = account_param(s, v)?;
            let show = |i: mailcraft_smime::CertInfo| {
                json!({ "name": i.name, "emails": i.emails, "issuer": i.issuer, "notBefore": i.not_before, "notAfter": i.not_after, "fingerprint": i.fingerprint })
            };
            Ok(json!({ "inUse": s.identity(&account).map(show), "older": s.older_identities(&account).into_iter().map(show).collect::<Vec<_>>() }))
        })
        .params(r#"{"account"?: string}"#),
        CommandSpec::new("security.defaults", "Sign or encrypt new messages", "File › Info › Security", |s, v| {
            let account = account_param(s, v)?;
            s.set_smime_defaults(&account, p::bool(v, "sign"), p::bool(v, "encrypt"), p::bool(v, "keepSubject"))
        })
        .params(r#"{"sign"?: bool, "encrypt"?: bool, "keepSubject"?: bool (encrypted mail shows its subject on the way, not "[...]"), "account"?: string}"#),
        CommandSpec::new("security.keepUnlocked", "Keep unlocked in this browser", "File › Info › Security", |s, v| {
            let account = account_param(s, v)?;
            s.keep_unlocked(&account, p::bool(v, "on").unwrap_or(true))
        })
        .when(|s| (!s.jmap_only).then_some("the desktop app keeps certificates in the system's keychain"))
        .params(r#"{"on"?: bool (default true; false forgets the kept password), "account"?: string}"#),
        CommandSpec::new("compose.security", "Sign and Encrypt", "Message › Options", |s, v| {
            let c = s.compose.as_ref().ok_or_else(|| CmdError::Disabled("no message is being written".into()))?;
            let wants = p::bool(v, "sign") == Some(true) || p::bool(v, "encrypt") == Some(true);
            if wants && s.identity(&c.account).is_none() {
                return Err(CmdError::Disabled("import a certificate for the account first (File › Info › Security)".into()));
            }
            let c = s.compose.as_mut().ok_or_else(|| CmdError::Disabled("no message is being written".into()))?;
            if let Some(on) = p::bool(v, "sign") {
                c.sign = on;
            }
            if let Some(on) = p::bool(v, "encrypt") {
                c.encrypt = on;
            }
            Ok(json!({ "sign": c.sign, "encrypt": c.encrypt }))
        })
        .params(r#"{"sign"?: bool, "encrypt"?: bool}"#),
    ]
}
