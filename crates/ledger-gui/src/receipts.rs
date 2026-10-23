//! Rendering receipts and documents to textures, off the UI thread.
//!
//! PDFs are rasterized with `hayro` (pure Rust), images decoded with
//! `image`; Markdown and plain text are read as UTF-8 and drawn as text
//! (Markdown by `egui_commonmark`), so they follow the theme and select
//! like any text. Results are cached by blob hash: the same bytes never render
//! twice, and a hash is the content, so the cache never goes stale. Bytes
//! are read through the verifying blob store: a corrupt blob shows as an
//! error, never as a picture.

use egui::{ColorImage, Context, TextureHandle, TextureOptions};
use ledger_store::BlobStore;
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::mpsc::{Receiver, Sender, channel};

pub enum State {
    Loading,
    Ready(Vec<TextureHandle>),
    /// A text file: its UTF-8 text, and whether it is Markdown.
    Text {
        text: String,
        markdown: bool,
    },
    Failed(String),
}

/// What a render made: pages as pictures, or a text file's text.
enum Rendered {
    Pages(Vec<ColorImage>),
    Text { text: String, markdown: bool },
}

/// The largest text file shown as text: past it the viewer says so
/// rather than lay out megabytes of text every frame.
pub const MAX_TEXT_BYTES: usize = 2 * 1024 * 1024;

struct Job {
    sha: String,
    mime: String,
    root: PathBuf,
}

struct Done {
    sha: String,
    result: Result<Rendered, String>,
}

pub struct Receipts {
    cache: HashMap<String, State>,
    /// egui_commonmark's layout cache, shared by every Markdown file.
    pub markdown: egui_commonmark::CommonMarkCache,
    jobs: Sender<Job>,
    done: Receiver<Done>,
}

impl Receipts {
    pub fn new(ctx: &Context) -> Self {
        let (jobs, job_rx) = channel::<Job>();
        let (done_tx, done) = channel();
        let job_rx = std::sync::Arc::new(std::sync::Mutex::new(job_rx));
        let workers = std::thread::available_parallelism().map_or(2, |n| n.get().clamp(2, 4));
        for i in 0..workers {
            let ctx = ctx.clone();
            let job_rx = job_rx.clone();
            let done_tx = done_tx.clone();
            std::thread::Builder::new()
                .name(format!("receipt-render-{i}"))
                .spawn(move || {
                    loop {
                        let job = match job_rx.lock() {
                            Ok(rx) => rx.recv(),
                            Err(_) => return,
                        };
                        let Ok(job) = job else {
                            return;
                        };
                        let result = render(&job);
                        let _ = done_tx.send(Done {
                            sha: job.sha,
                            result,
                        });
                        ctx.request_repaint();
                    }
                })
                .expect("spawn renderer");
        }
        Self {
            cache: HashMap::new(),
            markdown: egui_commonmark::CommonMarkCache::default(),
            jobs,
            done,
        }
    }

    /// The rendering of a blob, starting it if needed.
    pub fn get(&mut self, sha: &str, mime: &str, root: PathBuf) -> &State {
        if !self.cache.contains_key(sha) {
            self.cache.insert(sha.to_string(), State::Loading);
            let _ = self.jobs.send(Job {
                sha: sha.to_string(),
                mime: mime.to_string(),
                root,
            });
        }
        &self.cache[sha]
    }

    /// Turn finished renders into textures. Call once per frame.
    pub fn pump(&mut self, ctx: &Context) {
        while let Ok(done) = self.done.try_recv() {
            self.accept(ctx, done);
        }
    }

    fn accept(&mut self, ctx: &Context, done: Done) {
        let state = match done.result {
            Ok(Rendered::Text { text, markdown }) => State::Text { text, markdown },
            Ok(Rendered::Pages(images)) => State::Ready(
                images
                    .into_iter()
                    .enumerate()
                    .map(|(i, img)| {
                        ctx.load_texture(
                            format!("blob-{}-{i}", done.sha),
                            img,
                            TextureOptions::LINEAR,
                        )
                    })
                    .collect(),
            ),
            Err(e) => State::Failed(e),
        };
        self.cache.insert(done.sha, state);
    }

    /// Wait until nothing is rendering (tests and screenshots).
    pub fn settle(&mut self, ctx: &Context) {
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(20);
        while self.cache.values().any(|s| matches!(s, State::Loading))
            && std::time::Instant::now() < deadline
        {
            if let Ok(done) = self
                .done
                .recv_timeout(std::time::Duration::from_millis(100))
            {
                self.accept(ctx, done);
            }
        }
    }
}

fn render(job: &Job) -> Result<Rendered, String> {
    let bytes = BlobStore::open_read_only(&job.root)
        .get(&job.sha)
        .map_err(|e| e.to_string())?;
    let is_pdf = job.mime == "application/pdf" || bytes.starts_with(b"%PDF");
    if is_pdf {
        render_pdf(bytes).map(Rendered::Pages)
    } else if job.mime.starts_with("image/") {
        let img = image::load_from_memory(&bytes).map_err(|e| e.to_string())?;
        Ok(Rendered::Pages(vec![to_color_image(img.to_rgba8())]))
    } else if let Some(markdown) = text_kind(&job.mime) {
        let text = text_of(bytes)?;
        Ok(Rendered::Text { text, markdown })
    } else {
        Err(format!("{} can't be previewed", job.mime))
    }
}

/// Whether a media type is text the viewer shows as text: `Some(true)`
/// for Markdown (`text/markdown`, RFC 7763, and the older
/// `text/x-markdown`), `Some(false)` for other `text/*` types, `None`
/// otherwise. Parameters (`; charset=utf-8`) and case are ignored.
pub fn text_kind(mime: &str) -> Option<bool> {
    let essence = mime
        .split(';')
        .next()
        .unwrap_or("")
        .trim()
        .to_ascii_lowercase();
    match essence.as_str() {
        "text/markdown" | "text/x-markdown" => Some(true),
        t if t.starts_with("text/") => Some(false),
        _ => None,
    }
}

/// A text file's text: UTF-8 (a leading byte-order mark dropped), at most
/// [`MAX_TEXT_BYTES`]; anything else is refused with why, never shown
/// half-decoded.
pub fn text_of(bytes: Vec<u8>) -> Result<String, String> {
    if bytes.len() > MAX_TEXT_BYTES {
        return Err(format!(
            "the text is {} bytes, more than the {MAX_TEXT_BYTES} shown",
            bytes.len()
        ));
    }
    let mut text = String::from_utf8(bytes).map_err(|_| "the text isn't UTF-8".to_string())?;
    if text.starts_with('\u{feff}') {
        text.drain(..'\u{feff}'.len_utf8());
    }
    Ok(text)
}

fn render_pdf(bytes: Vec<u8>) -> Result<Vec<ColorImage>, String> {
    use hayro::hayro_interpret::InterpreterSettings;
    use hayro::hayro_syntax::Pdf;
    use hayro::vello_cpu::color::palette::css::WHITE;
    use hayro::{PixmapSettings, RenderCache, RenderSettings, render};
    let pdf = Pdf::new(bytes).map_err(|e| format!("not a readable PDF: {e:?}"))?;
    let cache = RenderCache::new();
    let interp = InterpreterSettings::default();
    let settings = RenderSettings::default();
    let px = PixmapSettings {
        x_scale: 2.0,
        y_scale: 2.0,
        bg_color: WHITE,
    };
    let mut out = Vec::new();
    for page in pdf.pages().iter().take(30) {
        let pixmap = render(page, &cache, &interp, &settings, &px);
        let png = pixmap.into_png().map_err(|e| format!("{e:?}"))?;
        let img = image::load_from_memory(&png).map_err(|e| e.to_string())?;
        out.push(to_color_image(img.to_rgba8()));
    }
    if out.is_empty() {
        return Err("the PDF has no pages".into());
    }
    Ok(out)
}

fn to_color_image(img: image::RgbaImage) -> ColorImage {
    let size = [img.width() as usize, img.height() as usize];
    ColorImage::from_rgba_unmultiplied(size, img.as_raw())
}
