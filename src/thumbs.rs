//! Video thumbnails for the preview panel. One background thread downloads
//! YouTube's small JPEGs (320×180, about 15 KB); kitty draws the selected one
//! with its graphics protocol, scaled to the cells the panel leaves free.

use std::collections::{HashMap, HashSet, VecDeque};
use std::io::{Read, Write};
use std::sync::mpsc;
use std::time::Duration;

use base64::Engine;
use ratatui::layout::Rect;

/// Our image in kitty, apart from the ones mpv draws.
const IMAGE_ID: u32 = 0x7475_6265;
/// Decoded thumbnails kept in memory (about 170 KB each).
const CACHE: usize = 64;
/// kitty takes the image in base64 chunks of at most this many bytes.
const CHUNK: usize = 4096;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Thumb {
    pub width: u32,
    pub height: u32,
    /// 8-bit RGB, row by row.
    pub rgb: Vec<u8>,
}

pub struct Thumbnails {
    cache: HashMap<String, Thumb>,
    order: VecDeque<String>,
    requested: HashSet<String>,
    jobs: mpsc::Sender<String>,
    done: mpsc::Receiver<(String, Option<Thumb>)>,
    /// What is on screen now.
    shown: Option<(String, Rect)>,
}

impl Default for Thumbnails {
    fn default() -> Self {
        let (jobs, queue) = mpsc::channel::<String>();
        let (finished, done) = mpsc::channel();
        std::thread::spawn(move || {
            for id in queue {
                if finished.send((id.clone(), download(&id).ok())).is_err() {
                    return;
                }
            }
        });
        Self { cache: HashMap::new(), order: VecDeque::new(), requested: HashSet::new(), jobs, done, shown: None }
    }
}

impl Thumbnails {
    /// Downloads these thumbnails, in order, unless they are here or on the way.
    pub fn want<'a>(&mut self, ids: impl IntoIterator<Item = &'a str>) {
        for id in ids {
            if !self.cache.contains_key(id) && self.requested.insert(id.to_owned()) {
                let _ = self.jobs.send(id.to_owned());
            }
        }
    }

    /// Takes the finished downloads; whether any arrived.
    pub fn receive(&mut self) -> bool {
        let mut any = false;
        while let Ok((id, thumb)) = self.done.try_recv() {
            self.requested.remove(&id);
            if let Some(thumb) = thumb {
                self.order.push_back(id.clone());
                self.cache.insert(id, thumb);
                any = true;
            }
        }
        while self.order.len() > CACHE {
            if let Some(old) = self.order.pop_front() {
                self.cache.remove(&old);
            }
        }
        any
    }

    /// Shows the thumbnail of `target` (video id, cells), or none. Writes to
    /// the terminal only when that changes.
    pub fn show(&mut self, target: Option<(String, Rect)>) {
        let target = target.filter(|(id, _)| self.cache.contains_key(id));
        if target == self.shown {
            return;
        }
        let mut out = delete();
        if let Some((id, area)) = &target {
            out.push_str(&place(&self.cache[id], *area));
        }
        let mut stdout = std::io::stdout().lock();
        let _ = stdout.write_all(out.as_bytes()).and_then(|()| stdout.flush());
        self.shown = target;
    }

    /// Something removed every image from the screen: draw it again next time.
    pub fn forget(&mut self) {
        self.shown = None;
    }
}

fn download(id: &str) -> Result<Thumb, String> {
    let url = format!("https://i.ytimg.com/vi/{id}/mqdefault.jpg");
    let agent: ureq::Agent = ureq::Agent::config_builder().timeout_global(Some(Duration::from_secs(10))).build().into();
    let mut jpeg = Vec::new();
    let reply = agent.get(&url).call().map_err(|e| e.to_string())?;
    reply.into_body().into_reader().take(1 << 20).read_to_end(&mut jpeg).map_err(|e| e.to_string())?;
    decode(&jpeg)
}

fn decode(jpeg: &[u8]) -> Result<Thumb, String> {
    let image = image::load_from_memory_with_format(jpeg, image::ImageFormat::Jpeg).map_err(|e| e.to_string())?;
    let rgb = image.to_rgb8();
    Ok(Thumb { width: rgb.width(), height: rgb.height(), rgb: rgb.into_raw() })
}

/// Removes our image (and frees it in kitty).
fn delete() -> String {
    format!("\x1b_Ga=d,d=I,i={IMAGE_ID},q=2\x1b\\")
}

/// Sends `thumb` and places it scaled to `area`, leaving the cursor where it was.
fn place(thumb: &Thumb, area: Rect) -> String {
    let data = base64::engine::general_purpose::STANDARD.encode(&thumb.rgb);
    let chunks: Vec<&[u8]> = data.as_bytes().chunks(CHUNK).collect();
    let mut out = format!("\x1b7\x1b[{};{}H", area.y + 1, area.x + 1);
    for (i, chunk) in chunks.iter().enumerate() {
        let more = u8::from(i + 1 < chunks.len());
        let chunk = std::str::from_utf8(chunk).unwrap_or_default();
        if i == 0 {
            out.push_str(&format!(
                "\x1b_Ga=T,f=24,s={},v={},i={IMAGE_ID},c={},r={},C=1,q=2,m={more};{chunk}\x1b\\",
                thumb.width, thumb.height, area.width, area.height
            ));
        } else {
            out.push_str(&format!("\x1b_Gm={more};{chunk}\x1b\\"));
        }
    }
    out.push_str("\x1b8");
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn thumb(width: u32, height: u32) -> Thumb {
        Thumb { width, height, rgb: vec![7; (width * height * 3) as usize] }
    }

    #[test]
    fn placing_sends_chunked_rgb_scaled_to_the_cells() {
        let out = place(&thumb(64, 36), Rect::new(10, 4, 40, 11));
        assert!(out.starts_with("\x1b7\x1b[5;11H\x1b_Ga=T,f=24,s=64,v=36,i="), "{out:.60}");
        assert!(out.contains(",c=40,r=11,C=1,q=2,m=1;"), "first chunk says more follow");
        assert!(out.contains("\x1b_Gm=0;"), "last chunk ends it");
        assert!(out.ends_with("\x1b\\\x1b8"), "cursor restored");
        let chunks = out.matches("\x1b_G").count();
        assert_eq!(chunks, (64 * 36 * 3_usize).div_ceil(3) * 4 / CHUNK + 1);
    }

    #[test]
    fn small_images_go_in_one_chunk() {
        let out = place(&thumb(2, 2), Rect::new(0, 0, 4, 2));
        assert!(out.contains(",m=0;"));
        assert_eq!(out.matches("\x1b_G").count(), 1);
    }

    #[test]
    fn only_downloaded_thumbnails_are_shown() {
        let mut thumbs = Thumbnails::default();
        thumbs.show(Some(("missing".into(), Rect::new(0, 0, 4, 2))));
        assert_eq!(thumbs.shown, None);
        thumbs.cache.insert("a".into(), thumb(2, 2));
        thumbs.show(Some(("a".into(), Rect::new(0, 0, 4, 2))));
        assert_eq!(thumbs.shown, Some(("a".into(), Rect::new(0, 0, 4, 2))));
        thumbs.forget();
        assert_eq!(thumbs.shown, None);
    }

    #[test]
    fn decodes_jpeg() {
        let mut jpeg = Vec::new();
        let image = image::RgbImage::from_pixel(8, 4, image::Rgb([200, 10, 10]));
        image::codecs::jpeg::JpegEncoder::new(&mut jpeg).encode_image(&image).ok();
        if jpeg.is_empty() {
            return; // encoder not built with this feature set
        }
        let thumb = decode(&jpeg).unwrap();
        assert_eq!((thumb.width, thumb.height, thumb.rgb.len()), (8, 4, 96));
    }
}
