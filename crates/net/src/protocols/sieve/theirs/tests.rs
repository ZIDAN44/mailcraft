use super::*;

/// SOGo's script with its automatic reply on, as Mailcow's SOGo writes it, its filters first.
#[test]
fn sogos_reply_is_read() {
    let sogo = "require [\"vacation\",\"fileinto\",\"envelope\"];\r\n\
        # Filter: Invoices\r\n\
        if header :contains \"subject\" \"invoice\" { fileinto \"Archive\"; stop; }\r\n\
        vacation :days 7 :addresses [\"carol@mc31.test\", \"c@mc31.test\"] :subject \"Away \\\"till\\\" Monday\" \"I'm out.\r\nBack on Monday.\";\r\n";
    assert_eq!(vacation_in(sogo), Some(("Away \"till\" Monday".into(), "I'm out.\r\nBack on Monday.".into())));
}

/// A reply limited to dates, its text a `text:` block with a dot doubled; comments of both
/// kinds; and "vacation" as a word elsewhere isn't one.
#[test]
fn a_text_block_in_a_test_is_read() {
    let script = "require [\"vacation\", \"date\", \"relational\"];\r\n\
        /* a note; vacation \"not this\" */\r\n\
        if header :contains \"subject\" \"vacation\" { keep; }\r\n\
        if allof(currentdate :value \"ge\" \"date\" \"2026-12-01\") {\r\n\
        \x20   vacation :mime :days 1 text: # the reply\r\n\
        Away until December.\r\n\
        ..signed\r\n\
        .\r\n\
        ;\r\n\
        }\r\n";
    assert_eq!(vacation_in(script), Some((String::new(), "Away until December.\n.signed".into())));
}

/// No reply, broken ones, and junk: nothing (or what there is), never a panic.
#[test]
fn no_reply_or_junk_is_nothing() {
    for (s, read) in [
        ("", None),
        ("require [\"fileinto\"]; fileinto \"Junk\";", None),
        ("vacation :subject", None),
        ("vacation :days 1 :addresses [\"a\"", None),
        ("/* never closed vacation \"x\";", None),
        ("é vacation ü \"x\";", None),
        ("vacation text:", Some("")),
        ("vacation \"unclosed", Some("unclosed")),
    ] {
        assert_eq!(vacation_in(s).map(|(_, t)| t), read.map(String::from), "{s}");
    }
    // A huge reply is cut, not kept whole.
    let long = format!("vacation \"{}\";", "x".repeat(MAX_KEPT * 2));
    assert!(vacation_in(&long).is_some_and(|(_, t)| t.len() <= MAX_KEPT));
}
