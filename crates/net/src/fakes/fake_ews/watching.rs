//! The fake server's pull subscriptions (`Subscribe`, `GetEvents`, issue #145): each says the
//! folders whose mail changed since it was last asked (come, gone, moved or marked), found by
//! comparing them with what it saw then.

use std::collections::BTreeMap;

use super::*;

/// What a subscription saw of a folder: its items' ids, read and flagged.
type Seen = Vec<(String, bool, bool)>;

/// A pull subscription.
#[derive(Clone, Debug, Default)]
pub struct FakeSub {
    pub id: String,
    /// The folders asked for (by id); empty for every one.
    pub folders: Vec<String>,
    seen: BTreeMap<String, Seen>,
    /// Folders changed, not said yet (more than an answer holds).
    pending: Vec<String>,
    mark: u64,
}

/// The cookie the fake's load balancer gives a subscription asked for with affinity (issue #145).
pub(super) const BACKEND: &str = "X-BackEndOverrideCookie=MBX1~2b5e";

impl Data {
    /// Whether a request lands on the other server of a load-balanced pair ([`Data::balanced`]):
    /// a look at a subscription without its server's cookie does, every other time, and that
    /// server hasn't it.
    pub(super) fn lands_elsewhere(&mut self, op: &str, affine: bool, cookie: &str) -> bool {
        if !self.balanced || op != "GetEvents" || (affine && cookie.split(';').any(|c| c.trim() == BACKEND)) {
            return false;
        }
        self.balance_turn = !self.balance_turn;
        self.balance_turn
    }

    /// What every mail folder holds now.
    fn mail_now(&self) -> BTreeMap<String, Seen> {
        let mut now: BTreeMap<String, Seen> = BTreeMap::new();
        for it in &self.items {
            now.entry(it.folder.clone()).or_default().push((it.id.clone(), it.read, it.flagged));
        }
        for v in now.values_mut() {
            v.sort();
        }
        now
    }

    pub(super) fn subscribe(&mut self, op: &El) -> String {
        let Some(req) = op.child("PullSubscriptionRequest") else { return err("ErrorInvalidSubscriptionRequest") };
        let all = req.attr("SubscribeToAllFolders") == Some("true");
        if all && self.subscribe_all_refused {
            return err("ErrorInvalidSubscriptionRequest");
        }
        let folders: Vec<String> =
            req.child("FolderIds").map(|f| f.kids.iter().filter_map(|k| k.attr("Id")).map(str::to_string).collect()).unwrap_or_default();
        if !all && folders.is_empty() {
            return err("ErrorInvalidSubscriptionRequest");
        }
        let id = self.id("SUB");
        let seen = self.mail_now();
        self.subscriptions.push(FakeSub { id: id.clone(), folders, seen, ..Default::default() });
        ok(format!("<m:SubscriptionId>{id}</m:SubscriptionId><m:Watermark>W0</m:Watermark>"))
    }

    pub(super) fn get_events(&mut self, op: &El) -> String {
        self.get_events_asked += 1;
        let id = op.text_of("SubscriptionId").unwrap_or("").to_string();
        let now = self.mail_now();
        let per = self.events_per_answer.unwrap_or(usize::MAX).max(1);
        let Some(sub) = self.subscriptions.iter_mut().find(|s| s.id == id) else { return err("ErrorSubscriptionNotFound") };
        let watched = |f: &String| sub_watches(&sub.folders, f);
        let mut changed: Vec<String> = now.keys().chain(sub.seen.keys()).filter(|f| now.get(*f) != sub.seen.get(*f)).cloned().collect();
        changed.sort();
        changed.dedup();
        for f in changed.into_iter().filter(watched) {
            if !sub.pending.contains(&f) {
                sub.pending.push(f);
            }
        }
        sub.seen = now;
        let said: Vec<String> = sub.pending.drain(..sub.pending.len().min(per)).collect();
        let more = !sub.pending.is_empty();
        let previous = sub.mark;
        let mut events = String::new();
        for f in &said {
            sub.mark += 1;
            events.push_str(&format!(
                "<t:CreatedEvent><t:Watermark>W{}</t:Watermark><t:TimeStamp>2026-11-18T06:00:00Z</t:TimeStamp>\
                 <t:ItemId Id=\"I\"/><t:ParentFolderId Id=\"{}\"/></t:CreatedEvent>",
                sub.mark,
                esc(f)
            ));
        }
        if said.is_empty() {
            events = format!("<t:StatusEvent><t:Watermark>W{previous}</t:Watermark></t:StatusEvent>");
        }
        ok(format!(
            "<m:Notification><t:SubscriptionId>{id}</t:SubscriptionId><t:PreviousWatermark>W{previous}</t:PreviousWatermark>\
             <t:MoreEvents>{more}</t:MoreEvents>{events}</m:Notification>"
        ))
    }

    /// Every subscription forgotten, as a server restarted does.
    pub fn drop_subscriptions(&mut self) {
        self.subscriptions.clear();
    }
}

/// Whether a subscription to `folders` (none: every one) watches folder `f`.
fn sub_watches(folders: &[String], f: &str) -> bool {
    folders.is_empty() || folders.iter().any(|x| x == f)
}
