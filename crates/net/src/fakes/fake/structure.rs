//! The fake's `BODYSTRUCTURE` and `BODY[1.2]` / `BODY[1.2.MIME]` (RFC 3501 §6.4.5, §7.4.2), from a
//! small MIME splitter: enough for the text-first tests (issue #24), not a full parser.

/// One MIME part: its header, its content, and its parts when it's a multipart.
pub(super) struct Part<'a> {
    header: &'a [u8],
    content: &'a [u8],
    parts: Vec<Part<'a>>,
}

fn split_head(raw: &[u8]) -> (&[u8], &[u8]) {
    match raw.windows(4).position(|w| w == b"\r\n\r\n") {
        Some(p) => (&raw[..p + 2], &raw[p + 4..]),
        None => (raw, &[]),
    }
}

/// A header field's value, unfolded (`content-type`).
fn field(header: &[u8], name: &str) -> Option<String> {
    let text = String::from_utf8_lossy(header).replace("\r\n ", " ").replace("\r\n\t", " ");
    text.lines().find_map(|l| {
        let (k, v) = l.split_once(':')?;
        k.trim().eq_ignore_ascii_case(name).then(|| v.trim().to_string())
    })
}

/// `value; a=b; c="d"` as the value and its parameters.
fn params(v: &str) -> (String, Vec<(String, String)>) {
    let mut it = v.split(';');
    let main = it.next().unwrap_or("").trim().to_ascii_lowercase();
    let ps = it.filter_map(|p| p.split_once('=')).map(|(k, v)| (k.trim().to_ascii_lowercase(), v.trim().trim_matches('"').to_string())).collect();
    (main, ps)
}

impl<'a> Part<'a> {
    pub(super) fn of(raw: &'a [u8]) -> Part<'a> {
        let (header, content) = split_head(raw);
        let mut p = Part { header, content, parts: Vec::new() };
        let (ty, ps) = params(&field(header, "content-type").unwrap_or_else(|| "text/plain".into()));
        if ty.starts_with("multipart/")
            && let Some((_, b)) = ps.iter().find(|(k, _)| k == "boundary")
        {
            let delim = format!("--{b}");
            let text = content;
            let mut starts = Vec::new();
            let mut i = 0;
            while let Some(off) = text[i..].windows(delim.len()).position(|w| w == delim.as_bytes()) {
                starts.push(i + off);
                i += off + delim.len();
            }
            for w in starts.windows(2) {
                let body = &text[w[0] + delim.len()..w[1]];
                let body = body.strip_prefix(b"\r\n").unwrap_or(body);
                let body = body.strip_suffix(b"\r\n").unwrap_or(body);
                p.parts.push(Part::of(body));
            }
        }
        p
    }

    fn quoted(s: &str) -> String {
        format!("\"{}\"", s.replace('\\', "\\\\").replace('"', "\\\""))
    }

    fn plist(ps: &[(String, String)]) -> String {
        if ps.is_empty() {
            return "NIL".into();
        }
        format!(
            "({})",
            ps.iter().map(|(k, v)| format!("{} {}", Self::quoted(&k.to_ascii_uppercase()), Self::quoted(v))).collect::<Vec<_>>().join(" ")
        )
    }

    /// The part's `BODYSTRUCTURE`, extension data with the disposition.
    pub(super) fn structure(&self) -> String {
        let (ty, ps) = params(&field(self.header, "content-type").unwrap_or_else(|| "text/plain; charset=us-ascii".into()));
        let (major, minor) = ty.split_once('/').unwrap_or(("text", "plain"));
        let disposition = match field(self.header, "content-disposition") {
            Some(d) => {
                let (d, dps) = params(&d);
                format!("({} {})", Self::quoted(&d.to_ascii_uppercase()), Self::plist(&dps))
            }
            None => "NIL".into(),
        };
        if major == "multipart" {
            let inner: String = self.parts.iter().map(Part::structure).collect();
            let ps: Vec<(String, String)> = ps.into_iter().filter(|(k, _)| k == "boundary").collect();
            return format!("({inner} {} {} {disposition} NIL NIL)", Self::quoted(&minor.to_ascii_uppercase()), Self::plist(&ps));
        }
        let id = field(self.header, "content-id").map_or("NIL".into(), |c| Self::quoted(&c));
        let enc = field(self.header, "content-transfer-encoding").unwrap_or_else(|| "7BIT".into()).to_ascii_uppercase();
        let size = self.content.len();
        let ps: Vec<(String, String)> = ps.into_iter().filter(|(k, _)| k != "boundary").collect();
        let head = format!(
            "{} {} {} {id} NIL {} {size}",
            Self::quoted(&major.to_ascii_uppercase()),
            Self::quoted(&minor.to_ascii_uppercase()),
            Self::plist(&ps),
            Self::quoted(&enc)
        );
        if major == "text" {
            let lines = self.content.split(|b| *b == b'\n').count();
            format!("({head} {lines} NIL {disposition} NIL NIL)")
        } else {
            format!("({head} NIL {disposition} NIL NIL)")
        }
    }

    fn at(&self, path: &[usize]) -> Option<&Part<'a>> {
        match path.split_first() {
            None => Some(self),
            Some((&n, rest)) => self.parts.get(n.checked_sub(1)?)?.at(rest),
        }
    }

    /// `BODY[1.2]` (`mime`: false) or `BODY[1.2.MIME]` (true): the part's content or header.
    pub(super) fn section(&self, path: &[usize], mime: bool) -> Option<Vec<u8>> {
        let p = self.at(path)?;
        Some(if mime { [p.header, b"\r\n"].concat() } else { p.content.to_vec() })
    }
}

/// The `BODY.PEEK[…]` sections a `FETCH` asks for, as part paths and whether each is the MIME
/// header: `BODY.PEEK[1.2.MIME]` is `([1, 2], true)`.
pub(super) fn asked_sections(command: &str) -> Vec<(String, Vec<usize>, bool)> {
    let mut out = Vec::new();
    let mut rest = command;
    while let Some(i) = rest.find("BODY.PEEK[") {
        rest = &rest[i + "BODY.PEEK[".len()..];
        let Some(end) = rest.find(']') else { break };
        let spec = &rest[..end];
        let mime = spec.ends_with(".MIME");
        let nums = spec.trim_end_matches(".MIME");
        let path: Option<Vec<usize>> = nums.split('.').map(|n| n.parse().ok()).collect();
        if let Some(path) = path.filter(|p| !p.is_empty()) {
            out.push((spec.to_string(), path, mime));
        }
    }
    out
}
