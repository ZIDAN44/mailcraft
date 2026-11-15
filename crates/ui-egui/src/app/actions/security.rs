//! An account's keys: its S/MIME certificate (from a file, a smart card or the system's), its
//! OpenPGP key and its correspondents'.

use crate::*;

impl MailApp {
    /// A `.p12` / `.pfx` file picked, then its password asked. The browser's picker answers
    /// later (with the file's bytes).
    pub(super) fn import_certificate(&mut self, account: String) {
        if self.services.pick_later(&crate::Picked::Certificate(account.clone())) {
            return;
        }
        if let Some(path) = self.services.pick_certificate() {
            self.ui.dialog = Some(Dialog::Certificate { account, path: path.to_string_lossy().into_owned(), data: None, password: String::new() });
        }
    }

    /// A certificate file picked for an account's mail servers to trust (issue #105): an
    /// authority the organisation runs, or a server's own.
    pub(super) fn trust_certificate_file(&mut self, account: String) {
        if let Some(path) = self.services.pick_server_certificate() {
            match self.session.run("account.trustCertificateFile", &json!({"account": account, "path": path.to_string_lossy()})) {
                Ok(_) => self.session.notice(tl!("The certificate is trusted for this account").to_string(), false),
                Err(e) => self.session.notice(e.to_string(), true),
            }
        }
    }

    pub(super) fn import_pgp_key(&mut self, account: String) {
        if let Some(path) = self.services.pick_pgp_key() {
            self.ui.dialog = Some(Dialog::PgpKey { account, path: path.to_string_lossy().into_owned(), passphrase: String::new() });
        }
    }

    pub(super) fn import_public_key(&mut self, account: &str) {
        if let Some(path) = self.services.pick_pgp_key() {
            // The session says what it kept.
            if let Err(e) = self.session.run("pgp.importPublic", &json!({"account": account, "path": path.to_string_lossy()})) {
                self.session.notice(e.to_string(), true);
            }
        }
    }

    pub(super) fn use_smart_card(&mut self, account: String) {
        let module = self.session.card_search().modules.first().cloned().unwrap_or_default();
        self.ui.dialog = Some(Dialog::SmartCard { account, module, pin: String::new(), system: false });
    }

    pub(super) fn use_system_certificate(&mut self, account: String) {
        self.run("security.findKeys", json!({"keychain": true}));
        self.ui.dialog = Some(Dialog::SmartCard { account, module: String::new(), pin: String::new(), system: true });
    }
}
