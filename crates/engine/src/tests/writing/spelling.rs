//! Engine tests: spelling as you type (issue #174): on unless turned off, and the words added to
//! the dictionary, kept with the preferences.

use super::*;

#[test]
fn the_spelling_settings_and_dictionary_are_kept() {
    let mut s = sample_session("spelling");
    assert!(s.prefs.spelling, "on unless turned off");
    assert_eq!(s.run("options.spelling", &json!({"on": false})).unwrap()["on"], false);
    assert!(!s.prefs.spelling);
    s.run("options.spelling", &json!({})).unwrap();
    assert!(s.prefs.spelling, "toggled back");

    // Added, trimmed; again, kept once; taken out.
    assert_eq!(s.run("spelling.addWord", &json!({"word": " MailCraft "})).unwrap()["added"], true);
    assert_eq!(s.run("spelling.addWord", &json!({"word": "MailCraft"})).unwrap()["added"], false);
    assert_eq!(s.prefs.dictionary.iter().collect::<Vec<_>>(), ["MailCraft"]);
    assert_eq!(s.run("spelling.addWord", &json!({"word": "MailCraft", "remove": true})).unwrap()["removed"], true);
    assert!(s.prefs.dictionary.is_empty());
    // Not words: refused.
    for bad in [json!({"word": ""}), json!({"word": "two words"}), json!({"word": "x".repeat(101)}), json!({"word": "a\u{7}b"}), json!({})] {
        assert!(s.run("spelling.addWord", &bad).is_err(), "{bad}");
    }

    // Kept with the preferences, and read back from disk only as words.
    s.run("spelling.addWord", &json!({"word": "Riley's"})).unwrap();
    let saved = serde_json::to_value(&s.prefs).unwrap();
    assert_eq!(saved["dictionary"], json!(["Riley's"]));
    let mut read = saved.clone();
    read["dictionary"] = json!(["ok", "", "two words", "x".repeat(500), "ok"]);
    let p: crate::Prefs = serde_json::from_value(read).unwrap();
    assert_eq!(p.dictionary.iter().collect::<Vec<_>>(), ["ok"]);
    let mut off = saved;
    off["spelling"] = json!(false);
    assert!(!serde_json::from_value::<crate::Prefs>(off).unwrap().spelling);
}

#[test]
fn spelling_language_preference_and_command() {
    let mut s = sample_session("spelling_lang");
    assert_eq!(s.prefs.spelling_language, None);

    // Normalizes tags and saves preference.
    let res = s.run("options.spellingLanguage", &json!({"language": "en-us"})).unwrap();
    assert_eq!(res["language"], "en_US");
    assert_eq!(s.prefs.spelling_language.as_deref(), Some("en_US"));

    let res = s.run("options.spellingLanguage", &json!({"language": "bg_bg"})).unwrap();
    assert_eq!(res["language"], "bg_BG");
    assert_eq!(s.prefs.spelling_language.as_deref(), Some("bg_BG"));

    // Persists through serialization and rejects damaged values on load.
    let saved = serde_json::to_value(&s.prefs).unwrap();
    assert_eq!(saved["spellingLanguage"], json!("bg_BG"));
    let loaded: crate::Prefs = serde_json::from_value(saved.clone()).unwrap();
    assert_eq!(loaded.spelling_language.as_deref(), Some("bg_BG"));

    let mut damaged = saved;
    damaged["spellingLanguage"] = json!("../bad");
    assert_eq!(serde_json::from_value::<crate::Prefs>(damaged).unwrap().spelling_language, None);

    // Null or "auto" resets to following the UI language.
    let res = s.run("options.spellingLanguage", &json!({"language": "auto"})).unwrap();
    assert_eq!(res["language"], json!(null));
    assert_eq!(s.prefs.spelling_language, None);

    s.run("options.spellingLanguage", &json!({"language": "de_DE"})).unwrap();
    let res = s.run("options.spellingLanguage", &json!({"language": null})).unwrap();
    assert_eq!(res["language"], json!(null));

    // Invalid inputs return Params errors.
    for bad in [json!({"language": "../etc/passwd"}), json!({"language": 123}), json!({"language": "a".repeat(50)})] {
        assert!(s.run("options.spellingLanguage", &bad).is_err(), "{bad}");
    }
}
