//! A POP3 server on localhost for tests: one maildrop, `USER`/`PASS`, `UIDL`, `RETR`, `DELE`.

use std::collections::HashSet;
use std::io::{BufRead, BufReader, Write};
use std::net::TcpListener;
use std::sync::{Arc, Mutex};

/// Asked with each `RETR`'s message number before it's answered: `false` drops the connection
/// there (a download stopped part-way).
pub type OnRetr = Box<dyn FnMut(usize) -> bool + Send>;

pub struct Server {
    pub port: u16,
    /// (unique id, message); deleted ones go at `QUIT`.
    pub drop: Arc<Mutex<Vec<(String, Vec<u8>)>>>,
    pub on_retr: Arc<Mutex<Option<OnRetr>>>,
}

pub fn serve(messages: Vec<(String, Vec<u8>)>) -> Server {
    let l = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = l.local_addr().unwrap().port();
    let drop = Arc::new(Mutex::new(messages));
    let on_retr: Arc<Mutex<Option<OnRetr>>> = Arc::default();
    let (shared, hook) = (drop.clone(), on_retr.clone());
    std::thread::spawn(move || {
        for conn in l.incoming() {
            let Ok(mut s) = conn else { continue };
            let (shared, hook) = (shared.clone(), hook.clone());
            std::thread::spawn(move || {
                let mut r = BufReader::new(s.try_clone().unwrap());
                let _ = s.write_all(b"+OK test POP3 ready\r\n");
                let (mut user, mut authed, mut deleted) = (String::new(), false, HashSet::<usize>::new());
                loop {
                    let mut line = String::new();
                    if r.read_line(&mut line).unwrap_or(0) == 0 {
                        return;
                    }
                    let line = line.trim_end().to_string();
                    let (cmd, arg) = line.split_once(' ').unwrap_or((line.as_str(), ""));
                    let cmd = cmd.to_ascii_uppercase();
                    if authed && cmd == "RETR" {
                        let n = arg.parse::<usize>().unwrap_or(0);
                        if let Some(f) = hook.lock().unwrap().as_mut()
                            && !f(n)
                        {
                            return;
                        }
                    }
                    let mut d = shared.lock().unwrap();
                    let out = match cmd.as_str() {
                        "USER" => {
                            user = arg.to_string();
                            "+OK\r\n".to_string()
                        }
                        "PASS" if user == "me@pop.example" && arg == "pw" => {
                            authed = true;
                            "+OK signed in\r\n".to_string()
                        }
                        "PASS" => "-ERR [AUTH] wrong password\r\n".to_string(),
                        _ if !authed => "-ERR sign in first\r\n".to_string(),
                        "UIDL" => {
                            let mut o = "+OK\r\n".to_string();
                            for (i, (id, _)) in d.iter().enumerate().filter(|(i, _)| !deleted.contains(i)) {
                                o.push_str(&format!("{} {id}\r\n", i + 1));
                            }
                            o + ".\r\n"
                        }
                        "RETR" => match arg.parse::<usize>().ok().and_then(|n| d.get(n.wrapping_sub(1))) {
                            Some((_, m)) => {
                                let body: String = String::from_utf8_lossy(m)
                                    .split("\r\n")
                                    .map(|l| if l.starts_with('.') { format!(".{l}\r\n") } else { format!("{l}\r\n") })
                                    .collect();
                                format!("+OK\r\n{body}.\r\n")
                            }
                            None => "-ERR no such message\r\n".to_string(),
                        },
                        "DELE" => {
                            if let Ok(n) = arg.parse::<usize>() {
                                deleted.insert(n - 1);
                            }
                            "+OK\r\n".to_string()
                        }
                        "QUIT" => {
                            let mut i = 0;
                            d.retain(|_| {
                                i += 1;
                                !deleted.contains(&(i - 1))
                            });
                            let _ = s.write_all(b"+OK bye\r\n");
                            return;
                        }
                        _ => "-ERR unknown\r\n".to_string(),
                    };
                    std::mem::drop(d);
                    let _ = s.write_all(out.as_bytes());
                }
            });
        }
    });
    Server { port, drop, on_retr }
}
