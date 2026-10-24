//! Commands: S/MIME (File › Info › Security, and the message being written).

use super::*;

pub(super) fn commands() -> Vec<CommandSpec> {
    vec![
        CommandSpec::new("security.import", "Import certificate", "File › Info › Security", |s, v| {
            let account = account_param(s, v)?;
            let path = p::req_str(v, "path")?.trim().to_string();
            let password = p::str(v, "password").unwrap_or_default().to_string();
            s.import_identity(&account, &path, &password)
        })
        .params(r#"{"path": string (a .p12 or .pfx file), "password"?: string, "account"?: string}"#),
        CommandSpec::new("security.forget", "Remove certificate", "File › Info › Security", |s, v| {
            let account = account_param(s, v)?;
            s.forget_identity(&account)
        })
        .params(r#"{"account"?: string}"#),
        CommandSpec::new("security.defaults", "Sign or encrypt new messages", "File › Info › Security", |s, v| {
            let account = account_param(s, v)?;
            s.set_smime_defaults(&account, p::bool(v, "sign"), p::bool(v, "encrypt"))
        })
        .params(r#"{"sign"?: bool, "encrypt"?: bool, "account"?: string}"#),
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
