use super::*;
use crate::fakes::measure::peak_working_set;

#[test]
fn answers_are_read_carefully() {
    let doc = parse(
        "<s:Envelope xmlns:s=\"x\"><s:Body><m:GetItemResponse><m:ResponseMessages>\
             <m:GetItemResponseMessage ResponseClass=\"Success\"><m:ResponseCode>NoError</m:ResponseCode><m:Items>\
             <t:Message><t:ItemId Id=\"A&amp;1\" ChangeKey=\"c\"/><t:Subject>Caf&#233; &lt;plan&gt;</t:Subject><t:IsRead>true</t:IsRead>\
             <t:ExtendedProperty><t:ExtendedFieldURI PropertyTag=\"0x1090\" PropertyType=\"Integer\"/><t:Value>2</t:Value></t:ExtendedProperty>\
             <t:From><t:Mailbox><t:Name>Zoe \"Z\" Park</t:Name><t:EmailAddress>zoe@contoso.example</t:EmailAddress><t:RoutingType>SMTP</t:RoutingType></t:Mailbox></t:From>\
             <t:ToRecipients><t:Mailbox><t:Name>Me</t:Name><t:EmailAddress>/o=Org/cn=me</t:EmailAddress><t:RoutingType>EX</t:RoutingType></t:Mailbox></t:ToRecipients>\
             <t:DateTimeSent>2026-10-15T09:00:00Z</t:DateTimeSent></t:Message></m:Items></m:GetItemResponseMessage>\
             <m:GetItemResponseMessage ResponseClass=\"Error\"><m:MessageText>The specified object was not found.</m:MessageText><m:ResponseCode>ErrorItemNotFound</m:ResponseCode></m:GetItemResponseMessage>\
             </m:ResponseMessages></m:GetItemResponse></s:Body></s:Envelope>",
    );
    let ms = messages(&doc);
    assert_eq!(ms.len(), 2);
    assert_eq!(format!("{:?}", into_messages(doc.clone())), format!("{ms:?}"), "the same messages, moved out");
    assert!(into_messages(parse("<a><Body><b/></Body></a>")).is_empty() && into_messages(El::default()).is_empty());
    assert!(check_message(ms[0]).is_ok());
    assert!(matches!(check_message(ms[1]), Err(NetError::No(m)) if m.contains("not found")));
    let it = ms[0].find("Message").unwrap();
    assert_eq!(it.child("ItemId").unwrap().attr("Id"), Some("A&1"));
    assert_eq!(it.text_of("Subject"), Some("Café <plan>"));
    let f = flags(it);
    assert!(f.seen && f.flagged && !f.draft);
    let h = String::from_utf8(header_block(it)).unwrap();
    assert!(h.contains("From: \"Zoe Z Park\" <zoe@contoso.example>\r\n"), "{h}");
    assert!(h.contains("To: \"Me\" <>\r\n"), "an Exchange-internal address keeps the name: {h}");
    assert!(h.contains("Subject: =?UTF-8?B?"), "{h}");
    // Hostile: deep, broken, junk.
    let deep = "<a>".repeat(10_000);
    let _ = parse(&deep);
    let _ = parse("<a><b></a></b>&bogus;&#xFFFFFFFF;");
    // Folders of every kind, in the answer's order (contacts and calendars aren't `Folder`).
    let f = parse(
        "<m:RootFolder><t:Folders><t:Folder><t:DisplayName>A</t:DisplayName></t:Folder>                       <t:ContactsFolder><t:DisplayName>B</t:DisplayName></t:ContactsFolder><t:CalendarFolder><t:DisplayName>C</t:DisplayName></t:CalendarFolder>                       <t:TasksFolder><t:DisplayName>D</t:DisplayName></t:TasksFolder><t:SearchFolder><t:DisplayName>E</t:DisplayName></t:SearchFolder></t:Folders></m:RootFolder>",
    );
    let names: Vec<&str> = f.folders().iter().filter_map(|x| x.text_of("DisplayName")).collect();
    assert_eq!(names, ["A", "B", "C", "D", "E"]);
    // Entities in text and attributes, CDATA and numeric references, read as text.
    let e = parse("<t:a xmlns:t=\"x\" t:Id=\"R&amp;D &quot;1&quot;\">Tom &amp; Jerry &lt;3 &#233;t&#xE9; <![CDATA[<raw> & co]]> ünï</t:a>");
    let a = e.find("a").unwrap();
    assert_eq!(a.attr("Id"), Some("R&D \"1\""));
    assert_eq!(a.text, "Tom & Jerry <3 été <raw> & co ünï");
    assert_eq!(unb64("aGk=\r\n"), Some(b"hi".to_vec()));
    assert_eq!(unb64("**"), None);
    assert_eq!(esc("a<b&\"c\u{1}"), "a&lt;b&amp;&quot;c");
}

/// Audit #112: a big message's source downloaded (60 MB, 80 MB of base64 in the answer), from a
/// server whose answer is made before the clock starts: how long it takes and how far the
/// process's peak memory grows, the source byte for byte. Run alone, in release:
/// `cargo test --release -p mailcraft-net --lib a_big_source_downloads -- --ignored --nocapture`.
#[test]
#[ignore = "a measurement"]
fn a_big_source_downloads() {
    use std::io::{BufRead, Read, Write};
    let line = b"Lorem ipsum dolor sit amet, consectetur adipiscing elit 0123\r\n";
    let raw: Vec<u8> = line.iter().copied().cycle().take(60 << 20).collect();
    // The answer made a piece at a time, so making it doesn't set the peak.
    let (head, tail) = (
        "<?xml version=\"1.0\"?><s:Envelope xmlns:s=\"s\"><s:Body><m:GetItemResponse xmlns:m=\"m\" xmlns:t=\"t\"><m:ResponseMessages>\
         <m:GetItemResponseMessage ResponseClass=\"Success\"><m:ResponseCode>NoError</m:ResponseCode><m:Items><t:Message><t:ItemId Id=\"I1\"/>\
         <t:MimeContent CharacterSet=\"UTF-8\">",
        "</t:MimeContent></t:Message></m:Items></m:GetItemResponseMessage></m:ResponseMessages></m:GetItemResponse></s:Body></s:Envelope>",
    );
    let len = head.len() + raw.len().div_ceil(3) * 4 + tail.len();
    let mut answer = format!("HTTP/1.1 200 OK\r\nContent-Type: text/xml; charset=utf-8\r\nContent-Length: {len}\r\n\r\n{head}").into_bytes();
    answer.reserve(len);
    for piece in raw.chunks(3 << 20) {
        answer.extend_from_slice(base64::encode(piece).as_bytes());
    }
    answer.extend_from_slice(tail.as_bytes());
    let answer = std::sync::Arc::new(answer);
    let l = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let url = format!("http://127.0.0.1:{}/EWS/Exchange.asmx", l.local_addr().unwrap().port());
    let served = answer.clone();
    std::thread::spawn(move || {
        for s in l.incoming() {
            let Ok(mut s) = s else { continue };
            let mut r = std::io::BufReader::new(s.try_clone().unwrap());
            loop {
                let mut len = 0usize;
                loop {
                    let mut h = String::new();
                    if r.read_line(&mut h).unwrap_or(0) == 0 {
                        return;
                    }
                    if h == "\r\n" {
                        break;
                    }
                    if let Some(v) = h.to_ascii_lowercase().strip_prefix("content-length:") {
                        len = v.trim().parse().unwrap_or(0);
                    }
                }
                let mut req = vec![0; len];
                r.read_exact(&mut req).unwrap();
                s.write_all(&served).unwrap();
            }
        }
    });
    let c = Client::new(&url, "u", "p", "riley@contoso.example").unwrap();
    let mut times = Vec::new();
    let mut growth = None;
    for round in 0..12 {
        let before = peak_working_set();
        let t = std::time::Instant::now();
        let got = c.download("I1").unwrap();
        times.push(t.elapsed().as_millis());
        if round == 0 {
            growth = before.zip(peak_working_set()).map(|(b, a)| a.saturating_sub(b) >> 20);
        }
        assert!(got == raw, "the source byte for byte");
    }
    let mut sorted = times.clone();
    sorted.sort_unstable();
    let median = sorted.get(sorted.len() / 2).copied().unwrap_or(0);
    let fastest = sorted.first().copied().unwrap_or(0);
    println!(
        "MEASURE ews download 60 MB source: fastest {fastest} ms, median {median} ms, ms per round {times:?}, peak working set grew {growth:?} MB on the first"
    );
}

#[test]
fn autodiscover_answers() {
    let ok = "<Autodiscover><Response><Account><AccountType>email</AccountType><Action>settings</Action>\
                  <Protocol><Type>EXCH</Type><EwsUrl>https://mail.corp.local/EWS/Exchange.asmx</EwsUrl></Protocol>\
                  <Protocol><Type>EXPR</Type><EwsUrl>https://mail.contoso.example/EWS/Exchange.asmx</EwsUrl></Protocol></Account></Response></Autodiscover>";
    assert_eq!(autodiscover_answer(ok, false).0.as_deref(), Some("https://mail.contoso.example/EWS/Exchange.asmx"), "the outside address first");
    let redirect = "<Autodiscover><Response><Account><Action>redirectAddr</Action><RedirectAddr>riley@corp.example</RedirectAddr></Account></Response></Autodiscover>";
    assert_eq!(autodiscover_answer(redirect, false).1.as_deref(), Some("riley@corp.example"));
    assert_eq!(autodiscover_answer("<x/>", false), (None, None));
    let plain = "<Autodiscover><Response><Account><Protocol><Type>EXPR</Type><EwsUrl>http://mail.contoso.example/EWS</EwsUrl></Protocol></Account></Response></Autodiscover>";
    for here in [false, true] {
        assert_eq!(autodiscover_answer(plain, here).0, None, "never plain http");
    }
    // But to this computer, under any of its names, when the server asked is on this computer
    // too (the user's own, or a test's): an answer from the network never names one.
    let at = |url: &str| {
        format!("<Autodiscover><Response><Account><Protocol><Type>EXPR</Type><EwsUrl>{url}</EwsUrl></Protocol></Account></Response></Autodiscover>")
    };
    for here in ["http://127.0.0.1:8080/EWS/Exchange.asmx", "http://localhost/EWS/Exchange.asmx", "http://[::1]:99/EWS/Exchange.asmx"] {
        assert_eq!(autodiscover_answer(&at(here), true).0.as_deref(), Some(here));
        assert_eq!(autodiscover_answer(&at(here), false).0, None, "{here} from the network");
    }
    for not in ["http://127.0.0.1.evil.example/EWS", "ftp://mail.contoso.example/EWS", "https://"] {
        assert_eq!(autodiscover_answer(&at(not), true).0, None, "{not}");
        assert_eq!(autodiscover_answer(&at(not), false).0, None, "{not}");
    }
    // Which server was asked is read from its address, as the client reads it.
    for (asked, here) in [
        ("http://127.0.0.1:8080/autodiscover/autodiscover.xml", true),
        ("https://localhost/autodiscover/autodiscover.xml", true),
        ("http://[::1]:99/autodiscover/autodiscover.xml", true),
        ("https://autodiscover.contoso.example/autodiscover/autodiscover.xml", false),
        ("https://127.0.0.1.evil.example/autodiscover/autodiscover.xml", false),
        ("https://localhost.evil.example/autodiscover/autodiscover.xml", false),
        ("", false),
    ] {
        assert_eq!(discover::asked_here(asked), here, "{asked}");
    }
    assert_eq!(autodiscover_urls("riley@Contoso.example")[0], "https://autodiscover.contoso.example/autodiscover/autodiscover.xml");
    assert!(autodiscover_urls("riley@bad/host").is_empty());
    // A public folder's content mailbox: PR_REPLICA_LIST as Exchange sends it.
    assert_eq!(replica_guid("MWVjMmEyMzYtZWQ5My00Zjg4LWI5YzYtMzNlNjNmYTRhYTQ0AA==").as_deref(), Some("1ec2a236-ed93-4f88-b9c6-33e63fa4aa44"));
    // Its tag, asked for as always and known however an answer writes it.
    assert_eq!(replica_list(), "<t:ExtendedFieldURI PropertyTag=\"26264\" PropertyType=\"Binary\"/>");
    for (tag, is) in
        [("0x6698", true), ("0X6698", true), ("26264", true), (" 26264 ", true), ("0x6699", false), ("6698", false), ("0x", false), ("", false)]
    {
        assert_eq!(is_replica_tag(tag), is, "{tag:?}");
    }
    assert_eq!(replica_guid("bm90IGEgZ3VpZA=="), None);
    assert_eq!(replica_guid("%%%"), None);
    let pf =
        "<Autodiscover><Response><User><AutoDiscoverSMTPAddress>PFContent@contoso.example</AutoDiscoverSMTPAddress></User></Response></Autodiscover>";
    assert_eq!(autodiscover_smtp(pf).as_deref(), Some("PFContent@contoso.example"));
    assert_eq!(autodiscover_smtp("<x/>"), None);
}

#[test]
fn only_https_unless_local() {
    assert!(Client::new("http://mail.contoso.example/EWS/Exchange.asmx", "u", "p", "e").is_err());
    assert!(Client::new("https://mail.contoso.example/EWS/Exchange.asmx", "u", "p", "e").is_ok());
    assert!(Client::new("http://127.0.0.1:9/EWS/Exchange.asmx", "u", "p", "e").is_ok());
}

/// Issue #24: a message's text first from a `GetItem` that asks for its body and files' list, as
/// grommunio answers it; a file of another message is a message; no files, no text first.
#[test]
fn a_message_text_first_from_its_item() {
    let doc = parse(
        "<s:Envelope xmlns:s=\"x\"><s:Body><m:GetItemResponse><m:ResponseMessages><m:GetItemResponseMessage ResponseClass=\"Success\"><m:Items><t:Message>\
         <t:ItemId Id=\"A1\"/><t:Subject>Rushes reel 2</t:Subject>\
         <t:Body BodyType=\"HTML\">&lt;p&gt;Here's reel 2.&lt;/p&gt;</t:Body>\
         <t:Attachments><t:FileAttachment><t:AttachmentId Id=\"F1\"/><t:Name>reel2.mov</t:Name><t:ContentType>video/quicktime</t:ContentType><t:Size>20971586</t:Size></t:FileAttachment>\
         <t:ItemAttachment><t:AttachmentId Id=\"F2\"/><t:Name>Earlier note</t:Name><t:Size>4096</t:Size></t:ItemAttachment></t:Attachments>\
         <t:Importance>High</t:Importance><t:DateTimeSent>2026-10-30T08:02:19Z</t:DateTimeSent>\
         <t:ToRecipients><t:Mailbox><t:Name>Riley</t:Name><t:EmailAddress>riley@studio.test</t:EmailAddress></t:Mailbox></t:ToRecipients>\
         <t:From><t:Mailbox><t:Name>Sam</t:Name><t:EmailAddress>sam@studio.test</t:EmailAddress></t:Mailbox></t:From>\
         <t:InternetMessageId>&lt;r2@studio.test&gt;</t:InternetMessageId></t:Message></m:Items></m:GetItemResponseMessage></m:ResponseMessages></m:GetItemResponse></s:Body></s:Envelope>",
    );
    let raw = text_first::text_first(&doc).expect("its text first");
    assert!(mime::is_text_first(&raw));
    let p = mime::parse(&raw).unwrap();
    assert_eq!((p.subject.as_str(), p.from.email.as_str(), p.message_id.as_str()), ("Rushes reel 2", "sam@studio.test", "r2@studio.test"));
    assert!(p.text.contains("Here's reel 2."), "{}", p.text);
    let files: Vec<(&str, &str, usize)> = p.attachments.iter().map(|a| (a.name.as_str(), a.mime.as_str(), a.size)).collect();
    assert_eq!(files[0], ("reel2.mov", "video/quicktime", 20_971_586));
    assert_eq!((files[1].1, files[1].2), ("message/rfc822", 4096));
    let none = parse("<s:Envelope><s:Body><t:Message><t:Body BodyType=\"Text\">hi</t:Body></t:Message></s:Body></s:Envelope>");
    assert!(text_first::text_first(&none).is_none(), "no files: the whole of it is as quick");
}

/// A sign-in scheme is known by its whole name in any case, and its token read after it.
#[test]
fn sign_in_schemes_by_their_whole_name_in_any_case() {
    assert_eq!(scheme_token("ntlm TlRMTVNTUA==", "NTLM"), Some("TlRMTVNTUA=="));
    assert_eq!(scheme_token("  NTLM  ", "NTLM"), Some(""));
    assert_eq!(scheme_token("NEGOTIATE YII=", "Negotiate"), Some("YII="));
    assert_eq!(scheme_token("Negotiate, NTLM", "Negotiate"), Some(""));
    assert_eq!(scheme_token("Basic realm=\"x\"", "Basic"), Some("realm=\"x\""));
    for (value, scheme) in [("NTLMfoo abc", "NTLM"), ("Negotiate", "NTLM"), ("", "NTLM"), ("Bas", "Basic")] {
        assert_eq!(scheme_token(value, scheme), None, "{value:?}");
    }
}

/// Issue #145: the mailbox server's cookie a load balancer sets is taken as it can go back in a
/// header: among other cookies, in any case; not one with a line break, a separator or no value,
/// nor a huge one.
#[test]
fn the_backend_cookie_is_taken_safely() {
    let set = |values: &[&str]| -> Vec<(String, String)> { values.iter().map(|v| ("Set-Cookie".to_string(), v.to_string())).collect() };
    let got = |values: &[&str]| watch::backend_cookie(&set(values));
    assert_eq!(got(&["ClientId=AB12; path=/", "X-BackEndOverrideCookie=MBX7~1942398215; path=/EWS; secure"]).as_deref(), Some("MBX7~1942398215"));
    assert_eq!(got(&["x-backendoverridecookie=mbx2~9"]).as_deref(), Some("mbx2~9"));
    assert_eq!(watch::backend_cookie(&[("set-cookie".into(), "X-BackEndOverrideCookie=A".into())]).as_deref(), Some("A"));
    for bad in ["X-BackEndOverrideCookie=", "X-BackEndOverrideCookie=a b", "X-BackEndOverrideCookie=a\r\nX-Evil: 1", "X-BackEndOverrideCookie=a\"b"] {
        assert_eq!(got(&[bad]), None, "{bad:?}");
    }
    assert_eq!(got(&[&format!("X-BackEndOverrideCookie={}", "x".repeat(5000))]), None);
    assert_eq!(watch::backend_cookie(&[("Set-Cookie2".into(), "X-BackEndOverrideCookie=A".into())]), None);
}
