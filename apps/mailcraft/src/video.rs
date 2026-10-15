//! Video thumbnails: a frame from ffmpeg when it's installed, otherwise from the system's own
//! thumbnailer (the Windows Shell, macOS Quick Look, or a Linux video thumbnailer), which decodes
//! with the codecs the system has. Every tool runs as a separate process, stopped after a while.

use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

/// How long a tool gets for one frame.
const TIMEOUT: Duration = Duration::from_secs(20);
/// The frame's width (or largest side).
const SIZE: u32 = 480;

/// Where frames come from.
#[derive(Clone, Debug)]
pub(crate) enum Source {
    Ffmpeg(PathBuf),
    /// The Windows Shell's thumbnails, through PowerShell.
    Shell(PathBuf),
    /// Quick Look's `qlmanage`.
    QuickLook(PathBuf),
    /// `ffmpegthumbnailer` or `totem-video-thumbnailer`.
    Thumbnailer(PathBuf),
}

/// An executable on the PATH, or in a usual place for it.
pub(crate) fn find_tool(name: &str, usual: &[&str]) -> Option<PathBuf> {
    let exe = if cfg!(windows) { format!("{name}.exe") } else { name.to_string() };
    let on_path = std::env::var_os("PATH").into_iter().flat_map(|p| std::env::split_paths(&p).collect::<Vec<_>>());
    on_path.chain(usual.iter().map(PathBuf::from)).map(|d| d.join(&exe)).find(|p| p.is_file())
}

/// The best way to get frames here, if any.
pub(crate) fn source() -> Option<Source> {
    if let Some(f) = find_tool("ffmpeg", &["/opt/homebrew/bin", "/usr/local/bin", "C:\\ffmpeg\\bin", "C:\\Program Files\\ffmpeg\\bin"]) {
        return Some(Source::Ffmpeg(f));
    }
    system()
}

/// The system's own thumbnailer.
pub(crate) fn system() -> Option<Source> {
    if cfg!(windows) {
        let root = std::env::var("SystemRoot").unwrap_or_else(|_| "C:\\Windows".into());
        let dir = format!("{root}\\System32\\WindowsPowerShell\\v1.0");
        find_tool("powershell", &[dir.as_str()]).map(Source::Shell)
    } else if cfg!(target_os = "macos") {
        find_tool("qlmanage", &["/usr/bin"]).map(Source::QuickLook)
    } else {
        find_tool("ffmpegthumbnailer", &["/usr/bin"]).or_else(|| find_tool("totem-video-thumbnailer", &["/usr/bin"])).map(Source::Thumbnailer)
    }
}

/// A frame of `video` as PNG.
pub(crate) fn frame(source: &Source, video: &Path) -> Option<Vec<u8>> {
    match source {
        // A second in shows more than the first frame (often black); short clips have none there.
        Source::Ffmpeg(f) => ffmpeg(f, video, "1").or_else(|| ffmpeg(f, video, "0")),
        Source::Shell(ps) => {
            let (script, out) = (scratch("ps1"), scratch("png"));
            std::fs::write(&script, SHELL_SCRIPT).ok()?;
            let mut cmd = Command::new(ps);
            cmd.args(["-NoProfile", "-NonInteractive", "-ExecutionPolicy", "Bypass", "-File"])
                .arg(&script)
                .env("MAILCRAFT_VIDEO", video)
                .env("MAILCRAFT_PNG", &out)
                .env("MAILCRAFT_SIZE", SIZE.to_string());
            let png = from_file(run(cmd), &out);
            let _ = std::fs::remove_file(&script);
            png
        }
        Source::QuickLook(ql) => {
            let dir = scratch("d");
            std::fs::create_dir_all(&dir).ok()?;
            let mut cmd = Command::new(ql);
            cmd.args(["-t", "-s", &SIZE.to_string(), "-o"]).arg(&dir).arg(video);
            let name = format!("{}.png", video.file_name()?.to_string_lossy());
            let png = from_file(run(cmd), &dir.join(name));
            let _ = std::fs::remove_dir_all(&dir);
            png
        }
        Source::Thumbnailer(t) => {
            let out = scratch("png");
            let mut cmd = Command::new(t);
            if t.file_name().is_some_and(|n| n.to_string_lossy().starts_with("totem")) {
                cmd.args(["-s", &SIZE.to_string()]).arg(video).arg(&out);
            } else {
                cmd.arg("-i").arg(video).arg("-o").arg(&out).args(["-s", &SIZE.to_string(), "-c", "png"]);
            }
            from_file(run(cmd), &out)
        }
    }
}

/// The Windows Shell's thumbnail of a file (`IShellItemImageFactory`, thumbnails only, never the
/// file type's icon), saved as PNG. Runs inside PowerShell, so MailCraft itself stays safe Rust.
const SHELL_SCRIPT: &str = r#"$ErrorActionPreference = 'Stop'
Add-Type -ReferencedAssemblies System.Drawing -TypeDefinition @'
using System;
using System.Drawing;
using System.Drawing.Imaging;
using System.Runtime.InteropServices;
[ComImport, Guid("bcc18b79-ba16-442f-80c4-8a59c30c463b"), InterfaceType(ComInterfaceType.InterfaceIsIUnknown)]
interface IShellItemImageFactory { [PreserveSig] int GetImage(Size size, int flags, out IntPtr bitmap); }
public static class MailCraftThumbnail {
  [DllImport("shell32.dll", CharSet = CharSet.Unicode, PreserveSig = false)]
  static extern void SHCreateItemFromParsingName(string path, IntPtr bc, [MarshalAs(UnmanagedType.LPStruct)] Guid iid, [MarshalAs(UnmanagedType.Interface)] out IShellItemImageFactory item);
  [DllImport("gdi32.dll")] static extern bool DeleteObject(IntPtr h);
  public static void Save(string path, string png, int px) {
    IShellItemImageFactory f;
    SHCreateItemFromParsingName(path, IntPtr.Zero, typeof(IShellItemImageFactory).GUID, out f);
    IntPtr h;
    int hr = f.GetImage(new Size(px, px), 0x8, out h);
    if (hr != 0) throw new Exception("no thumbnail");
    try { using (var b = Image.FromHbitmap(h)) b.Save(png, ImageFormat.Png); } finally { DeleteObject(h); }
  }
}
'@
[MailCraftThumbnail]::Save($env:MAILCRAFT_VIDEO, $env:MAILCRAFT_PNG, [int]$env:MAILCRAFT_SIZE)
"#;

/// A file name of our own in the temp folder.
fn scratch(ext: &str) -> PathBuf {
    static N: AtomicU64 = AtomicU64::new(0);
    std::env::temp_dir().join(format!("mailcraft-frame-{}-{}.{ext}", std::process::id(), N.fetch_add(1, Ordering::Relaxed)))
}

/// The PNG a tool wrote (removed afterwards), when it finished well.
fn from_file(ok: Option<Vec<u8>>, out: &Path) -> Option<Vec<u8>> {
    let png = ok.and_then(|_| std::fs::read(out).ok()).filter(|b| b.starts_with(b"\x89PNG"));
    let _ = std::fs::remove_file(out);
    png
}

/// One frame, `at` seconds in, from ffmpeg.
fn ffmpeg(ffmpeg: &Path, video: &Path, at: &str) -> Option<Vec<u8>> {
    let mut cmd = Command::new(ffmpeg);
    cmd.args(["-nostdin", "-loglevel", "error", "-ss", at, "-i"]).arg(video).args([
        "-frames:v",
        "1",
        "-vf",
        &format!("scale={SIZE}:-2"),
        "-f",
        "image2pipe",
        "-vcodec",
        "png",
        "-",
    ]);
    run(cmd).filter(|b| b.starts_with(b"\x89PNG"))
}

/// Run a tool: what it printed, if it ended well in time.
fn run(mut cmd: Command) -> Option<Vec<u8>> {
    cmd.stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::null());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        // No console window flashing up.
        cmd.creation_flags(0x0800_0000);
    }
    let mut child = cmd.spawn().ok()?;
    let mut out = child.stdout.take()?;
    let reader = std::thread::spawn(move || {
        let mut buf = Vec::new();
        let _ = out.by_ref().take(20 << 20).read_to_end(&mut buf);
        buf
    });
    let end = Instant::now() + TIMEOUT;
    let status = loop {
        match child.try_wait() {
            Ok(Some(s)) => break s,
            Ok(None) if Instant::now() < end => std::thread::sleep(Duration::from_millis(50)),
            _ => {
                let _ = child.kill();
                let _ = child.wait();
                return None;
            }
        }
    };
    let printed = reader.join().ok()?;
    status.success().then_some(printed)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A short test clip, made with ffmpeg (when it's installed).
    fn clip(dir: &Path) -> Option<PathBuf> {
        let ffmpeg = find_tool("ffmpeg", &[])?;
        let clip = dir.join("clip.mp4");
        let made = Command::new(&ffmpeg)
            .args(["-y", "-loglevel", "error", "-f", "lavfi", "-i", "testsrc=duration=2:size=320x240:rate=10", "-pix_fmt", "yuv420p"])
            .arg(&clip)
            .status();
        made.is_ok_and(|s| s.success()).then_some(clip)
    }

    #[test]
    fn frames_come_from_ffmpeg_or_the_system() {
        let dir = std::env::temp_dir().join(format!("mailcraft-video-{}", std::process::id()));
        let _ = std::fs::create_dir_all(&dir);
        let Some(clip) = clip(&dir) else { return };
        let junk = dir.join("junk.mp4");
        std::fs::write(&junk, b"not a video").unwrap();
        let sources = [find_tool("ffmpeg", &[]).map(Source::Ffmpeg), system()];
        for s in sources.iter().flatten() {
            let png = frame(s, &clip).unwrap_or_else(|| panic!("a frame from {s:?}"));
            let img = image::load_from_memory(&png).unwrap();
            assert!(img.width() >= 200, "{s:?}: {}", img.width());
            // Not a video: no frame, and no hang.
            assert!(frame(s, &junk).is_none(), "{s:?}");
        }
        let _ = std::fs::remove_dir_all(dir);
    }
}
