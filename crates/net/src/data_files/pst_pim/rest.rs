//! What an Outlook item has no place for, kept in its text so nothing is lost on the way through
//! a data file: a card's fourth address or a phone of a kind Outlook has no field for, a task's
//! iCalendar properties MailCraft doesn't read. The export writes them after the item's own text
//! as the block of content lines a card or to-do is (`BEGIN:VCARD` … `END:VCARD`, RFC 6350;
//! `BEGIN:VTODO` … `END:VTODO`, RFC 5545), which a person reads in Outlook and the import reads
//! back into the card or task. Its first line says it's MailCraft's ([`MARK`]): a text that ends
//! in a card or to-do of its own (one pasted into a note, one from Outlook) stays text.

/// The most lines of such a block read back.
const MAX_LINES: usize = 1_000;
/// The first line of a block an export writes.
const MARK: &str = "X-MAILCRAFT-REST:1";

/// `text` with `lines` (content lines, unfolded) after it as a block of `kind` (`VCARD`,
/// `VTODO`); `text` alone when there are none, unless it ends in such a block itself: then an
/// empty block after it keeps it text.
pub(crate) fn with_rest(text: &str, kind: &str, lines: &[String]) -> String {
    if lines.is_empty() && !text.trim_end().ends_with(&format!("END:{kind}")) {
        return text.to_string();
    }
    let mut out = text.trim_end().to_string();
    if !out.is_empty() {
        out.push_str("\n\n");
    }
    out.push_str(&format!("BEGIN:{kind}\n{MARK}\n"));
    for l in lines {
        out.push_str(&l.replace(['\r', '\n'], ""));
        out.push('\n');
    }
    out.push_str(&format!("END:{kind}"));
    out
}

/// `body` (its line ends `\n`) without a block of `kind` an export wrote at its end, trimmed, and
/// the block's lines; the body as it is, and none, when it doesn't end with one.
pub(crate) fn split_rest(body: &str, kind: &str) -> (String, Vec<String>) {
    let whole = || (body.trim().to_string(), Vec::new());
    let trimmed = body.trim_end();
    let (begin, end) = (format!("BEGIN:{kind}"), format!("END:{kind}"));
    let Some(head) = trimmed.strip_suffix(end.as_str()).filter(|h| h.is_empty() || h.ends_with('\n')) else { return whole() };
    // The last line that begins the block.
    let Some(at) = head.match_indices(begin.as_str()).map(|(i, _)| i).filter(|i| *i == 0 || head.get(..*i).is_some_and(|b| b.ends_with('\n'))).last()
    else {
        return whole();
    };
    let (text, block) = (head.get(..at).unwrap_or_default(), head.get(at + begin.len()..).unwrap_or_default());
    if !block.starts_with('\n') && !block.is_empty() {
        return whole();
    }
    let mut lines = block.lines().map(str::trim_end).filter(|l| !l.is_empty());
    if !lines.next().is_some_and(|l| l.eq_ignore_ascii_case(MARK)) {
        return whole();
    }
    (text.trim().to_string(), lines.take(MAX_LINES).map(str::to_string).collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_block_after_the_text_and_back() {
        let lines = vec!["ADR;TYPE=postal:;;1 Quay;Porto;;;PT".to_string(), "NICKNAME:Bee".to_string()];
        let body = with_rest("Met at the fair.\n", "VCARD", &lines);
        assert_eq!(body, "Met at the fair.\n\nBEGIN:VCARD\nX-MAILCRAFT-REST:1\nADR;TYPE=postal:;;1 Quay;Porto;;;PT\nNICKNAME:Bee\nEND:VCARD");
        assert_eq!(split_rest(&body, "VCARD"), ("Met at the fair.".to_string(), lines.clone()));
        assert_eq!(split_rest(&with_rest("", "VCARD", &lines), "VCARD"), (String::new(), lines.clone()));
        assert_eq!(with_rest("Plain", "VTODO", &[]), "Plain");
        // Another kind's block, one not at the end, or a word that only looks like one: text.
        assert_eq!(split_rest(&body, "VTODO").1, Vec::<String>::new());
        let inside = format!("{body}\nmore");
        assert_eq!(split_rest(&inside, "VCARD"), (inside.trim().to_string(), Vec::new()));
        assert_eq!(split_rest("see END:VCARD", "VCARD").1, Vec::<String>::new());
        assert_eq!(split_rest("xBEGIN:VCARD\nEND:VCARD", "VCARD").1, Vec::<String>::new());
        assert_eq!(split_rest("BEGIN:VCARDS\nEND:VCARD", "VCARD").1, Vec::<String>::new());
        // A card or to-do MailCraft didn't write stays text; a text that ends in one gets an
        // empty block after it, so it comes back as it was.
        let pasted = "Their card:\nBEGIN:VCARD\nTEL:+1 555\nEND:VCARD";
        assert_eq!(split_rest(pasted, "VCARD"), (pasted.to_string(), Vec::new()));
        let kept = with_rest(pasted, "VCARD", &[]);
        assert_ne!(kept, pasted);
        assert_eq!(split_rest(&kept, "VCARD"), (pasted.to_string(), Vec::new()));
        let todo = "Plan\nBEGIN:VTODO\nRRULE:FREQ=DAILY\nEND:VTODO";
        assert_eq!(split_rest(&with_rest(todo, "VTODO", &["X-A:1".into()]), "VTODO"), (todo.to_string(), vec!["X-A:1".to_string()]));
        // A line break written into a line can't make a second line.
        assert_eq!(split_rest(&with_rest("", "VTODO", &["X-A:1\nX-B:2".to_string()]), "VTODO").1, ["X-A:1X-B:2"]);
    }
}
