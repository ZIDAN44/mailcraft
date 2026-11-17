use super::*;

impl AndroidServices {
    pub(crate) fn video_frame_impl(&mut self, path: &std::path::Path) -> mailcraft_ui_egui::VideoFrame {
        use mailcraft_ui_egui::{Lane, VideoFrame, Work};

        if let Some(slot) = self.video_frames.get(path) {
            return slot.lock().map(|frame| frame.clone().unwrap_or(VideoFrame::Pending)).unwrap_or(VideoFrame::Unavailable);
        }
        if self.video_frames.len() >= 64 {
            self.video_frames.retain(|_, slot| slot.lock().is_ok_and(|frame| frame.is_none()));
            if self.video_frames.len() >= 64 {
                return VideoFrame::Unavailable;
            }
        }
        let slot = std::sync::Arc::new(Mutex::new(None));
        self.video_frames.insert(path.to_path_buf(), slot.clone());
        let video = path.to_path_buf();
        let activity = self.activity.clone();
        let repaint = self.repaint.clone();
        let run = Box::new(move || {
            let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| platform::video_frame(&activity, &video))).unwrap_or_else(|_| {
                log::warn!("an Android video thumbnail couldn't be made (the decoder failed)");
                VideoFrame::Unavailable
            });
            if let Ok(mut frame) = slot.lock() {
                *frame = Some(result);
            }
            repaint.request_repaint();
        });
        let work = Work { run, cancelled: std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false)), first: false };
        if self.lanes.submit(Lane::Videos, work).is_err() {
            self.video_frames.remove(path);
            return VideoFrame::Unavailable;
        }
        VideoFrame::Pending
    }
}
