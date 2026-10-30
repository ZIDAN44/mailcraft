//! Engine tests: a message shown as its text first, its files still coming (issue #24).

use mailcraft_mime as mime;

use super::*;

/// Shown as it comes, its files listed; what needs their bytes waits for them, and says so
/// (forwarding it went without them); the whole message, once it comes, takes its place.
#[test]
fn a_message_text_first_then_whole() {
    let mut s = sample_session("text-first");
    let k = key_of(&s, "Storyboard review on Thursday");
    s.run("list.open", &k).unwrap();
    let key = s.reading.as_ref().unwrap().key.clone();
    let whole = s.store.as_ref().unwrap().read_body(&key).unwrap();
    let parsed = mime::parse(&whole).unwrap();
    assert!(!parsed.attachments.is_empty(), "the sample message has a file");
    let d = mime::Draft { subject: parsed.subject.clone(), body: "Hi Riley, the animatic is in good shape.".into(), ..Default::default() };
    let files: Vec<mime::PendingFile> = parsed
        .attachments
        .iter()
        .map(|a| mime::PendingFile { name: a.name.clone(), mime: a.mime.clone(), size: a.size as u64, ..Default::default() })
        .collect();
    let text = mime::text_first(&d, &parsed.from, NOW, &parsed.message_id, &files).unwrap();
    // As it would be opened from the server: nothing kept here yet.
    s.store.as_ref().unwrap().remove_body(&key);
    s.apply(Event::Body { key: key.clone(), raw: text });
    let r = s.reading.as_ref().unwrap();
    assert!(r.text_first);
    let shown = r.parsed.as_ref().unwrap();
    assert_eq!(shown.attachments.iter().map(|a| a.size).collect::<Vec<_>>(), parsed.attachments.iter().map(|a| a.size).collect::<Vec<_>>());
    let index = shown.attachments[0].index;
    for (cmd, v) in
        [("mail.forward", json!({})), ("attachment.save", json!({"index": index, "path": "x"})), ("attachment.saveAll", json!({"dir": "."}))]
    {
        let e = s.run(cmd, &v).err().map(|e| e.to_string()).unwrap_or_default();
        assert!(e.contains("still downloading"), "{cmd}: {e}");
    }
    assert!(s.attachment_bytes(index).is_err());
    // The whole of it.
    s.store.as_ref().unwrap().write_body(&key, &whole).unwrap();
    s.apply(Event::Body { key: key.clone(), raw: whole });
    let r = s.reading.as_ref().unwrap();
    assert!(!r.text_first);
    assert!(s.attachment_bytes(index).is_ok_and(|b| !b.is_empty()));
    s.run("mail.forward", &json!({})).unwrap();
    assert_eq!(s.compose.as_ref().unwrap().attachments.len(), parsed.attachments.iter().filter(|a| !a.inline).count().max(1));
    done(s);
}
