//! Video frames for attachments' thumbnails ([`Services::video_frame`]): made by a tool, a
//! process of its own (`video.rs`), on a thread of their own, at most [`AT_ONCE`] together; the
//! rest wait for a turn. A frame is kept here only until it's handed to the window, which keeps
//! what it was given and doesn't ask for that file again.

use std::collections::HashMap;
use std::sync::Mutex;
use std::sync::atomic::{AtomicUsize, Ordering};

use super::*;

/// Most tools making frames at once. Each is a process (on Windows, PowerShell compiling a few
/// lines of C#), and under load the system's thumbnailer fails now and then: a message with many
/// videos started one per video, all together.
const AT_ONCE: usize = 2;

/// Where the frame being made for a file is put.
type Slot = Arc<Mutex<Option<VideoFrame>>>;

pub(crate) struct VideoFrames {
    /// Where frames come from (looked up once).
    source: Option<Option<video::Source>>,
    /// The frames asked for and not handed over yet, by file: the slot its tool fills, or
    /// `None` while it waits for its turn.
    asked: HashMap<PathBuf, Option<Slot>>,
    /// Tools making frames now.
    running: Arc<AtomicUsize>,
    /// Makes a frame (`video::frame`; a stand-in in tests).
    make: fn(&video::Source, &Path) -> Option<Vec<u8>>,
}

impl Default for VideoFrames {
    fn default() -> VideoFrames {
        VideoFrames { source: None, asked: HashMap::new(), running: Arc::default(), make: video::frame }
    }
}

/// Counts a tool's thread as done however it ends.
struct Running(Arc<AtomicUsize>);

impl Drop for Running {
    fn drop(&mut self) {
        self.0.fetch_sub(1, Ordering::AcqRel);
    }
}

impl VideoFrames {
    /// Frames from `source` (`None`: there's no way to make them here), not the one looked up.
    #[cfg(test)]
    pub(crate) fn with_source(source: Option<video::Source>) -> VideoFrames {
        VideoFrames { source: Some(source), ..VideoFrames::default() }
    }

    /// The frame of `video`: [`VideoFrame::Pending`] while it's being made or waits for its
    /// turn (asked again, it's started when a tool is free), then what was made, once.
    pub(crate) fn frame(&mut self, video: &Path) -> VideoFrame {
        let Some(source) = self.source.get_or_insert_with(video::source).clone() else { return VideoFrame::Unavailable };
        if let Some(Some(slot)) = self.asked.get(video) {
            let made = slot.lock().map(|g| g.clone()).unwrap_or(Some(VideoFrame::Unavailable));
            return match made {
                Some(frame) => {
                    // The window keeps it from here.
                    self.asked.remove(video);
                    frame
                }
                None => VideoFrame::Pending,
            };
        }
        if self.running.load(Ordering::Acquire) >= AT_ONCE {
            self.asked.insert(video.to_path_buf(), None);
            return VideoFrame::Pending;
        }
        let slot = Slot::default();
        let (theirs, path, make) = (slot.clone(), video.to_path_buf(), self.make);
        self.running.fetch_add(1, Ordering::AcqRel);
        let running = Running(self.running.clone());
        let spawned = std::thread::Builder::new().name("video-thumbnail".into()).spawn(move || {
            let _running = running;
            let png = make(&source, &path);
            if let Ok(mut g) = theirs.lock() {
                *g = Some(png.map_or(VideoFrame::Unavailable, VideoFrame::Ready));
            }
        });
        // A thread the system won't start is no frame (the count went back with the closure).
        if spawned.is_err() {
            self.asked.remove(video);
            return VideoFrame::Unavailable;
        }
        self.asked.insert(video.to_path_buf(), Some(slot));
        VideoFrame::Pending
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::AtomicBool;
    use std::time::{Duration, Instant};

    static NOW: AtomicUsize = AtomicUsize::new(0);
    static MOST: AtomicUsize = AtomicUsize::new(0);
    static GO: AtomicBool = AtomicBool::new(false);

    /// A tool that holds its frame until the test lets it go; the frame is the file's name.
    fn held(_: &video::Source, video: &Path) -> Option<Vec<u8>> {
        let n = NOW.fetch_add(1, Ordering::AcqRel) + 1;
        MOST.fetch_max(n, Ordering::AcqRel);
        let end = Instant::now() + Duration::from_secs(30);
        while !GO.load(Ordering::Acquire) && Instant::now() < end {
            std::thread::sleep(Duration::from_millis(5));
        }
        NOW.fetch_sub(1, Ordering::AcqRel);
        video.file_name().map(|n| n.to_string_lossy().as_bytes().to_vec())
    }

    /// Found in audit (#228): a message with many videos started a tool for each at once, and
    /// every frame stayed here until MailCraft closed. The window is answered at once (the
    /// tools run elsewhere), two tools run at a time, each file gets its frame, and nothing is
    /// kept once it's handed over.
    #[test]
    fn frames_are_made_a_few_at_a_time_and_not_kept() {
        let mut frames = VideoFrames { make: held, ..VideoFrames::with_source(Some(video::Source::Ffmpeg(PathBuf::from("ffmpeg")))) };
        let files: Vec<PathBuf> = (0..6).map(|i| PathBuf::from(format!("clip{i}.mp4"))).collect();
        for f in &files {
            let asked = Instant::now();
            assert_eq!(frames.frame(f), VideoFrame::Pending);
            assert!(asked.elapsed() < Duration::from_secs(5), "the window waited for the tool");
        }
        // Asked again, as the window does each frame: still only two at a time.
        for _ in 0..20 {
            for f in &files {
                assert_eq!(frames.frame(f), VideoFrame::Pending);
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        assert_eq!(MOST.load(Ordering::Acquire), AT_ONCE, "two tools at a time");
        GO.store(true, Ordering::Release);
        let mut got: HashMap<PathBuf, VideoFrame> = HashMap::new();
        let end = Instant::now() + Duration::from_secs(30);
        while got.len() < files.len() && Instant::now() < end {
            for f in &files {
                if !got.contains_key(f) {
                    match frames.frame(f) {
                        VideoFrame::Pending => {}
                        frame => {
                            got.insert(f.clone(), frame);
                        }
                    }
                }
            }
            std::thread::sleep(Duration::from_millis(5));
        }
        for f in &files {
            let name = f.to_string_lossy().as_bytes().to_vec();
            assert_eq!(got.get(f), Some(&VideoFrame::Ready(name)), "{}", f.display());
        }
        assert!(MOST.load(Ordering::Acquire) <= AT_ONCE);
        assert!(frames.asked.is_empty(), "handed over: not kept");
    }

    /// No way to make frames here: none, and nothing is started.
    #[test]
    fn no_source_no_frames() {
        let mut frames = VideoFrames::with_source(None);
        assert_eq!(frames.frame(Path::new("a.mp4")), VideoFrame::Unavailable);
        assert!(frames.asked.is_empty());
    }
}
