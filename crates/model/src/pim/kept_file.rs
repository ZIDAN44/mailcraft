//! A file kept inline in a content line, as an event, a task and a note keep theirs: `ATTACH`
//! with its bytes in base64 (RFC 5545 3.8.1.1: `ENCODING=BASE64;VALUE=BINARY`), its type in
//! `FMTTYPE` and its name in `X-FILENAME`, as an Outlook data file's import writes one. Read
//! without decoding the bytes, so a list of them costs nothing to show.

/// The longest name read, in characters.
const MAX_NAME: usize = 255;

/// A file as its line says it: its name and type (empty when the line gives none), and its bytes
/// in base64, as kept.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct KeptFile<'a> {
    pub name: &'a str,
    pub mime: &'a str,
    pub base64: &'a str,
}

impl KeptFile<'_> {
    /// About how many bytes it holds (from its base64's length).
    pub fn size(&self) -> usize {
        let pad = self.base64.bytes().rev().take_while(|b| *b == b'=').count();
        (self.base64.len() / 4).saturating_mul(3).saturating_sub(pad)
    }

    /// Its name, or "attachment" when it has none.
    pub fn file_name(&self) -> &str {
        if self.name.trim().is_empty() { "attachment" } else { self.name }
    }
}

/// The file `line` holds, when it holds one inline (`ATTACH` with `VALUE=BINARY` and
/// `ENCODING=BASE64`); `None` for a link (`ATTACH:https://…`) or any other line.
pub fn kept_file(line: &str) -> Option<KeptFile<'_>> {
    // The value starts at the first ':' outside double quotes.
    let mut quoted = false;
    let colon = line.char_indices().find(|(_, c)| {
        if *c == '"' {
            quoted = !quoted;
        }
        *c == ':' && !quoted
    })?;
    let head = line.get(..colon.0)?;
    let base64 = line.get(colon.0.checked_add(1)?..)?;
    let (name, params) = head.split_once(';').unwrap_or((head, ""));
    if !name.trim().eq_ignore_ascii_case("ATTACH") {
        return None;
    }
    let get = |key: &str| param(params, key);
    let inline = get("VALUE").is_some_and(|v| v.eq_ignore_ascii_case("BINARY")) && get("ENCODING").is_some_and(|e| e.eq_ignore_ascii_case("BASE64"));
    if !inline {
        return None;
    }
    let name = get("X-FILENAME").unwrap_or_default();
    // At most MAX_NAME characters, cut at a character's start.
    let end = name.char_indices().nth(MAX_NAME).map_or(name.len(), |(i, _)| i);
    Some(KeptFile { name: name.get(..end).unwrap_or_default(), mime: get("FMTTYPE").unwrap_or_default(), base64: base64.trim() })
}

/// The value of the parameter `key` among `params` (`A=1;B="x;y"`), without its quotes.
fn param<'a>(params: &'a str, key: &str) -> Option<&'a str> {
    let mut quoted = false;
    let mut start = 0;
    let mut found = None;
    for (i, c) in params.char_indices().chain(Some((params.len(), ';'))) {
        match c {
            '"' => quoted = !quoted,
            ';' if !quoted => {
                let p = params.get(start..i).unwrap_or_default();
                if let Some((k, v)) = p.split_once('=')
                    && k.trim().eq_ignore_ascii_case(key)
                {
                    found = Some(v.trim().trim_matches('"'));
                }
                start = i.saturating_add(1);
            }
            _ => {}
        }
    }
    found
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The import's line reads back as its name, type and bytes; a link, another property or a
    /// damaged line isn't a file; a name with a semicolon or colon in its quotes is whole.
    #[test]
    fn a_kept_file_is_read_from_its_line() {
        let f = kept_file("ATTACH;FMTTYPE=application/pdf;ENCODING=BASE64;VALUE=BINARY;X-FILENAME=\"plan; v2: final.pdf\":JVBERg==").unwrap();
        assert_eq!((f.name, f.mime, f.base64, f.size()), ("plan; v2: final.pdf", "application/pdf", "JVBERg==", 4));
        assert_eq!(f.file_name(), "plan; v2: final.pdf");
        let bare = kept_file("attach;value=binary;encoding=base64:aGk=").unwrap();
        assert_eq!((bare.file_name(), bare.mime, bare.size()), ("attachment", "", 2));
        for line in [
            "ATTACH:https://example.com/a.pdf",
            "ATTACH;VALUE=BINARY:aGk=",
            "PHOTO:data:image/png;base64,aGk=",
            "ATTACH",
            "",
            ":",
            "ATTACH;X-FILENAME=\"open:aGk=",
        ] {
            assert_eq!(kept_file(line), None, "{line}");
        }
        let long = format!("ATTACH;ENCODING=BASE64;VALUE=BINARY;X-FILENAME=\"{}\":", "é".repeat(400));
        assert_eq!(kept_file(&long).unwrap().name.chars().count(), MAX_NAME);
    }
}
