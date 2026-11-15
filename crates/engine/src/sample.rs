//! The sample mailbox (`mailcraft --sample`): a local-only account full of our own example mail,
//! so the app can be explored, screenshotted and tested without a server. Every word here is
//! original; addresses use the reserved `example` domains (RFC 2606).

use mailcraft_mime::{self as mime, Draft, OutAttachment};
use mailcraft_model::{AccountConfig, EmailAddress, Flags, Folder, FolderRole, MessageKey, sort_newest_first};
use mailcraft_store::{FolderState, Store, StoreError};

mod attachments;
#[cfg(any(test, feature = "test-fixtures"))]
pub mod big;
mod pim;
pub use attachments::{sample_banner_png, sample_docx, sample_pdf, sample_png, sample_xlsx};
pub use pim::sample_pim;

pub const SAMPLE_ACCOUNT: &str = "sample";
/// The sample's second calendar (the studio's), beside the personal one.
pub const STUDIO_CALENDAR: &str = "studio";

struct Mail {
    folder: &'static str,
    from: (&'static str, &'static str),
    to: &'static [(&'static str, &'static str)],
    cc: &'static [(&'static str, &'static str)],
    subject: &'static str,
    body: &'static str,
    /// Minutes before "now".
    ago: i64,
    seen: bool,
    flagged: bool,
    attachments: &'static [(&'static str, Attach)],
    /// A formatted version of the body.
    html: Option<&'static str>,
    reply_to_index: Option<usize>,
}

#[derive(Clone, Copy)]
enum Attach {
    Png,
    Text(&'static str),
    Pdf,
    /// An invitation (`METHOD:REQUEST`) to the sample event with this uid.
    Invite(&'static str),
    /// A Word document and an Excel workbook, written here (an order confirmation).
    Docx,
    Xlsx,
}

const ME: (&str, &str) = ("Riley Moreno", "riley@studio.example");

fn mails() -> Vec<Mail> {
    vec![
        Mail {
            folder: "INBOX",
            from: ("Zoe Park", "zoe@studio.example"),
            to: &[ME],
            cc: &[("Sam Okafor", "sam@studio.example")],
            subject: "Storyboard review on Thursday",
            body: "Hi Riley,\n\nThe animatic is in good shape. Could you bring the revised frames for scenes 4 to 7 on Thursday? I attached the colour key we agreed on so we stay consistent.\n\nThanks,\nZoe",
            ago: 25,
            seen: false,
            flagged: true,
            attachments: &[("colour-key.png", Attach::Png)],
            reply_to_index: None,
            html: None,
        },
        Mail {
            folder: "INBOX",
            from: ("Sam Okafor", "sam@studio.example"),
            to: &[("Zoe Park", "zoe@studio.example"), ME],
            cc: &[],
            subject: "Re: Storyboard review on Thursday",
            body: "Works for me. I'll book the small screening room from 2 pm and set up the projector.\n\nSam",
            ago: 12,
            seen: false,
            flagged: false,
            attachments: &[],
            reply_to_index: Some(0),
            html: None,
        },
        Mail {
            folder: "INBOX",
            from: ("Print Shop", "orders@printshop.example"),
            to: &[ME],
            cc: &[],
            subject: "Your proof for the festival poster is ready",
            body: "Hello,\n\nThe proof for order 4471 (festival poster, A2, matte) is ready for your approval. Reply to this message with any changes, or with 'approved' and we will start printing tomorrow morning.\n\nPrint Shop",
            ago: 60 * 5,
            seen: true,
            flagged: false,
            attachments: &[
                ("proof-notes.txt", Attach::Text("Bleed: 3 mm\nColour profile: coated\nPaper: 170 gsm matte\n")),
                ("festival-poster-proof.pdf", Attach::Pdf),
                ("order-4471.docx", Attach::Docx),
                ("order-4471.xlsx", Attach::Xlsx),
            ],
            reply_to_index: None,
            html: None,
        },
        Mail {
            folder: "INBOX",
            from: ("Ana Lima", "ana@soundlab.example"),
            to: &[ME],
            cc: &[],
            subject: "Sound mix, version 3",
            body: "Riley,\n\nVersion 3 of the mix is up on the shared drive. I lowered the music under the dialogue in the café scene and added the rain ambience you asked for.\n\nCan you come and hear it in the lab tomorrow? I sent an invitation.\n\nLet me know what you think!\nAna",
            ago: 60 * 26,
            seen: false,
            flagged: false,
            attachments: &[("invite.ics", Attach::Invite("mix@sample.mailcraft"))],
            reply_to_index: None,
            html: None,
        },
        Mail {
            folder: "INBOX",
            from: ("Community Gallery", "hello@gallery.example"),
            to: &[ME],
            cc: &[],
            subject: "Open call: winter group show",
            body: "Dear artists,\n\nOur winter group show is open for submissions until the end of the month. Paintings, prints, photography and short films are all welcome. Each artist may submit up to three works.\n\nSee you at the opening!",
            ago: 60 * 24 * 3,
            seen: true,
            flagged: false,
            attachments: &[],
            reply_to_index: None,
            // Laid out like a newsletter: a centred table, a coloured band, two columns.
            html: Some(
                "<div style=\"display:none\">Submissions close 30 November</div>\
                 <table width=\"600\" align=\"center\" cellpadding=\"0\" cellspacing=\"0\">\
                 <tr><td align=\"center\" bgcolor=\"#2e7d6b\" background=\"cid:banner@gallery.example\" style=\"color:#ffffff\">\
                 <h2>Winter group show: open call</h2></td></tr>\
                 <tr><td><table width=\"100%\"><tr>\
                 <td width=\"60%\" valign=\"top\">\
                 <p>Dear artists,</p>\
                 <p>Our <b>winter group show</b> is open for submissions <i>until the end of the month</i>. \
                 Paintings, prints, photography and short films are all welcome.</p>\
                 <ul><li>Up to <b>three works</b> per artist</li><li>Opening night: the first Friday of December</li></ul>\
                 </td>\
                 <td width=\"40%\" valign=\"top\" align=\"center\">\
                 <div style=\"position:relative;width:180px\"><img src=\"cid:poster@gallery.example\" alt=\"Show poster\">\
                 <span style=\"position:absolute;top:6px;right:6px;background-color:#c62828;color:#ffffff\"><b>NEW</b></span></div>\
                 <p><span style=\"color:#c62828\">Deadline: 30 November</span> \
                 <span style=\"background-color:#fff176\">(no late entries)</span></p>\
                 </td></tr></table></td></tr>\
                 <tr><td align=\"center\">\
                 <p>Read the <a href=\"https://gallery.example/open-call?utm_source=newsletter&amp;utm_campaign=winter\">full guidelines</a> on our website.</p>\
                 <p>See you at the opening!</p></td></tr></table>\
                 <img src=\"https://gallery.example/track/open.gif\" width=\"1\" height=\"1\" alt=\"\">",
            ),
        },
        Mail {
            folder: "INBOX",
            from: ("Jun Watanabe", "jun@studio.example"),
            to: &[ME],
            cc: &[],
            subject: "Character turnarounds",
            body: "Here are the first turnarounds for the fox. Front, side and back are done; the three-quarter view needs another pass.\n\nJun",
            ago: 60 * 24 * 6,
            seen: true,
            flagged: false,
            attachments: &[("fox-turnaround.png", Attach::Png)],
            reply_to_index: None,
            html: None,
        },
        Mail {
            folder: "INBOX",
            from: ("Studio Ops", "ops@studio.example"),
            to: &[ME],
            cc: &[],
            subject: "Render farm maintenance this weekend",
            body: "The render farm will be offline from Saturday 08:00 to Sunday 20:00 for upgrades. Please queue long renders before Friday evening.",
            ago: 60 * 24 * 12,
            seen: true,
            flagged: false,
            attachments: &[],
            reply_to_index: None,
            html: None,
        },
        Mail {
            folder: "Sent",
            from: ME,
            to: &[("Print Shop", "orders@printshop.example")],
            cc: &[],
            subject: "Festival poster order",
            body: "Hello,\n\nPlease find the print-ready file for the festival poster attached. We need 40 copies in A2.\n\nBest,\nRiley",
            ago: 60 * 24 * 2,
            seen: true,
            flagged: false,
            attachments: &[],
            reply_to_index: None,
            html: None,
        },
        Mail {
            folder: "Archive",
            from: ("Zoe Park", "zoe@studio.example"),
            to: &[ME],
            cc: &[],
            subject: "Kick-off notes",
            body: "Notes from the kick-off: running time 8 minutes, hand-painted backgrounds, festival deadline in March.",
            ago: 60 * 24 * 40,
            seen: true,
            flagged: false,
            attachments: &[],
            reply_to_index: None,
            html: None,
        },
        Mail {
            folder: "Projects/Short film",
            from: ("Sam Okafor", "sam@studio.example"),
            to: &[ME],
            cc: &[],
            subject: "Shot list v2",
            body: "Updated shot list attached in the body below:\n\n1. Wide: rooftop at dusk\n2. Close: fox ears twitch\n3. Insert: lantern flickers\n",
            ago: 60 * 24 * 9,
            seen: true,
            flagged: true,
            attachments: &[],
            reply_to_index: None,
            html: None,
        },
        // Right-to-left mail: Arabic, a line of Hebrew, names and numbers inside.
        Mail {
            folder: "Projects/Short film",
            from: ("ليلى حداد", "layla@subtitles.example"),
            to: &[ME],
            cc: &[],
            subject: "الترجمة العربية جاهزة – Short film",
            body: "مرحبًا Riley،\n\nأرفقنا الترجمة العربية للفيلم القصير (النسخة 2). راجعنا التوقيت مع ملف shot_list_v2 وعدد الأسطر 128.\n\nשלום! גם הכתוביות בעברית מוכנות.\n\nمع التحية،\nليلى حداد",
            ago: 60 * 24 * 12,
            seen: true,
            flagged: false,
            attachments: &[],
            reply_to_index: None,
            html: None,
        },
        Mail {
            folder: "Junk",
            from: ("Prize Desk", "win@prizes.example"),
            to: &[ME],
            cc: &[],
            subject: "You may have won a camera",
            body: "Click to claim your prize. (This is a sample junk message: MailCraft never loads links or remote images on its own.)",
            ago: 60 * 24 * 4,
            seen: false,
            flagged: false,
            attachments: &[],
            reply_to_index: None,
            html: None,
        },
    ]
}

fn list(v: &[(&str, &str)]) -> Vec<EmailAddress> {
    v.iter().map(|(n, e)| EmailAddress::new(*n, *e)).collect()
}

/// Fill `store` with the sample account if it holds no accounts yet, or only the sample account
/// with none of its folders (its mail lost, or kept where this build doesn't read it, as an
/// older build's data file: the sample then stayed on "Loading folders…" for good). Returns
/// whether it did.
pub fn seed_if_empty(store: &Store, now: i64) -> Result<bool, StoreError> {
    seed_if_empty_at(store, now, 0)
}

/// [`seed_if_empty`] with the local UTC offset (seconds), so the sample's events fall in local
/// working hours.
pub fn seed_if_empty_at(store: &Store, now: i64, offset: i64) -> Result<bool, StoreError> {
    let accounts = store.load_accounts()?;
    let hollow = matches!(accounts.as_slice(), [a] if a.id == SAMPLE_ACCOUNT) && store.load_folders(SAMPLE_ACCOUNT).unwrap_or_default().is_empty();
    if !accounts.is_empty() && !hollow {
        return Ok(false);
    }
    let account = AccountConfig {
        id: SAMPLE_ACCOUNT.into(),
        name: ME.0.into(),
        email: ME.1.into(),
        username: ME.1.into(),
        signature: "Riley Moreno · Studio Lantern".into(),
        ..Default::default()
    };
    let folders: Vec<Folder> = [
        ("INBOX", FolderRole::Inbox),
        ("Drafts", FolderRole::Drafts),
        ("Sent", FolderRole::Sent),
        ("Archive", FolderRole::Archive),
        ("Junk", FolderRole::Junk),
        ("Trash", FolderRole::Trash),
        ("Projects", FolderRole::Other),
        ("Projects/Short film", FolderRole::Other),
    ]
    .into_iter()
    .map(|(p, role)| Folder { account: SAMPLE_ACCOUNT.into(), path: p.into(), delimiter: Some('/'), role, selectable: true, ..Default::default() })
    .collect();

    let mut states: Vec<(String, FolderState)> =
        folders.iter().map(|f| (f.path.clone(), FolderState { validity: 1, uid_next: 1, ..Default::default() })).collect();
    let mut ids: Vec<String> = Vec::new();
    for (i, m) in mails().into_iter().enumerate() {
        let date = now - m.ago * 60;
        let id = format!("sample-{i}@studio.example");
        let (in_reply_to, references) = match m.reply_to_index.and_then(|j| ids.get(j)) {
            Some(parent) => (parent.clone(), vec![parent.clone()]),
            None => (String::new(), Vec::new()),
        };
        let attachments = m
            .attachments
            .iter()
            .map(|(name, a)| match a {
                Attach::Png => OutAttachment { name: (*name).into(), mime: "image/png".into(), data: sample_png(), ..Default::default() },
                Attach::Text(t) => {
                    OutAttachment { name: (*name).into(), mime: "text/plain".into(), data: t.as_bytes().to_vec(), ..Default::default() }
                }
                Attach::Pdf => OutAttachment { name: (*name).into(), mime: "application/pdf".into(), data: sample_pdf(), ..Default::default() },
                Attach::Docx => OutAttachment {
                    name: (*name).into(),
                    mime: "application/vnd.openxmlformats-officedocument.wordprocessingml.document".into(),
                    data: sample_docx(),
                    ..Default::default()
                },
                Attach::Xlsx => OutAttachment {
                    name: (*name).into(),
                    mime: "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet".into(),
                    data: sample_xlsx(),
                    ..Default::default()
                },
                Attach::Invite(uid) => {
                    let events: Vec<_> = sample_pim(now, offset).events.into_iter().map(|e| e.event).filter(|e| e.uid == *uid).collect();
                    OutAttachment {
                        name: (*name).into(),
                        mime: "text/calendar; method=REQUEST".into(),
                        data: mailcraft_model::ical::write(&events, Some("REQUEST"), date).into_bytes(),
                        ..Default::default()
                    }
                }
            })
            .collect();
        let d = Draft {
            reply_to: Vec::new(),
            to: list(m.to),
            cc: list(m.cc),
            bcc: Vec::new(),
            subject: m.subject.into(),
            body: m.body.into(),
            in_reply_to,
            references,
            attachments,
            html: m.html.map(str::to_string),
            inline: if m.html.is_some_and(|h| h.contains("cid:poster@gallery.example")) {
                vec![
                    mime::InlinePicture {
                        cid: "poster@gallery.example".into(),
                        name: "poster.png".into(),
                        mime: "image/png".into(),
                        data: sample_png(),
                    },
                    mime::InlinePicture {
                        cid: "banner@gallery.example".into(),
                        name: "banner.png".into(),
                        mime: "image/png".into(),
                        data: sample_banner_png(),
                    },
                ]
            } else {
                Vec::new()
            },
            calendar: None,
            importance: 0,
            read_receipt: false,
            seven_bit: false,
            keep_bcc: false,
            source: String::new(),
        };
        let mut raw = mime::build(&d, &EmailAddress::new(m.from.0, m.from.1), date, &id)
            .map_err(|e| StoreError::Io { path: "sample".into(), source: std::io::Error::other(e) })?;
        // The newsletter and the prize mail come from mailing lists (they land in Other).
        if let Some(list) = match m.from.1 {
            "hello@gallery.example" => Some("news.gallery.example"),
            "win@prizes.example" => Some("offers.prizes.example"),
            _ => None,
        } {
            let domain = list.split_once('.').map_or(list, |(_, d)| d);
            let head = format!("List-Id: <{list}>\r\nList-Unsubscribe: <mailto:leave@{domain}>\r\n");
            raw = [head.as_bytes(), &raw].concat();
        }
        ids.push(id);
        let Some((path, st)) = states.iter_mut().find(|(p, _)| p == m.folder) else { continue };
        let uid = st.uid_next;
        st.uid_next += 1;
        let key = MessageKey { account: SAMPLE_ACCOUNT.into(), folder: path.clone(), validity: 1, uid };
        let flags = Flags { seen: m.seen, flagged: m.flagged, ..Default::default() };
        let size = u32::try_from(raw.len()).unwrap_or(u32::MAX);
        store.write_body(&key, &raw)?;
        st.messages.push(mime::summary_from_full(&raw, key, flags, size, date).into());
    }
    let mut folders = folders;
    for (path, st) in &mut states {
        sort_newest_first(&mut st.messages);
        store.save_folder(SAMPLE_ACCOUNT, path, st)?;
        if let Some(f) = folders.iter_mut().find(|f| &f.path == path) {
            f.unread = st.unread();
            f.total = u32::try_from(st.messages.len()).unwrap_or(0);
        }
    }
    store.save_folders(SAMPLE_ACCOUNT, &folders)?;
    store.save_accounts(&[account])?;
    store.save_account_doc(SAMPLE_ACCOUNT, crate::session::pim::PIM_DOC, &sample_pim(now, offset))?;
    Ok(true)
}
