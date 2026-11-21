//! People's photos in their avatars' place (issue #223): the person's photo as the session knows
//! it (their contact's, or their organisation's), cropped to the circle; their initials while
//! there's none. Who was shown is noted, for the session to ask after their photos as the frame
//! ends ([`take_shown`]).

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use egui::{Color32, Pos2, Rect, Ui, pos2, vec2};
use mailcraft_engine::Session;

use super::widgets;

/// The most photos kept as pictures for drawing, and people noted in a frame.
const MAX_TEXTURES: usize = 300;
const MAX_SHOWN: usize = 200;
/// The size photos are decoded at (they're drawn at most 72 points across).
const DECODED: u32 = 160;

/// The photos drawn, as pictures, by address: the photo each was made from (to tell when it
/// changes), and the picture (`None`: it couldn't be read).
#[derive(Default)]
struct Kept {
    textures: HashMap<String, (Arc<[u8]>, Option<egui::TextureHandle>)>,
    shown: Vec<String>,
}

type Shared = Arc<Mutex<Kept>>;

fn kept(ctx: &egui::Context) -> Shared {
    ctx.data_mut(|d| d.get_temp_mut_or_default::<Shared>(egui::Id::unique("people-photos")).clone())
}

/// A person's avatar: their photo where there's one, else `initials` on `color`.
pub fn avatar(ui: &Ui, s: &Session, center: Pos2, radius: f32, email: &str, initials: &str, color: Color32) {
    let photo = (!email.trim().is_empty()).then(|| s.photo(email)).flatten();
    let shared = kept(ui.ctx());
    let texture = {
        let Ok(mut k) = shared.lock() else {
            return widgets::avatar(ui, center, radius, initials, color);
        };
        if k.shown.len() < MAX_SHOWN && !email.trim().is_empty() && !k.shown.iter().any(|e| e.eq_ignore_ascii_case(email)) {
            k.shown.push(email.trim().to_string());
        }
        match photo {
            None => None,
            Some(photo) => {
                let key = email.trim().to_lowercase();
                let fresh = k.textures.get(&key).is_some_and(|(made_of, _)| Arc::ptr_eq(made_of, &photo));
                if !fresh {
                    if k.textures.len() >= MAX_TEXTURES {
                        k.textures.clear();
                    }
                    let limit = ui.ctx().input(|i| i.max_texture_side).try_into().unwrap_or(u32::MAX);
                    let tex = crate::reading::decode_pixels(&photo, DECODED, limit).map(|(image, filter)| {
                        ui.ctx().load_texture(
                            format!("photo:{key}"),
                            image,
                            egui::TextureOptions { magnification: egui::TextureFilter::Linear, ..filter },
                        )
                    });
                    k.textures.insert(key.clone(), (photo, tex));
                }
                k.textures.get(&key).and_then(|(_, t)| t.clone())
            }
        }
    };
    match texture {
        Some(tex) => {
            let rect = Rect::from_center_size(center, vec2(radius * 2.0, radius * 2.0));
            // The middle of a photo that isn't square, as much as fills the circle.
            let [w, h] = tex.size().map(|v| v.max(1) as f32);
            let uv = if w > h {
                let cut = (1.0 - h / w) / 2.0;
                Rect::from_min_max(pos2(cut, 0.0), pos2(1.0 - cut, 1.0))
            } else {
                let cut = (1.0 - w / h) / 2.0;
                Rect::from_min_max(pos2(0.0, cut), pos2(1.0, 1.0 - cut))
            };
            egui::Image::new((tex.id(), rect.size())).uv(uv).corner_radius(radius).paint_at(ui, rect);
        }
        None => widgets::avatar(ui, center, radius, initials, color),
    }
}

/// Who was shown since this was last asked: the session asks after their photos.
pub fn take_shown(ctx: &egui::Context) -> Vec<String> {
    kept(ctx).lock().map(|mut k| std::mem::take(&mut k.shown)).unwrap_or_default()
}
