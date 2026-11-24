//! Engine tests: the interface's text size (the comment on issue #267), kept with the
//! preferences and stepped by its keys.

use super::*;

#[test]
fn the_text_size_is_kept_and_stepped_by_its_keys() {
    let mut s = Session::blank();
    assert_eq!(s.prefs.text_size, 100, "the interface's own");
    assert_eq!(s.run("view.textSize", &json!({"percent": 150})).unwrap()["percent"], 150);
    // Outside what's kept, or not a number: refused, the size kept.
    for bad in [json!({"percent": 79}), json!({"percent": 201}), json!({"percent": "150"}), json!({"percent": 1.5}), json!({"percent": -100})] {
        assert!(s.run("view.textSize", &bad).is_err(), "{bad}");
    }
    assert_eq!(s.prefs.text_size, 150);

    // Ctrl and +, − and 0, whichever key types the plus.
    let reg = s.registry();
    for (key, id) in [
        ("Mod+Plus", "view.textBigger"),
        ("Mod+Shift+Plus", "view.textBigger"),
        ("Mod+Equals", "view.textBigger"),
        ("Mod+Minus", "view.textSmaller"),
        ("Mod+0", "view.textNormal"),
    ] {
        assert_eq!(reg.by_shortcut(key).map(|c| c.id), Some(id), "{key}");
    }
    // By tens, from a size between them too, and no further than kept.
    assert_eq!(s.run("view.textBigger", &json!({})).unwrap()["percent"], 160);
    for (from, id, to) in [
        (125, "view.textBigger", 130),
        (125, "view.textSmaller", 120),
        (100, "view.textSmaller", 90),
        (200, "view.textBigger", 200),
        (80, "view.textSmaller", 80),
        (175, "view.textNormal", 100),
    ] {
        s.run("view.textSize", &json!({"percent": from})).unwrap();
        assert_eq!(s.run(id, &json!({})).unwrap()["percent"], to, "{id} from {from}");
    }
    assert_eq!(s.run("view.textSize", &json!({"percent": null})).unwrap()["percent"], 100);

    // Kept with the preferences (left out at its own size); read back only within what's kept.
    s.run("view.textSize", &json!({"percent": 175})).unwrap();
    let saved = serde_json::to_value(&s.prefs).unwrap();
    assert_eq!(saved["textSize"], 175);
    for (on_disk, read) in [(json!(175), 175), (json!(500), 100), (json!(10), 100), (json!("big"), 100), (json!(null), 100)] {
        let mut p = saved.clone();
        p["textSize"] = on_disk.clone();
        assert_eq!(serde_json::from_value::<crate::Prefs>(p).unwrap().text_size, read, "{on_disk}");
    }
    s.run("view.textNormal", &json!({})).unwrap();
    assert!(serde_json::to_value(&s.prefs).unwrap().get("textSize").is_none());
}
