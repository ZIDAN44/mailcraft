//! The store: message bodies, changes waiting for the server, and the Outbox.

use crate::*;

impl Store {
    pub fn read_body(&self, key: &MessageKey) -> Option<Vec<u8>> {
        self.files.read(&self.body_path(key)).ok()
    }

    pub fn has_body(&self, key: &MessageKey) -> bool {
        self.files.is_file(&self.body_path(key))
    }

    pub fn write_body(&self, key: &MessageKey, raw: &[u8]) -> Result<()> {
        self.files.write(&self.body_path(key), raw)
    }

    /// Many message sources at once: one transaction for all of them in a data file (importing
    /// hundreds of thousands, a commit each spent most of the time waiting for the disk).
    pub fn write_bodies(&self, bodies: &[(MessageKey, Vec<u8>)]) -> Result<()> {
        let items: Vec<(PathBuf, Vec<u8>)> = bodies.iter().map(|(k, raw)| (self.body_path(k), raw.clone())).collect();
        #[cfg(not(target_arch = "wasm32"))]
        {
            self.files.write_many(&items)
        }
        #[cfg(target_arch = "wasm32")]
        {
            for (p, raw) in &items {
                self.files.write(p, raw)?;
            }
            Ok(())
        }
    }

    pub fn remove_body(&self, key: &MessageKey) {
        let _ = self.files.remove_file(&self.body_path(key));
    }

    pub fn load_ops(&self, account: &str) -> Result<Vec<Op>> {
        self.read_json(&self.account_dir(account).join("ops.json"))
    }

    /// Append `op` to ops.json.
    pub fn add_op(&self, account: &str, op: Op) -> Result<()> {
        let _held = self.changes.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        let mut ops = self.load_ops(account).unwrap_or_default();
        ops.push(op);
        self.save_ops(account, &ops)
    }

    /// Delete the first entry equal to `op` from ops.json, if any.
    pub fn remove_op(&self, account: &str, op: &Op) -> Result<()> {
        let _held = self.changes.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        let mut ops = self.load_ops(account).unwrap_or_default();
        if let Some(i) = ops.iter().position(|o| o == op) {
            ops.remove(i);
            self.save_ops(account, &ops)?;
        }
        Ok(())
    }

    pub fn save_ops(&self, account: &str, ops: &[Op]) -> Result<()> {
        let path = self.account_dir(account).join("ops.json");
        if ops.is_empty() {
            match self.files.remove_file(&path) {
                Err(e) if e.kind() != io::ErrorKind::NotFound => return Err(io_err(&path)(e)),
                _ => return Ok(()),
            }
        }
        self.write_json(&path, ops)
    }

    pub(super) fn outbox_dir(&self, account: &str) -> PathBuf {
        self.account_dir(account).join("outbox")
    }

    /// Queue a message to send when the account is back online.
    pub fn outbox_add(&self, account: &str, item: &OutboxItem, raw: &[u8]) -> Result<()> {
        let dir = self.outbox_dir(account);
        self.files.write(&dir.join(format!("{}.eml", item.id)), raw)?;
        self.write_json(&dir.join(format!("{}.json", item.id)), item)
    }

    /// Update a queued message's bookkeeping (attempts, last error).
    pub fn outbox_update(&self, account: &str, item: &OutboxItem) -> Result<()> {
        self.write_json(&self.outbox_dir(account).join(format!("{}.json", item.id)), item)
    }

    /// Queued messages, oldest first.
    pub fn outbox(&self, account: &str) -> Vec<OutboxItem> {
        let dir = self.outbox_dir(account);
        let mut items: Vec<OutboxItem> = self
            .files
            .list(&dir)
            .into_iter()
            .filter(|(name, _)| name.ends_with(".json"))
            .filter_map(|(_, p)| self.files.read(&p).ok())
            .filter_map(|b| serde_json::from_slice::<OutboxItem>(&b).ok())
            .filter(|i| self.files.is_file(&dir.join(format!("{}.eml", i.id))))
            .collect();
        items.sort_by_key(|i| (i.queued, i.id));
        items
    }

    pub fn outbox_raw(&self, account: &str, id: u64) -> Option<Vec<u8>> {
        self.files.read(&self.outbox_dir(account).join(format!("{id}.eml"))).ok()
    }

    pub fn outbox_remove(&self, account: &str, id: u64) {
        let dir = self.outbox_dir(account);
        let _ = self.files.remove_file(&dir.join(format!("{id}.json")));
        let _ = self.files.remove_file(&dir.join(format!("{id}.eml")));
    }

    /// Cached sources across all folders of an account, as (key, path) pairs, for indexing.
    pub fn cached_bodies(&self, account: &str, folder: &str, validity: u32) -> Vec<u32> {
        let dir = self.folder_dir(account, folder).join(validity.to_string());
        self.files.list(&dir).into_iter().filter_map(|(n, _)| n.strip_suffix(".eml").and_then(|n| n.parse().ok())).collect()
    }
}
