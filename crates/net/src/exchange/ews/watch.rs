//! EWS: which folders changed since the account last looked (issue #145), from a pull
//! subscription to the mailbox's folders (`Subscribe`, `GetEvents`). Mail an Inbox rule files
//! into a folder of its own came only with the next full sync, a quarter of an hour on: told
//! where it went, the look every two minutes reads that folder too.

use super::*;

/// The event types asked for: mail come or made, moved, copied, changed (read elsewhere, say) or
/// deleted. What each says is only where it happened: that folder is read again.
const EVENTS: [&str; 6] = ["NewMailEvent", "CreatedEvent", "MovedEvent", "CopiedEvent", "ModifiedEvent", "DeletedEvent"];
/// Asks for more events at most this many times in one look (the server says when there are).
const ROUNDS_MAX: usize = 20;
/// Folders kept of one look at most: more than this is a look at everything, which the next full
/// sync is.
pub const CHANGED_MAX: usize = 500;

/// A subscription to the mailbox's folders: its id, and where the events read so far end (some
/// servers give no watermark: then empty, and each look gets what came since the last).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Watch {
    pub id: String,
    pub watermark: String,
}

impl Client {
    /// A subscription to every folder of the mailbox, lasting `minutes` between looks (1 to
    /// 1440). A server that can't subscribe to all of them at once (Exchange 2010 before its
    /// first service pack) is asked for the folders given, by their ids.
    pub fn watch(&self, minutes: u32, folders: &[String]) -> Result<Watch> {
        let events: String = EVENTS.iter().map(|e| format!("<t:EventType>{e}</t:EventType>")).collect();
        let timeout = minutes.clamp(1, 1440);
        let all = format!(
            "<m:Subscribe><m:PullSubscriptionRequest SubscribeToAllFolders=\"true\"><t:EventTypes>{events}</t:EventTypes>\
             <t:Timeout>{timeout}</t:Timeout></m:PullSubscriptionRequest></m:Subscribe>"
        );
        match self.call_one(&all).and_then(|m| watch_in(&m)) {
            Ok(w) => Ok(w),
            Err(e) if folders.is_empty() => Err(e),
            Err(e) => {
                log::info!("subscribing to every folder at once: {e}; to each instead");
                let ids: String = folders.iter().take(CHANGED_MAX).map(|id| format!("<t:FolderId Id=\"{}\"/>", esc(id))).collect();
                let each = format!(
                    "<m:Subscribe><m:PullSubscriptionRequest><t:FolderIds>{ids}</t:FolderIds><t:EventTypes>{events}</t:EventTypes>\
                     <t:Timeout>{timeout}</t:Timeout></m:PullSubscriptionRequest></m:Subscribe>"
                );
                watch_in(&self.call_one(&each)?)
            }
        }
    }

    /// The ids of the folders something happened in since the last look, the watermark moved on.
    /// An error when the subscription is gone (it lapsed, the server restarted): make another.
    pub fn changed(&self, w: &mut Watch) -> Result<Vec<String>> {
        let mut out: Vec<String> = Vec::new();
        for _ in 0..ROUNDS_MAX {
            let doc = self.call(&format!(
                "<m:GetEvents><m:SubscriptionId>{}</m:SubscriptionId><m:Watermark>{}</m:Watermark></m:GetEvents>",
                esc(&w.id),
                esc(&w.watermark)
            ))?;
            let got = events_in(&doc)?;
            for id in got.folders {
                if !out.contains(&id) && out.len() < CHANGED_MAX {
                    out.push(id);
                }
            }
            if let Some(mark) = got.watermark {
                w.watermark = mark;
            }
            if !got.more {
                break;
            }
        }
        Ok(out)
    }
}

/// A subscription's id and first watermark, in `Subscribe`'s response message.
fn watch_in(m: &El) -> Result<Watch> {
    let id = m.text_of("SubscriptionId").map(str::trim).filter(|s| !s.is_empty() && s.len() <= 1024);
    let id = id.ok_or_else(|| NetError::Protocol("the server gave no subscription".into()))?;
    let watermark = m.text_of("Watermark").map(str::trim).filter(|s| s.len() <= 1024).unwrap_or("");
    Ok(Watch { id: id.to_string(), watermark: watermark.to_string() })
}

/// What one `GetEvents` answer says.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct Events {
    /// The folders events happened in, each once, in the order said.
    pub folders: Vec<String>,
    /// More are waiting: ask again.
    pub more: bool,
    /// Where the events read end, when the server says.
    pub watermark: Option<String>,
}

/// The events in a `GetEvents` answer: an error when it refused (the subscription gone).
pub fn events_in(doc: &El) -> Result<Events> {
    let Some(m) = messages(doc).into_iter().next() else { return Err(NetError::Protocol("an empty answer".into())) };
    check_message(m)?;
    let Some(n) = m.find("Notification") else { return Ok(Events::default()) };
    let mut out = Events { more: n.text_of("MoreEvents").is_some_and(|t| t.trim().eq_ignore_ascii_case("true")), ..Default::default() };
    let mut folders: Vec<String> = Vec::new();
    let mut add = |id: Option<&str>| {
        if let Some(id) = id.map(str::trim).filter(|id| !id.is_empty() && id.len() <= 1024)
            && !folders.iter().any(|f| f == id)
            && folders.len() < CHANGED_MAX
        {
            folders.push(id.to_string());
        }
    };
    for e in &n.kids {
        if !e.name.ends_with("Event") {
            continue;
        }
        // An item's event happened in its folder (and the one it left); a folder's, to the
        // folder itself (what it holds changed: its counts).
        let id_of = |name: &str| e.child(name).and_then(|f| f.attr("Id"));
        if e.child("FolderId").is_some() {
            add(id_of("FolderId"));
        } else {
            add(id_of("ParentFolderId"));
            add(id_of("OldParentFolderId"));
        }
        if let Some(mark) = e.text_of("Watermark").map(str::trim).filter(|s| !s.is_empty() && s.len() <= 1024) {
            out.watermark = Some(mark.to_string());
        }
    }
    out.folders = folders;
    Ok(out)
}
