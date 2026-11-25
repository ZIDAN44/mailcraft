//! A toast as MailCraft shows it: its title and text, and the `mailcraft:` link a click starts
//! when one reaches this MailCraft (`link.rs`), else its click event, waited for while it shows.
//! notify-rust's toasts can't carry a link (issue #273).

use std::sync::mpsc;

use windows::Data::Xml::Dom::XmlDocument;
use windows::Foundation::TypedEventHandler;
use windows::UI::Notifications::{ToastNotification, ToastNotificationManager};
use windows::core::HSTRING;

/// The id PowerShell's toasts go out under: Windows shows a toast only for an id it knows, and a
/// portable copy where MailCraft was never installed has none of its own.
pub const POWERSHELL: &str = r"{1AC14E77-02E7-4E5D-B744-2EB1AE5198B7}\WindowsPowerShell\v1.0\powershell.exe";

/// The longest a toast without a link is waited on: a banner stays seconds, a minute at most.
const WAIT: std::time::Duration = std::time::Duration::from_secs(10 * 60);

/// Show a toast under `app_id`, carrying `link` when there is one (Windows starts it when it's
/// clicked), sounding Windows' reminder sound for a `reminder`. Without a link, the toast's own
/// click event is waited for while it shows: whether it was clicked.
pub fn show(app_id: &str, title: &str, body: &str, link: Option<&str>, reminder: bool) -> Result<bool, String> {
    let doc = XmlDocument::new().map_err(|e| e.to_string())?;
    doc.LoadXml(&HSTRING::from(toast_xml(title, body, link, reminder))).map_err(|e| e.to_string())?;
    let toast = ToastNotification::CreateToastNotification(&doc).map_err(|e| e.to_string())?;
    let (tx, rx) = mpsc::channel::<bool>();
    if link.is_none() {
        let (clicked, gone, failed) = (tx.clone(), tx.clone(), tx);
        toast
            .Activated(&TypedEventHandler::new(move |_, _| {
                let _ = clicked.send(true);
                Ok(())
            }))
            .map_err(|e| e.to_string())?;
        toast
            .Dismissed(&TypedEventHandler::new(move |_, _| {
                let _ = gone.send(false);
                Ok(())
            }))
            .map_err(|e| e.to_string())?;
        toast
            .Failed(&TypedEventHandler::new(move |_, _| {
                let _ = failed.send(false);
                Ok(())
            }))
            .map_err(|e| e.to_string())?;
    }
    ToastNotificationManager::CreateToastNotifierWithId(&HSTRING::from(app_id)).and_then(|n| n.Show(&toast)).map_err(|e| e.to_string())?;
    if link.is_some() {
        return Ok(false);
    }
    // The toast is kept while it's waited on: its events come only while it lives.
    let clicked = rx.recv_timeout(WAIT).unwrap_or(false);
    drop(toast);
    Ok(clicked)
}

/// A toast's XML: its title and text, and the link a click starts (`activationType="protocol"`)
/// when there's one; silent as new mail's were, or a reminder's sound, each time.
pub fn toast_xml(title: &str, body: &str, link: Option<&str>, reminder: bool) -> String {
    let launch = link.map_or_else(String::new, |l| format!(" launch=\"{}\" activationType=\"protocol\"", xml_text(l)));
    let audio = if reminder { r#"<audio src="ms-winsoundevent:Notification.Reminder"/>"# } else { r#"<audio silent="true"/>"# };
    format!(
        "<toast{launch}><visual><binding template=\"ToastGeneric\"><text>{}</text><text>{}</text></binding></visual>{audio}</toast>",
        xml_text(title),
        xml_text(body)
    )
}

/// Text as XML holds it: its markup characters escaped, and the characters XML can't hold (a
/// subject's control characters) left out, as Windows refuses a toast with them.
fn xml_text(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&apos;"),
            '\t' | '\n' | '\r' => out.push(c),
            c if c < ' ' || matches!(c, '\u{fffe}' | '\u{ffff}') => {}
            c => out.push(c),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_toast_carries_its_link_and_holds_any_text() {
        let xml = toast_xml("New mail: <Ana> & \"Bo\"", "Re: 'plans'\u{7}\u{0}\nnext", Some("mailcraft:show?key=a%3A1%3A2%3AINBOX&x=1"), false);
        assert!(xml.starts_with("<toast launch=\"mailcraft:show?key=a%3A1%3A2%3AINBOX&amp;x=1\" activationType=\"protocol\">"), "{xml}");
        assert!(xml.contains("<text>New mail: &lt;Ana&gt; &amp; &quot;Bo&quot;</text><text>Re: &apos;plans&apos;\nnext</text>"), "{xml}");
        assert!(xml.contains(r#"<audio silent="true"/>"#), "new mail silent, as it was");
        assert!(!toast_xml("a", "b", None, false).contains("launch"));
    }

    /// Issue #239: a reminder sounds Windows' reminder sound, each one (the second of two came
    /// silent).
    #[test]
    fn a_reminder_sounds() {
        let xml = toast_xml("Standup", "09:00", None, true);
        assert!(xml.contains(r#"<audio src="ms-winsoundevent:Notification.Reminder"/>"#) && !xml.contains("silent"), "{xml}");
    }

    /// Windows reads the XML (a control character in a subject made it refuse the toast).
    #[test]
    fn windows_reads_every_toast() {
        for (title, body) in [("New mail", "Ana: plans"), ("a\u{1}b", "\u{0}<&>\u{fffe}"), ("", "")] {
            for reminder in [false, true] {
                let doc = XmlDocument::new().unwrap();
                doc.LoadXml(&HSTRING::from(toast_xml(title, body, Some("mailcraft:show"), reminder))).unwrap();
                assert!(ToastNotification::CreateToastNotification(&doc).is_ok(), "{title:?}");
            }
        }
    }
}
