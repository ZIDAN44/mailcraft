use super::*;

/// Where a test's socket goes, in a folder of its own, its name kept short: a socket's whole
/// path must fit its address (103 bytes on macOS, whose temporary folder alone takes about 50).
fn socket_at(test: &str) -> PathBuf {
    std::env::temp_dir().join(format!("mci-{:x}-{test}", std::process::id())).join(socket_name(Path::new("C:/data"), ""))
}

/// That socket's folder, emptied.
fn socket(test: &str) -> PathBuf {
    let path = socket_at(test);
    if let Some(dir) = path.parent() {
        let _ = std::fs::remove_dir_all(dir);
    }
    path
}

/// Every socket fits a Unix socket's address (`sun_path`: 104 bytes on macOS, 108 elsewhere,
/// the NUL included). One that doesn't can't be listened at, and every launch opened a window
/// of its own: the tests' did on the macOS CI runner, with a five-digit process id.
#[test]
fn socket_paths_fit_a_socket_address() {
    let app = socket_path(Path::new("/home/zoe/.local/share/mailcraft"));
    assert_eq!(app.is_some(), mailcraft_engine::store::scratch_dir("instance").is_some(), "a socket wherever its folder can be");
    let app = app.unwrap_or_default();
    for p in [
        app,
        socket_at("together"),
        socket_at("stale"),
        socket_at("silent"),
        socket_at("show"),
        socket_at("keyed"),
        socket_at("handed"),
        socket_at("lost"),
        socket_at("early"),
        socket_at("patient"),
    ] {
        let len = p.as_os_str().len();
        assert!(len < 104, "{len} bytes: {}", p.display());
    }
}

/// Windows names the screen by the session the process runs in, `SESSIONNAME` only when the
/// id can't be read.
#[test]
fn each_windows_session_has_its_own_socket() {
    let name = |id, n| socket_name(Path::new(r"C:\d"), &windows_session(id, n));
    // A launch without SESSIONNAME (Task Scheduler) in the console's session finds the
    // window opened from Explorer there...
    assert_eq!(name(Some(1), None), name(Some(1), Some("Console")));
    // ...and one in a Remote Desktop session never reaches the console's.
    assert_ne!(name(Some(2), None), name(Some(1), Some("Console")));
    assert_ne!(name(Some(2), Some("RDP-Tcp#3")), name(Some(3), Some("RDP-Tcp#3")));
    // Only without an id does SESSIONNAME name it.
    assert_eq!(name(None, Some("Console")), socket_name(Path::new(r"C:\d"), "Console"));
    assert_ne!(name(None, Some("RDP-Tcp#3")), name(None, Some("RDP-Tcp#4")));
    assert_eq!(name(None, None), socket_name(Path::new(r"C:\d"), ""));
}

/// The socket is named by the session this process runs in, which Windows always gives.
#[cfg(windows)]
#[test]
fn this_process_has_a_session_id() {
    let id = session_id();
    assert!(id.is_some(), "the access token names the session");
    assert!(screen().starts_with('s'));
}

/// One data folder, one name; another folder (`--data`, `--sample`, a portable copy) or
/// another screen, another name; never a path.
#[test]
fn each_data_folder_and_screen_has_its_own_socket() {
    let a = socket_name(Path::new("/home/zoe/.local/share/mailcraft"), "wayland-0");
    assert_eq!(a, socket_name(Path::new("/home/zoe/.local/share/mailcraft/"), "wayland-0"));
    assert_ne!(a, socket_name(Path::new("/home/zoe/.local/share/mailcraft/sample"), "wayland-0"));
    assert_ne!(a, socket_name(Path::new("/media/usb/MailCraftData"), "wayland-0"));
    assert_ne!(a, socket_name(Path::new("/home/zoe/.local/share/mailcraft"), "wayland-1"));
    assert_ne!(socket_name(Path::new("/d"), "RDP-Tcp#3"), socket_name(Path::new("/d"), "Console"));
    for odd in ["../../x/y", "RDP-Tcp#3", &"z".repeat(200), ""] {
        let n = socket_name(Path::new("/d"), odd);
        assert!(n.ends_with(".sock") && n.matches('.').count() == 1 && !n.contains(['/', '\\', '#']), "{n}");
        assert!(n.len() <= 16 + 1 + 24 + 5, "{n}");
    }
    #[cfg(windows)]
    assert_eq!(
        socket_name(Path::new(r"C:\Users\Zoe\AppData\Roaming\MailCraft"), ""),
        socket_name(Path::new(r"c:\users\zoe\appdata\roaming\mailcraft"), "")
    );
}

/// The first launch listens; a second asks it to show its window and exits, and the first
/// sees the ask once; closing removes the socket, and the next launch is the first again.
#[test]
fn a_second_launch_shows_the_first() {
    let path = socket("show");
    let Claim::Primary(first) = claim_at(&path, &Handed::default()) else { panic!("the first launch listens") };
    assert!(first.take().is_none());
    assert!(matches!(claim_at(&path, &Handed::default()), Claim::Shown));
    assert!(matches!(claim_at(&path, &Handed::default()), Claim::Shown));
    assert_eq!(first.take(), Some(Handed::default()), "asked");
    assert!(first.take().is_none(), "once");
    // Woken when the window exists.
    let ctx = egui::Context::default();
    first.connect(&ctx);
    assert!(matches!(claim_at(&path, &Handed::default()), Claim::Shown));
    assert!(first.take().is_some());
    first.close();
    assert!(std::fs::symlink_metadata(&path).is_err(), "the socket goes");
    assert!(matches!(claim_at(&path, &Handed::default()), Claim::Primary(_)), "no one listens: this one is first");
    let _ = std::fs::remove_dir_all(path.parent().unwrap());
}

/// Found in audit (#242): a launch whose connect failed just before another bound the
/// socket (without the election lock, which is best effort) wrote its key over the other's,
/// then found the socket taken. The running MailCraft's later launches sent that key and
/// were refused, each opening a window of its own. A key takes its name only once its
/// launch listens.
#[test]
fn a_launch_that_loses_the_socket_leaves_the_key() {
    let path = socket("lost");
    let Claim::Primary(first) = claim_at(&path, &Handed::default()) else { panic!("the first launch listens") };
    let key = read_key(&path).unwrap();
    // The other launch's step after its connect failed, while the first listens.
    assert!(matches!(listen(&path), Ok(None)), "the socket is the first's");
    assert_eq!(read_key(&path).unwrap(), key, "the first's key stays");
    assert!(matches!(claim_at(&path, &Handed::default()), Claim::Shown), "later launches still reach it");
    assert_eq!(first.take(), Some(Handed::default()));
    // Nothing of the other launch's is left beside the socket: the socket, its lock, its key.
    let dir = path.parent().unwrap();
    let mut left: Vec<String> = std::fs::read_dir(dir).unwrap().flatten().map(|e| e.file_name().to_string_lossy().into_owned()).collect();
    left.sort();
    let name = |ext: &str| path.with_extension(ext).file_name().unwrap().to_string_lossy().into_owned();
    assert_eq!(left, [name("key"), name("lock"), name("sock")]);
    first.close();
    let _ = std::fs::remove_dir_all(dir);
}

/// A launch that reaches the running MailCraft before its key is in place (it listens first;
/// the election lock, when it can't be had, doesn't keep the launch out) asks again once it
/// is: with no key beside the socket, or the key of the run before, it was refused and opened
/// a window of its own.
#[test]
fn a_launch_before_the_key_is_in_place_asks_again() {
    let path = socket("early");
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    let open = Handed::opening(vec!["mailto:ana@x.example".to_string()]);
    for before in [None, Some("the-key-of-the-run-before")] {
        // Listening, its key not yet under its name.
        let listener = UnixListener::bind(&path).unwrap();
        let Claim::Primary(first) = serve(listener, &path, "this-runs-key".into()) else { panic!("it answers") };
        let _ = std::fs::remove_file(path.with_extension("key"));
        if let Some(old) = before {
            write_key(&path.with_extension("key"), old).unwrap();
        }
        let mut asked = 0;
        let stream = UnixStream::connect(&path).unwrap();
        let handed = hand_over(stream, &path, &open, || {
            asked += 1;
            // The key takes its name after the launch's second ask.
            if asked == 2 {
                write_key(&path.with_extension("key"), "this-runs-key").unwrap();
            }
        });
        assert!(handed.is_ok(), "{before:?}: {handed:?}");
        assert_eq!((asked, first.take()), (2, Some(open.clone())), "{before:?}");
        // A key that never comes: given up on after a few asks, for a window of its own.
        let _ = std::fs::remove_file(path.with_extension("key"));
        let mut asked = 0;
        let stream = UnixStream::connect(&path).unwrap();
        assert!(hand_over(stream, &path, &open, || asked += 1).is_err());
        assert_eq!((asked, first.take()), (KEY_TRIES - 1, None));
        first.close();
    }
    let _ = std::fs::remove_dir_all(path.parent().unwrap());
}

/// Found while a test failed one run in twenty: a read on the socket is made again every
/// [`READ_SLICE`] until its time is up, so a close Windows didn't wake it for is seen at the
/// next (it waited out all of [`TIMEOUT`], and the ask behind it timed out). What comes after
/// a few slices is still read, and the read gives up when its time is.
#[test]
fn a_read_is_made_again_until_its_time_is_up() {
    use std::io::{Read, Write};
    let path = socket("patient");
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    let listener = UnixListener::bind(&path).unwrap();
    let mut late = UnixStream::connect(&path).unwrap();
    let (ours, _) = listener.accept().unwrap();
    let writer = std::thread::spawn(move || {
        std::thread::sleep(READ_SLICE * 4);
        late.write_all(b"ok\n").unwrap();
        late
    });
    let mut got = [0u8; 3];
    Patient::new(&ours).read_exact(&mut got).unwrap();
    assert_eq!(&got, b"ok\n", "read after four slices");
    let late = writer.join().unwrap();
    // Nothing more comes: the read gives up at its time, not at the first slice.
    let started = std::time::Instant::now();
    let until = started + READ_SLICE * 5;
    let gave_up = Patient { stream: &ours, until }.read(&mut got);
    assert_eq!(gave_up.map_err(|e| e.kind()), Err(std::io::ErrorKind::TimedOut));
    assert!(started.elapsed() >= READ_SLICE * 5, "{:?}", started.elapsed());
    // A close is an end, as before.
    drop(late);
    assert_eq!(Patient::new(&ours).read(&mut got).unwrap(), 0);
    drop((ours, listener));
    let _ = std::fs::remove_dir_all(path.parent().unwrap());
}

/// Several launches at once (a double click and then some): one window.
#[test]
fn launches_started_together_open_one_window() {
    let path = socket("together");
    let barrier = Arc::new(std::sync::Barrier::new(8));
    let launches: Vec<_> = (0..8)
        .map(|_| {
            let (path, barrier) = (path.clone(), Arc::clone(&barrier));
            std::thread::spawn(move || {
                barrier.wait();
                claim_at(&path, &Handed::default())
            })
        })
        .collect();
    let claims: Vec<Claim> = launches.into_iter().map(|t| t.join().unwrap()).collect();
    assert_eq!(claims.iter().filter(|c| matches!(c, Claim::Primary(_))).count(), 1, "one window");
    assert_eq!(claims.iter().filter(|c| matches!(c, Claim::Shown)).count(), 7, "the rest show it");
    let _ = std::fs::remove_dir_all(path.parent().unwrap());
}

/// A socket left by a MailCraft that didn't exit cleanly is replaced; something that isn't
/// MailCraft at the other end, or junk sent to it, opens a window of its own.
#[test]
fn a_stale_socket_is_replaced_and_junk_is_refused() {
    let path = socket("stale");
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    drop(UnixListener::bind(&path).unwrap());
    assert!(std::fs::symlink_metadata(&path).is_ok(), "left behind");
    let Claim::Primary(first) = claim_at(&path, &Handed::default()) else { panic!("replaced") };
    {
        use std::io::{Read, Write};
        let mut s = UnixStream::connect(&path).unwrap();
        // Bounded, as a launch's own read is: a close Windows loses can't hang the test.
        s.set_read_timeout(Some(TIMEOUT * 2)).unwrap();
        s.write_all(b"open C:/x\n").unwrap();
        let mut reply = Vec::new();
        let _ = s.read_to_end(&mut reply);
        assert!(reply.is_empty(), "no ok for junk");
    }
    assert!(first.take().is_none());
    first.close();

    // A listener that never answers: a window of its own after the wait.
    let silent = socket("silent");
    std::fs::create_dir_all(silent.parent().unwrap()).unwrap();
    let _listener = UnixListener::bind(&silent).unwrap();
    assert!(matches!(claim_at(&silent, &Handed::default()), Claim::Alone));
    let _ = std::fs::remove_dir_all(path.parent().unwrap());
    let _ = std::fs::remove_dir_all(silent.parent().unwrap());
}

/// `p` (`/u/a.pdf`) as an absolute path on this system: on Windows, on drive C.
fn abs(p: &str) -> String {
    if cfg!(windows) { format!("C:{p}") } else { p.to_string() }
}

/// What a launch hands over reaches the running window, in order, once; the request carries
/// the run's key, which is in a file beside the socket and new each run.
#[test]
fn a_launch_hands_over_its_links_and_files() {
    let path = socket("handed");
    let Claim::Primary(first) = claim_at(&path, &Handed::default()) else { panic!("the first launch listens") };
    let key = read_key(&path).unwrap();
    assert_eq!(key, first.key);
    assert!(key.len() >= 40, "{key}");
    let open = vec!["mailto:ana@x.example?subject=Hi".to_string(), abs("/Users/zoe/Downloads/Invite.ics")];
    assert!(matches!(claim_at(&path, &Handed::opening(open.clone())), Claim::Shown));
    assert!(matches!(claim_at(&path, &Handed::opening(vec![abs("/home/zoe/x.eml")])), Claim::Shown));
    let mut all = open.clone();
    all.push(abs("/home/zoe/x.eml"));
    assert_eq!(first.take(), Some(Handed::opening(all)));
    assert!(first.take().is_none());
    first.close();
    assert!(std::fs::symlink_metadata(path.with_extension("key")).is_err(), "the key goes with the socket");
    let Claim::Primary(second) = claim_at(&path, &Handed::default()) else { panic!("first again") };
    assert_ne!(second.key, key, "a new key each run");
    second.close();
    let _ = std::fs::remove_dir_all(path.parent().unwrap());
}

/// Another program on the computer can't open anything through the socket: a request without
/// the key, with another key, or in the old form gets no `ok` and hands nothing over.
#[test]
fn requests_without_the_key_are_refused() {
    use std::io::{Read, Write};
    let path = socket("keyed");
    let Claim::Primary(first) = claim_at(&path, &Handed::default()) else { panic!("the first launch listens") };
    let guesses = [
        "show\n".to_string(),
        "{\"open\":[\"mailto:a@x.example\"]}\n".into(),
        request("", &Handed::opening(vec!["mailto:a@x.example".into()])),
        request("not-the-key", &Handed::opening(vec!["mailto:a@x.example".into()])),
        request(&format!("{}x", first.key), &Handed::opening(vec!["mailto:a@x.example".into()])),
        request(first.key.get(1..).unwrap(), &Handed::opening(vec!["mailto:a@x.example".into()])),
        format!("{{\"key\":\"{}\",\"open\":\"mailto:a@x.example\"}}\n", first.key),
        format!("{{\"key\":\"{}\",\"open\":[1]}}\n", first.key),
    ];
    for g in &guesses {
        let mut s = UnixStream::connect(&path).unwrap();
        s.set_read_timeout(Some(TIMEOUT * 2)).unwrap();
        s.write_all(g.as_bytes()).unwrap();
        let mut reply = Vec::new();
        let _ = s.read_to_end(&mut reply);
        assert!(reply.is_empty(), "no ok for {g:?}");
    }
    assert!(first.take().is_none(), "nothing handed over");
    // The right key: accepted.
    let mut s = UnixStream::connect(&path).unwrap();
    s.set_read_timeout(Some(TIMEOUT * 2)).unwrap();
    s.write_all(request(&first.key, &Handed::opening(vec!["mailto:a@x.example".into()])).as_bytes()).unwrap();
    let mut reply = String::new();
    let _ = s.read_to_string(&mut reply);
    assert_eq!(reply, OK);
    assert_eq!(first.take(), Some(Handed::opening(vec!["mailto:a@x.example".to_string()])));
    first.close();
    let _ = std::fs::remove_dir_all(path.parent().unwrap());
}

/// The request's parser: the key exactly, a list of strings, empties and extras left out.
#[test]
fn requests_are_read_strictly() {
    let open: Vec<String> = (0..100).map(|i| abs(&format!("/m/{i}.eml"))).collect();
    let line = request("k3y", &Handed::opening(open.clone()));
    assert!(line.ends_with('\n') && !line.trim_end().contains('\n'), "one line");
    assert_eq!(parse_request(&line, "k3y").map(|h| h.open.len()), Some(MAX_OPEN));
    assert_eq!(parse_request(&line, "k3Y"), None);
    assert_eq!(parse_request(&line, ""), None, "no key, no requests");
    assert_eq!(parse_request(&request("", &Handed::default()), ""), None);
    assert_eq!(
        parse_request("{\"key\":\"k\",\"open\":[\"\",\" \",\"mailto:a@x.example\"]}", "k"),
        Some(Handed::opening(vec!["mailto:a@x.example".to_string()]))
    );
    // A notification's link, clicked in the notification centre (issue #273).
    assert_eq!(
        parse_request("{\"key\":\"k\",\"open\":[\"mailcraft:show?key=a%3A1%3A2%3AINBOX\",\"https://x.example\"]}", "k"),
        Some(Handed::opening(vec!["mailcraft:show?key=a%3A1%3A2%3AINBOX".to_string()]))
    );
    // A path that isn't absolute (a launch makes them so) would be opened from this MailCraft's
    // folder: left out, here and among the files to send.
    let relative = request("k", &Handed { open: vec!["x.eml".into(), abs("/m/y.eml")], attach: vec![vec!["a.pdf".into(), abs("/u/b.pdf")]] });
    assert_eq!(parse_request(&relative, "k"), Some(Handed::new(vec![abs("/m/y.eml")], vec![abs("/u/b.pdf")])));
    for junk in ["", "{}", "[]", "null", "{\"key\":\"k\"}", "{\"key\":5,\"open\":[]}", "\u{0}\u{ff}"] {
        assert_eq!(parse_request(junk, "k"), None, "{junk:?}");
    }
    assert!(keys_match(b"abc", b"abc") && !keys_match(b"abc", b"abd") && !keys_match(b"abc", b"ab"));
}

/// Issue #96: a launch's files to send reach the running window as one group, a new message
/// each; a request from a MailCraft before 0.39 (no `attach`) is read as ever, and one whose
/// `attach` isn't a list of strings is refused.
#[test]
fn a_launch_hands_over_its_files_to_send() {
    let path = socket("attach");
    let Claim::Primary(first) = claim_at(&path, &Handed::default()) else { panic!("the first launch listens") };
    let files = |a: &[&str]| a.iter().map(|s| s.to_string()).collect::<Vec<_>>();
    let paths = |a: &[&str]| a.iter().map(|s| abs(s)).collect::<Vec<_>>();
    let one = Handed { open: Vec::new(), attach: vec![paths(&["/u/report.pdf", "/u/photo.jpg"])] };
    let two = Handed { open: files(&["mailto:a@x.example"]), attach: vec![paths(&["/u/notes.txt"])] };
    assert!(matches!(claim_at(&path, &one), Claim::Shown));
    assert!(matches!(claim_at(&path, &two), Claim::Shown));
    let got = first.take().unwrap();
    assert_eq!(got.open, files(&["mailto:a@x.example"]));
    assert_eq!(got.attach, [paths(&["/u/report.pdf", "/u/photo.jpg"]), paths(&["/u/notes.txt"])], "a message each, in order");
    // As many as wait at once.
    for _ in 0..MAX_WAITING_SENDS + 4 {
        first.ask(one.clone());
    }
    assert_eq!(first.take().unwrap().attach.len(), MAX_WAITING_SENDS);
    first.close();
    let _ = std::fs::remove_dir_all(path.parent().unwrap());

    let many: Vec<String> = (0..500).map(|i| abs(&format!("/u/{i}.pdf"))).collect();
    let read = parse_request(&request("k", &Handed { open: Vec::new(), attach: vec![many] }), "k").unwrap();
    assert_eq!(read.attach.iter().map(Vec::len).collect::<Vec<_>>(), [MAX_ATTACHED]);
    assert_eq!(
        parse_request("{\"key\":\"k\",\"open\":[\"mailto:a@x.example\"]}", "k"),
        Some(Handed::opening(vec!["mailto:a@x.example".to_string()])),
        "before 0.39"
    );
    assert_eq!(parse_request("{\"key\":\"k\",\"open\":[],\"attach\":[\"\",\" \"]}", "k"), Some(Handed::default()), "nothing to send");
    for junk in [
        "{\"key\":\"k\",\"open\":[],\"attach\":\"/u/a.pdf\"}",
        "{\"key\":\"k\",\"open\":[],\"attach\":[1]}",
        "{\"key\":\"k\",\"attach\":[\"/u/a.pdf\"]}",
    ] {
        assert_eq!(parse_request(junk, "k"), None, "{junk:?}");
    }
}
