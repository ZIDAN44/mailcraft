//! Where passwords live: the system keychain in the app, memory in the CLI and the tests.

use std::collections::HashMap;
use std::sync::Mutex;

use mailcraft_model::AccountConfig;

/// Where passwords live. The desktop app backs this with the system keychain.
pub trait Credentials: Send + Sync {
    fn get(&self, account: &AccountConfig) -> Option<String>;
    fn set(&self, account: &AccountConfig, password: &str) -> std::result::Result<(), String>;
    fn delete(&self, account: &AccountConfig);
    /// The outgoing server's own password (accounts with an `smtp_username`), kept apart.
    fn get_smtp(&self, account: &AccountConfig) -> Option<String>;
    fn set_smtp(&self, account: &AccountConfig, password: &str) -> std::result::Result<(), String>;
    fn delete_smtp(&self, account: &AccountConfig);
    /// Another secret of the account's, named by `what` (`smime`: its S/MIME certificate's
    /// private key). A store without them keeps none.
    fn get_secret(&self, _account: &AccountConfig, _what: &str) -> Option<String> {
        None
    }
    fn set_secret(&self, _account: &AccountConfig, _what: &str, _secret: &str) -> std::result::Result<(), String> {
        Err("there's no keychain to keep it in".into())
    }
    fn delete_secret(&self, _account: &AccountConfig, _what: &str) {}
}

/// Passwords held in memory only (tests, the CLI, and when no keychain is available).
#[derive(Default)]
pub struct MemoryCredentials(Mutex<HashMap<String, String>>);

impl Credentials for MemoryCredentials {
    fn get(&self, account: &AccountConfig) -> Option<String> {
        self.0.lock().ok().and_then(|m| m.get(&account.id).cloned())
    }
    fn set(&self, account: &AccountConfig, password: &str) -> std::result::Result<(), String> {
        self.0.lock().map_err(|e| e.to_string())?.insert(account.id.clone(), password.to_string());
        Ok(())
    }
    fn delete(&self, account: &AccountConfig) {
        if let Ok(mut m) = self.0.lock() {
            m.remove(&account.id);
        }
    }
    fn get_smtp(&self, account: &AccountConfig) -> Option<String> {
        self.0.lock().ok().and_then(|m| m.get(&format!("{}|smtp", account.id)).cloned())
    }
    fn set_smtp(&self, account: &AccountConfig, password: &str) -> std::result::Result<(), String> {
        self.0.lock().map_err(|e| e.to_string())?.insert(format!("{}|smtp", account.id), password.to_string());
        Ok(())
    }
    fn delete_smtp(&self, account: &AccountConfig) {
        if let Ok(mut m) = self.0.lock() {
            m.remove(&format!("{}|smtp", account.id));
        }
    }
    fn get_secret(&self, account: &AccountConfig, what: &str) -> Option<String> {
        self.0.lock().ok().and_then(|m| m.get(&format!("{}|{what}", account.id)).cloned())
    }
    fn set_secret(&self, account: &AccountConfig, what: &str, secret: &str) -> std::result::Result<(), String> {
        self.0.lock().map_err(|e| e.to_string())?.insert(format!("{}|{what}", account.id), secret.to_string());
        Ok(())
    }
    fn delete_secret(&self, account: &AccountConfig, what: &str) {
        if let Ok(mut m) = self.0.lock() {
            m.remove(&format!("{}|{what}", account.id));
        }
    }
}

impl super::Worker {
    /// Before the account's thread first connects, on that thread: the password just given kept
    /// in the keychain, or the one kept there read. The keychain may ask the user first (macOS
    /// does after an update, for a build signed ad hoc), so it isn't done on the window's
    /// thread, which would stop drawing till it's answered. `false` when the keychain has no
    /// password: the account asks for one, and the pool forgets this thread.
    pub(super) fn settle_password(&mut self) -> bool {
        let Some(given) = self.keychain.take() else { return true };
        let Some(creds) = self.creds.clone() else { return true };
        let password = match given {
            Some(p) => {
                if let Err(e) = creds.set(&self.acc, &p) {
                    self.sink.emit(mailcraft_model::job::Event::Notice {
                        account: self.acc.id.clone(),
                        message: format!("the password could not be saved in the keychain: {e}"),
                    });
                }
                p
            }
            None => match creds.get(&self.acc) {
                Some(p) => p,
                None => {
                    self.no_password.store(true, std::sync::atomic::Ordering::Relaxed);
                    self.status(mailcraft_model::job::SyncStatus::NeedsPassword { error: String::new() });
                    return false;
                }
            },
        };
        if let Ok(mut a) = self.auth.lock() {
            a.secret = password;
        }
        if let Some(start) = self.quick_start.take() {
            start();
        }
        true
    }
}
