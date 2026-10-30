use super::*;

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

#[test]
fn autodiscover_answers() {
    let ok = "<Autodiscover><Response><Account><AccountType>email</AccountType><Action>settings</Action>\
                  <Protocol><Type>EXCH</Type><EwsUrl>https://mail.corp.local/EWS/Exchange.asmx</EwsUrl></Protocol>\
                  <Protocol><Type>EXPR</Type><EwsUrl>https://mail.contoso.example/EWS/Exchange.asmx</EwsUrl></Protocol></Account></Response></Autodiscover>";
    assert_eq!(autodiscover_answer(ok).0.as_deref(), Some("https://mail.contoso.example/EWS/Exchange.asmx"), "the outside address first");
    let redirect = "<Autodiscover><Response><Account><Action>redirectAddr</Action><RedirectAddr>riley@corp.example</RedirectAddr></Account></Response></Autodiscover>";
    assert_eq!(autodiscover_answer(redirect).1.as_deref(), Some("riley@corp.example"));
    assert_eq!(autodiscover_answer("<x/>"), (None, None));
    let plain = "<Autodiscover><Response><Account><Protocol><Type>EXPR</Type><EwsUrl>http://mail.contoso.example/EWS</EwsUrl></Protocol></Account></Response></Autodiscover>";
    assert_eq!(autodiscover_answer(plain).0, None, "never plain http");
    assert_eq!(autodiscover_urls("riley@Contoso.example")[0], "https://autodiscover.contoso.example/autodiscover/autodiscover.xml");
    assert!(autodiscover_urls("riley@bad/host").is_empty());
    // A public folder's content mailbox: PR_REPLICA_LIST as Exchange sends it.
    assert_eq!(replica_guid("MWVjMmEyMzYtZWQ5My00Zjg4LWI5YzYtMzNlNjNmYTRhYTQ0AA==").as_deref(), Some("1ec2a236-ed93-4f88-b9c6-33e63fa4aa44"));
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
