//! Video attachments' thumbnails: their files written and their frames decoded on the pictures
//! lane, not in a frame, and no more of them kept than so many.

use super::*;

/// Services whose platform gives every video a frame.
struct Frames;

impl crate::Services for Frames {
    stub_services!();
    fn video_frame(&mut self, _: &std::path::Path) -> crate::VideoFrame {
        crate::VideoFrame::Ready(png(32, 18))
    }
}

/// Services whose platform makes each video's frame of what its file holds: as many pixels
/// wide as 8 plus its first byte.
struct OwnFrames;

impl crate::Services for OwnFrames {
    stub_services!();
    fn video_frame(&mut self, path: &std::path::Path) -> crate::VideoFrame {
        match std::fs::read(path).ok().and_then(|d| d.first().copied()) {
            Some(b) => crate::VideoFrame::Ready(png(8 + u32::from(b), 8)),
            None => crate::VideoFrame::Unavailable,
        }
    }
}

/// The widths of the video thumbnails shown of `key`, by attachment, once they're all made.
fn thumb_widths(ctx: &egui::Context, a: &mut MailApp, key: &MessageKey, videos: usize) -> Vec<usize> {
    a.run("list.open", json!({"key": key.to_id()}));
    let shown = |a: &MailApp| a.ui.video_thumbs.iter().filter(|((k, _), t)| k == key && t.is_some()).count() == videos;
    pump(ctx, a, WINDOW, 20, shown);
    let mut widths: Vec<(usize, usize)> =
        a.ui.video_thumbs.iter().filter(|((k, _), _)| k == key).filter_map(|((_, i), t)| Some((*i, t.as_ref()?.size()[0]))).collect();
    widths.sort_unstable();
    widths.into_iter().map(|(_, w)| w).collect()
}

/// Two videos of the same name each have their own file for the platform's decoder, and so
/// their own thumbnail: two in one message (both were written to one file, and both chips
/// showed the frame of whichever was written last), in two messages, and the same message's
/// in two data folders (a portable copy beside the installed one: the scratch folder is the
/// user's, not the data folder's).
#[test]
fn videos_of_one_name_each_have_their_own_thumbnail() {
    let ctx = egui::Context::default();
    let (mut a, _) = app("video-names");
    a.services = Box::new(OwnFrames);
    frame(&ctx, &mut a, vec![]);
    let both = with_files(&mut a, 9040, vec![("clip.mp4".into(), "video/mp4", vec![1u8; 64]), ("clip.mp4".into(), "video/mp4", vec![5u8; 64])]);
    assert_eq!(thumb_widths(&ctx, &mut a, &both, 2), vec![9, 13], "each its own frame");
    let files: Vec<&std::path::PathBuf> = a.ui.video_files.iter().filter(|((k, _), _)| *k == both).map(|(_, p)| p).collect();
    assert_eq!(files.len(), 2);
    assert_ne!(files[0], files[1], "each its own file");
    // Two messages, each with its clip.
    let first = with_files(&mut a, 9041, vec![("clip.mp4".into(), "video/mp4", vec![2u8; 64])]);
    let second = with_files(&mut a, 9042, vec![("clip.mp4".into(), "video/mp4", vec![3u8; 64])]);
    assert_eq!(thumb_widths(&ctx, &mut a, &first, 1), vec![10]);
    assert_eq!(thumb_widths(&ctx, &mut a, &second, 1), vec![11]);
    // The same message (account, folder, uid) in another data folder, with another clip.
    let (mut b, _) = app("video-names-other");
    b.services = Box::new(OwnFrames);
    frame(&ctx, &mut b, vec![]);
    let there = with_files(&mut b, 9041, vec![("clip.mp4".into(), "video/mp4", vec![4u8; 64])]);
    assert_eq!(there, first, "one key in both");
    let ctx_b = egui::Context::default();
    assert_eq!(thumb_widths(&ctx_b, &mut b, &there, 1), vec![12]);
    let file = |x: &MailApp| x.ui.video_files.iter().find(|((k, _), _)| *k == first).map(|(_, p)| p.clone());
    assert!(file(&a).is_some());
    assert_ne!(file(&b), file(&a), "each data folder's own file");
}

/// More videos in a message than video thumbnails are kept: those past them show their kind's
/// icon, and no frame makes a texture once the others show (one past them was decoded again
/// every frame).
#[test]
fn videos_past_those_kept_make_no_texture_each_frame() {
    let (mut a, _) = app("many-videos");
    let ctx = egui::Context::default();
    a.services = Box::new(Frames);
    frame(&ctx, &mut a, vec![]);
    let files = (0..70).map(|i| (format!("clip {i}.mp4"), "video/mp4", vec![i as u8; 64])).collect();
    let k = with_files(&mut a, 9018, files);
    a.run("list.open", json!({"key": k.to_id()}));
    let input = || RawInput { screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, WINDOW)), ..Default::default() };
    for _ in 0..10 {
        ctx.run_ui(input(), |ui| a.ui(ui)).textures_delta.clear();
    }
    assert_eq!(a.ui.video_thumbs.len(), 64);
    for _ in 0..5 {
        let out = ctx.run_ui(input(), |ui| a.ui(ui));
        assert!(out.textures_delta.set.is_empty(), "{} textures made", out.textures_delta.set.len());
    }
}

/// Services that keep the pictures lane's work for the test to run when it says (the reading
/// lane's is done at once), and whose platform gives every video a frame.
struct HeldFrames(std::sync::Arc<std::sync::Mutex<Vec<crate::Work>>>);

impl crate::Services for HeldFrames {
    stub_services!();
    fn video_frame(&mut self, _: &std::path::Path) -> crate::VideoFrame {
        crate::VideoFrame::Ready(png(32, 18))
    }
    fn background(&mut self, lane: crate::Lane, work: crate::Work) -> Result<(), crate::Work> {
        if lane != crate::Lane::Pictures {
            return Err(work);
        }
        self.0.lock().unwrap().push(work);
        Ok(())
    }
    fn has_background(&self) -> bool {
        true
    }
}

/// A video's chip has its file written out and the platform's frame decoded on the pictures
/// lane: the frames that show the message read and write nothing for it (on a real file,
/// reading a message to write its 1.7 MB video held its first frame for 12 ms), and the
/// thumbnail shows once that work is done. Work for a message left does nothing.
#[test]
fn a_video_is_written_and_decoded_off_the_window() {
    let (mut a, _) = app("video-held");
    let ctx = egui::Context::default();
    let held = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
    a.services = Box::new(HeldFrames(held.clone()));
    frame(&ctx, &mut a, vec![]);
    let left = with_files(&mut a, 9031, vec![("left.mp4".into(), "video/mp4", vec![3u8; 64])]);
    let k = with_files(&mut a, 9030, vec![("clip.mp4".into(), "video/mp4", vec![7u8; 64])]);
    let take = || std::mem::take(&mut *held.lock().unwrap());
    // Opened and left before its work ran: cancelled, it writes nothing.
    a.run("list.open", json!({"key": left.to_id()}));
    pump(&ctx, &mut a, WINDOW, 2, |_| false);
    a.run("list.open", json!({"key": k.to_id()}));
    pump(&ctx, &mut a, WINDOW, 3, |_| false);
    assert!(a.ui.video_files.is_empty(), "no file written in a frame");
    let work = take();
    assert_eq!(work.len(), 2, "the file of each, asked once");
    assert!(work[0].cancelled.load(std::sync::atomic::Ordering::Relaxed), "the one left is cancelled");
    for w in work {
        (w.run)();
    }
    pump(&ctx, &mut a, WINDOW, 2, |_| false);
    assert_eq!(a.ui.video_files.len(), 1, "the file of the one shown, and only it");
    assert!(a.ui.video_files.keys().all(|(m, _)| *m == k));
    // The platform's frame came: decoded on the lane too, not in a frame.
    pump(&ctx, &mut a, WINDOW, 2, |_| false);
    assert!(a.ui.video_thumbs.is_empty(), "nothing decoded in a frame");
    let work = take();
    assert_eq!(work.len(), 1, "the frame's decoding, asked once");
    for w in work {
        (w.run)();
    }
    pump(&ctx, &mut a, WINDOW, 2, |_| false);
    assert!(a.ui.video_thumbs.iter().all(|(_, t)| t.is_some()) && a.ui.video_thumbs.len() == 1, "its thumbnail shows");
    assert!(take().is_empty(), "nothing asked again");
}

/// Found in audit (#228): the platform's frame of every video looked at was kept for the
/// session. A frame made into its thumbnail goes; of those its message left before they came,
/// a few are kept, those asked for least recently going first, never one asked for in this
/// frame.
#[test]
fn video_frames_are_not_kept_for_the_session() {
    let ctx = egui::Context::default();
    let (mut a, _) = app("video-frames-kept");
    a.services = Box::new(OwnFrames);
    frame(&ctx, &mut a, vec![]);
    let k = with_files(&mut a, 9050, vec![("one.mp4".into(), "video/mp4", vec![1u8; 64]), ("two.mp4".into(), "video/mp4", vec![2u8; 64])]);
    assert_eq!(thumb_widths(&ctx, &mut a, &k, 2), vec![9, 10]);
    pump(&ctx, &mut a, WINDOW, 3, |_| false);
    assert!(a.ui.video_frames.values().all(|(f, _)| !matches!(f, crate::VideoFrame::Ready(_))), "made into thumbnails: not kept");
    assert_eq!(thumb_widths(&ctx, &mut a, &k, 2), vec![9, 10], "the thumbnails stay");
    // Frames that came for messages left: no more than so many.
    let now = a.ui.pictures.frame();
    for i in 0..40u64 {
        a.ui.video_frames.insert(std::path::PathBuf::from(format!("left-{i}.mp4")), (crate::VideoFrame::Ready(png(8, 8)), i % now.max(1)));
    }
    a.ui.video_frames.insert("shown.mp4".into(), (crate::VideoFrame::Ready(png(8, 8)), now));
    a.ui.video_frames.insert("none.mp4".into(), (crate::VideoFrame::Unavailable, 0));
    crate::reading::bound_video_frames(&mut a.ui);
    let ready: Vec<&std::path::PathBuf> =
        a.ui.video_frames.iter().filter(|(_, (f, _))| matches!(f, crate::VideoFrame::Ready(_))).map(|(p, _)| p).collect();
    assert_eq!(ready.len(), 16, "{ready:?}");
    assert!(a.ui.video_frames.contains_key(std::path::Path::new("shown.mp4")), "asked for in this frame: kept");
    assert!(a.ui.video_frames.contains_key(std::path::Path::new("none.mp4")), "one that can't be made stays noted");
}
