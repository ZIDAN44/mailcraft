//! Issue #39: reactions (RFC 9078), sent and read.

use super::*;

fn original() -> Parsed {
    let raw = b"From: Sam Lee <sam@studio.test>\r\nTo: Ana <ana@studio.test>, Ben <ben@studio.test>\r\nCc: Kim <kim@studio.test>\r\n\
Subject: Launch plan\r\nMessage-ID: <plan-1@studio.test>\r\nReferences: <kickoff@studio.test>\r\n\r\nHere's the plan.\r\n";
    parse(raw).unwrap()
}

/// A reaction goes to the sender and everyone else the message went to (not to the one
/// reacting), threaded under it, its one part the emoji marked a reaction; read back, it says
/// so, in its summary too.
#[test]
fn a_reaction_is_a_reply_marked_so() {
    let me = EmailAddress::new("Ana", "ana@studio.test");
    let raw = reaction(&original(), "👍", &me, &["ana@studio.test".into()], 1_793_700_000, "<r1@studio.test>").unwrap();
    let text = String::from_utf8_lossy(&raw);
    assert!(text.contains("Content-Disposition: reaction"), "{text}");
    let p = parse(&raw).unwrap();
    assert_eq!(p.reaction, "👍");
    assert_eq!(p.subject, "Re: Launch plan");
    assert_eq!(p.in_reply_to, "plan-1@studio.test");
    assert_eq!(p.references, ["kickoff@studio.test", "plan-1@studio.test"]);
    assert_eq!(p.to.iter().map(|a| a.email.as_str()).collect::<Vec<_>>(), ["sam@studio.test"]);
    assert_eq!(p.cc.iter().map(|a| a.email.as_str()).collect::<Vec<_>>(), ["ben@studio.test", "kim@studio.test"]);
    let key = MessageKey { account: "a".into(), folder: "INBOX".into(), validity: 1, uid: 1 };
    let s = summary_from_full(&raw, key, Flags::default(), 100, 0);
    assert_eq!((s.reaction.as_str(), s.in_reply_to.as_str()), ("👍", "plan-1@studio.test"));
    // On one's own message: to the others.
    let mine = parse(b"From: Ana <ana@studio.test>\r\nTo: Sam <sam@studio.test>\r\nSubject: Re: x\r\nMessage-ID: <m@x>\r\n\r\nhi\r\n").unwrap();
    let raw = reaction(&mine, "🎉", &me, &["ana@studio.test".into()], 0, "").unwrap();
    assert_eq!(parse(&raw).unwrap().to.iter().map(|a| a.email.as_str()).collect::<Vec<_>>(), ["sam@studio.test"]);
    assert_eq!(parse(&raw).unwrap().subject, "Re: x");
}

/// Only emoji are a reaction: words in a part marked as one, or markup, aren't shown as one;
/// sequences (a skin tone, a joined family, a flag, a keycap) are; nothing is sent but emoji.
#[test]
fn only_emoji_are_a_reaction() {
    for good in ["👍", "❤️", "👍🏽", "👨‍👩‍👧", "🇫🇷", "1️⃣", "😆 🎉", "©️"] {
        assert!(is_reaction_text(good), "{good}");
    }
    for bad in ["", "  ", "ok", "👍 great", "<b>👍</b>", "1", &"👍".repeat(40)] {
        assert!(!is_reaction_text(bad), "{bad}");
    }
    let me = EmailAddress::new("Ana", "ana@studio.test");
    assert!(reaction(&original(), "nice", &me, &[], 0, "").is_err());
    let words = b"From: x@y.test\r\nIn-Reply-To: <plan-1@studio.test>\r\nContent-Type: text/plain\r\nContent-Disposition: reaction\r\n\r\nGreat job, click here\r\n";
    assert_eq!(parse(words).unwrap().reaction, "");
    // A part of a multipart reply, as RFC 9078 allows, with text for programs that don't know it.
    let multi = b"From: x@y.test\r\nIn-Reply-To: <plan-1@studio.test>\r\nContent-Type: multipart/mixed; boundary=B\r\n\r\n--B\r\n\
Content-Type: text/plain\r\n\r\nReacted with a heart.\r\n--B\r\nContent-Type: text/plain; charset=utf-8\r\nContent-Disposition: reaction\r\n\r\n\xe2\x9d\xa4\xef\xb8\x8f\r\n--B--\r\n";
    assert_eq!(parse(multi).unwrap().reaction, "❤️");
    assert!(REACTIONS.iter().all(|r| is_reaction_text(r)));
}
