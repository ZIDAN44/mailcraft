//! Engine tests: categories' keywords, as other programs make and read them (issue #28).

use super::*;

/// New categories get the keyword Thunderbird gives a tag of the same name; the five defaults
/// keep theirs; a keyword can be given (`$label6`, as SOGo numbers them) and is checked.
#[test]
fn new_categories_take_thunderbirds_keywords() {
    let mut s = sample_session("cat-keys");
    let kw =
        |s: &mut Session, name: &str| s.run("category.create", &json!({"name": name})).map(|r| r["keyword"].as_str().unwrap_or_default().to_string());
    assert_eq!(kw(&mut s, "Rückfrage").unwrap(), "r=c3=bcckfrage");
    assert_eq!(kw(&mut s, "Vertraulich").unwrap(), "vertraulich");
    assert_eq!(kw(&mut s, "Team task").unwrap(), "team=20task");
    assert_eq!(s.category("To do").map(|c| c.keyword.as_str()), Some("$label4"), "the defaults keep Thunderbird's own");
    // A name whose key another category has: numbered, as Thunderbird does.
    s.run("category.edit", &json!({"name": "Vertraulich", "keyword": "geheim"})).unwrap();
    assert_eq!(kw(&mut s, "Geheim").unwrap(), "geheim_1");
    assert_eq!(s.category("Vertraulich").map(|c| c.keyword.as_str()), Some("geheim"), "never re-keyed");

    // Given keywords.
    let r = s.run("category.create", &json!({"name": "Projekt", "keyword": "$label6", "color": 5})).unwrap();
    assert_eq!((r["keyword"].as_str(), r["color"].as_u64()), (Some("$label6"), Some(5)));
    for bad in ["two words", "a(b", "$Junk", "NonJunk", "\\Seen", "ü", &"x".repeat(51)] {
        assert!(s.run("category.create", &json!({"name": "Bad", "keyword": bad})).is_err(), "{bad}");
    }
    let taken = s.run("category.create", &json!({"name": "Other", "keyword": "$LABEL6"})).unwrap_err().to_string();
    assert!(taken.contains("Projekt"), "{taken}");
    assert!(s.run("category.create", &json!({"name": "  "})).is_err());
    assert!(s.run("category.create", &json!({"name": "a\u{7}b"})).is_err());
    assert!(s.run("category.create", &json!({"name": "x".repeat(61)})).is_err());
}

/// Keywords on mail that aren't in the list (a colleague's Thunderbird or SOGo tag) show as
/// categories, named from the keyword; system keywords don't; adding one to the list keeps
/// its keyword.
#[test]
fn keywords_not_in_the_list_show_as_categories() {
    let mut s = sample_session("cat-unlisted");
    let k = key_of(&s, "Character turnarounds");
    s.run("list.select", &k).unwrap();
    let key = mailcraft_model::MessageKey::from_id(k["key"].as_str().unwrap()).unwrap();
    // As a colleague's Thunderbird and SOGo would leave them, with junk marks and a receipt.
    for kw in ["r=c3=bcckfrage", "$label6", "to=20do", "$Junk", "NonJunk", "$MDNSent", "$label2"] {
        s.apply_op("sample", mailcraft_model::Op::SetKeyword { keys: vec![key.clone()], keyword: kw.into(), on: true });
    }
    let m = s.selected()[0].clone();
    let shown: Vec<(String, String)> = s.categories_of(&m).into_iter().map(|c| (c.name, c.keyword)).collect();
    assert_eq!(
        shown,
        [("Work", "$label2"), ("$label6", "$label6"), ("rückfrage", "r=c3=bcckfrage"), ("to do", "to=20do")]
            .map(|(a, b)| (a.to_string(), b.to_string()))
    );
    assert!(s.is_categorized(&m));
    let unlisted = s.run("category.unlisted", &json!({})).unwrap();
    let names: Vec<&str> = unlisted.as_array().unwrap().iter().filter_map(|v| v["name"].as_str()).collect();
    assert_eq!(names, ["$label6", "rückfrage", "to do"]);
    // Filter and search find them too.
    s.run("list.filter", &json!({"value": "categorized"})).unwrap();
    assert!(s.list.iter().any(|x| x.key == m.key));
    s.run("list.filter", &json!({"value": "all"})).unwrap();
    assert_eq!(s.run("search.run", &json!({"query": "category:rückfrage"})).unwrap()["results"], 1);
    s.run("search.clear", &json!({})).unwrap();

    // Taken off by its keyword, put back the same way.
    s.run("list.select", &k).unwrap();
    s.run("mail.categorize", &json!({"keyword": "to=20do", "on": false})).unwrap();
    assert!(!s.selected()[0].flags.has("to=20do"));
    s.run("mail.categorize", &json!({"keyword": "to=20do"})).unwrap();
    assert!(s.selected()[0].flags.has("to=20do"));
    assert!(s.run("mail.categorize", &json!({"keyword": "$Junk"})).is_err(), "not a category's keyword");
    assert!(s.run("mail.categorize", &json!({"keyword": "a b"})).is_err());
    assert!(s.run("mail.categorize", &json!({})).is_err());

    // Named and coloured: in the list with the same keyword, its name instead of the keyword's.
    s.run("category.create", &json!({"name": "Rückfrage", "keyword": "r=c3=bcckfrage", "color": 6})).unwrap();
    let m = s.selected()[0].clone();
    let c = s.categories_of(&m).into_iter().find(|c| c.keyword == "r=c3=bcckfrage").unwrap();
    assert_eq!((c.name.as_str(), c.color), ("Rückfrage", 6));
    // Deleted from the list: the mail still shows it, as one not in the list.
    s.run("category.delete", &json!({"name": "Rückfrage"})).unwrap();
    assert!(s.categories_of(&m).iter().any(|c| c.name == "rückfrage"));
}

/// Renaming keeps the keyword and takes rules and Quick Steps along; a new keyword is checked;
/// the list goes to a file and comes back on another computer with the same keywords.
#[test]
fn categories_edit_export_and_import() {
    let mut s = sample_session("cat-edit");
    s.run("category.create", &json!({"name": "Clients"})).unwrap();
    s.run("rules.add", &json!({"rule": {"name": "Print", "conditions": [{"if": "from", "value": "printshop"}], "actions": [{"do": "categorize", "value": "Clients"}]}}))
        .unwrap();
    s.run("quickStep.create", &json!({"name": "Client", "actions": [{"do": "categorize", "value": "clients"}]})).unwrap();
    let r = s.run("category.edit", &json!({"name": "clients", "newName": "Kunden", "color": 3})).unwrap();
    assert_eq!((r["keyword"].as_str(), r["followed"].as_u64()), (Some("clients"), Some(2)), "{r}");
    assert!(s.category("Clients").is_none() && s.category("Kunden").is_some_and(|c| c.color == 3 && c.keyword == "clients"));
    assert_eq!(s.rules[0].actions, [mailcraft_model::organize::RuleAction::Categorize("Kunden".into())]);
    assert_eq!(s.prefs.quick_steps[0].actions, [mailcraft_model::organize::RuleAction::Categorize("Kunden".into())]);
    assert!(s.run("category.edit", &json!({"name": "Kunden", "newName": "work"})).is_err(), "taken");
    assert!(s.run("category.edit", &json!({"name": "Kunden", "keyword": "$label1"})).is_err(), "Important's");
    assert!(s.run("category.edit", &json!({"name": "Kunden", "keyword": "bad word"})).is_err());
    assert!(s.run("category.edit", &json!({"name": "Nope", "color": 1})).is_err());
    s.run("category.edit", &json!({"name": "Kunden", "keyword": "$label6"})).unwrap();
    s.run("category.edit", &json!({"name": "Kunden", "keyword": "$LABEL6"})).unwrap();
    // The same name in another case is a rename, not a clash.
    s.run("category.edit", &json!({"name": "Kunden", "newName": "KUNDEN"})).unwrap();

    let file = s.store.as_ref().unwrap().root().join("team.json");
    s.run("category.export", &json!({"path": file.to_string_lossy()})).unwrap();
    let mut other = sample_session("cat-edit-2");
    other.run("category.edit", &json!({"name": "Work", "color": 7})).unwrap();
    other.run("category.create", &json!({"name": "Mine", "keyword": "$label6"})).unwrap();
    let r = other.run("category.import", &json!({"path": file.to_string_lossy()})).unwrap();
    assert_eq!((r["added"].as_u64(), r["changed"].as_u64()), (Some(0), Some(1)), "{r}");
    assert_eq!(r["skipped"], json!(["KUNDEN"]), "its keyword is another's here");
    other.run("category.delete", &json!({"name": "Mine"})).unwrap();
    let r = other.run("category.import", &json!({"path": file.to_string_lossy()})).unwrap();
    assert_eq!(r["added"].as_u64(), Some(1));
    assert_eq!(other.category("kunden").map(|c| c.keyword.as_str()), Some("$label6"));
    assert_eq!(other.category("Work").map(|c| c.color), Some(1), "the file's colour");

    // Not a categories file, a broken one, a huge one.
    std::fs::write(&file, b"{\"rules\": []}").unwrap();
    assert!(other.run("category.import", &json!({"path": file.to_string_lossy()})).is_err());
    std::fs::write(&file, b"\xEF\xBB\xBF{\"mailcraftCategories\": 1, \"categories\": [{\"name\": 5}, {\"name\": \"\", \"keyword\": \"x\"}, {\"name\": \"Ok\", \"keyword\": \"a b\"}]}").unwrap();
    let r = other.run("category.import", &json!({"path": file.to_string_lossy()})).unwrap();
    assert_eq!((r["added"].as_u64(), r["skipped"].as_array().map(Vec::len)), (Some(0), Some(3)));
    std::fs::write(&file, vec![b' '; 2 << 20]).unwrap();
    assert!(other.run("category.import", &json!({"path": file.to_string_lossy()})).is_err());
}

/// A rules file carries the keywords of the categories its rules use: another computer makes a
/// missing one with the same keyword (not one made again from the name).
#[test]
fn rules_files_carry_their_categories_keywords() {
    let mut s = sample_session("cat-rules");
    s.run("category.create", &json!({"name": "Team", "keyword": "$label7", "color": 2})).unwrap();
    s.run(
        "rules.add",
        &json!({"rule": {"name": "T", "conditions": [{"if": "from", "value": "team@"}], "actions": [{"do": "categorize", "value": "Team"}]}}),
    )
    .unwrap();
    let file = s.store.as_ref().unwrap().root().join("rules.json");
    s.run("rules.export", &json!({"path": file.to_string_lossy()})).unwrap();
    let mut other = sample_session("cat-rules-2");
    other.run("rules.import", &json!({"path": file.to_string_lossy()})).unwrap();
    assert_eq!(other.category("Team").map(|c| (c.keyword.as_str(), c.color)), Some(("$label7", 2)));
    // Its keyword taken here: the category is still made, with one from its name.
    let mut third = sample_session("cat-rules-3");
    third.run("category.create", &json!({"name": "Mine", "keyword": "$label7"})).unwrap();
    third.run("rules.import", &json!({"path": file.to_string_lossy()})).unwrap();
    assert_eq!(third.category("Team").map(|c| c.keyword.as_str()), Some("team"));
}

/// A long list with many keywords: the dialog's list of those not in the category list is
/// counted in one pass, at most 100, most used first, any case counted as one.
#[test]
fn many_unlisted_keywords_are_counted_and_capped() {
    let mut s = sample_session("cat-many");
    let base = s.list[0].clone();
    s.list = (0..20_000u32)
        .map(|i| {
            let mut m = base.clone();
            m.key.uid = i + 1;
            let kws: Vec<String> = (0..30).map(|j| format!("k{}", (i + j) % 400)).chain(["$Junk".into(), "Common".into()]).collect();
            m.flags = mailcraft_model::Flags::from_imap(kws.iter().map(String::as_str).chain(if i % 2 == 0 { Some("COMMON") } else { None }));
            m
        })
        .collect();
    let started = std::time::Instant::now();
    let found = s.unlisted_keywords();
    eprintln!("{} keywords counted in {:?}", found.len(), started.elapsed());
    assert_eq!(found.len(), 100);
    assert_eq!(found[0], ("Common".to_string(), 20_000), "one keyword whatever its case");
    assert!(!found.iter().any(|(k, _)| k == "$Junk"));
}
