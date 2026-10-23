//! Rendering receipts and documents to textures, off the UI thread.
//!
//! PDFs are rasterized with `hayro` (pure Rust), images decoded with
//! `image`; Markdown and plain text are read as UTF-8 and drawn as text
//! (Markdown by `egui_commonmark`), so they follow the theme and select
//! like any text. Results are cached by blob hash: the same bytes never render
//! twice, and a hash is the content, so the cache never goes stale. Bytes
//! are read through the verifying blob store: a corrupt blob shows as an
//! error, never as a picture.
//!
//! Tiles draw a thumbnail, not the file: its first page, [`THUMB_PX`]
//! wide, rendered by workers of their own (opening a file never waits
//! behind them). The live viewer keeps each thumbnail on disk as
//! `<sha>.png` ([`thumb_dir`]) and starts every one the books hold when
//! they load ([`Receipts::prefetch`]), so a tile's picture is there
//! before the page is opened and the next launch reads small PNGs, not
//! PDFs. A file's name is its content's hash, so a thumbnail always
//! matches the file the books hold: a changed file is another hash.

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

/// A thumbnail's width in pixels: a tile's picture is 260 points wide at
/// most, twice that on a high-density screen.
pub const THUMB_PX: f32 = 560.0;

/// A thumbnail's greatest height in pixels: a tile shows only the top of
/// the first page (170 points of a 180 to 260 point width), so the rest
/// is cut, and a long receipt costs no more than a short one.
pub const THUMB_MAX_H: u32 = 420;

/// The longest side of any texture drawn: GPUs refuse larger (8192 is
/// the least a wgpu device must take), and a refused texture panics.
pub const MAX_SIDE: f32 = 8192.0;

/// A text thumbnail keeps only this much of its text: a tile shows a few
/// lines.
const THUMB_TEXT_BYTES: usize = 2048;

/// Where the live viewer keeps thumbnails: the user's cache directory
/// (`$XDG_CACHE_HOME`, else `~/.cache`), `wanderbooks/thumbs/v1-560`;
/// the version and width name how they were drawn, so a change in either
/// is a new directory, never an old picture. None without a home.
pub fn thumb_dir() -> Option<PathBuf> {
    // A relative XDG_CACHE_HOME is ignored, as the XDG spec says.
    let base = std::env::var_os("XDG_CACHE_HOME")
        .map(PathBuf::from)
        .filter(|p| p.is_absolute())
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".cache")))?;
    Some(base.join("wanderbooks").join("thumbs").join("v1-560"))
}

struct Job {
    sha: String,
    mime: String,
    root: PathBuf,
    /// Some: a thumbnail, kept in this directory when it is Some(Some).
    thumb: Option<Option<PathBuf>>,
    /// The thumbnails' generation when this was asked for.
    generation: u64,
}

struct Done {
    sha: String,
    thumb: bool,
    generation: u64,
    result: Result<Rendered, String>,
}

pub struct Receipts {
    cache: HashMap<String, State>,
    thumbs: HashMap<String, State>,
    /// Where thumbnails are kept on disk; None keeps them in memory only
    /// (tests and screenshots, unless they ask).
    pub thumb_dir: Option<PathBuf>,
    /// Bumped when the thumbnails are forgotten: a thumbnail asked for
    /// before is dropped when it comes, never shown over a newer one.
    generation: u64,
    thumb_jobs: Sender<Job>,
    /// egui_commonmark's layout cache, shared by every Markdown file.
    pub markdown: egui_commonmark::CommonMarkCache,
    jobs: Sender<Job>,
    done: Receiver<Done>,
}

impl Receipts {
    pub fn new(ctx: &Context) -> Self {
        let (done_tx, done) = channel();
        let workers = std::thread::available_parallelism().map_or(2, |n| n.get().clamp(2, 4));
        let jobs = workers_for(ctx, &done_tx, "receipt-render", workers);
        let thumb_jobs = workers_for(ctx, &done_tx, "thumb-render", 2);
        Self {
            cache: HashMap::new(),
            thumbs: HashMap::new(),
            thumb_dir: None,
            generation: 0,
            thumb_jobs,
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
                thumb: None,
                generation: 0,
            });
        }
        &self.cache[sha]
    }

    /// A blob's thumbnail (its first page, [`THUMB_PX`] wide; a text
    /// file's text), starting it if needed.
    pub fn thumb(&mut self, sha: &str, mime: &str, root: PathBuf) -> &State {
        if !self.thumbs.contains_key(sha) {
            self.thumbs.insert(sha.to_string(), State::Loading);
            let _ = self.thumb_jobs.send(Job {
                sha: sha.to_string(),
                mime: mime.to_string(),
                root,
                thumb: Some(self.thumb_dir.clone()),
                generation: self.generation,
            });
        }
        &self.thumbs[sha]
    }

    /// Start the thumbnail of every file the books hold, so each tile's
    /// picture is ready before its page is opened. Files already started
    /// cost nothing; call it whenever books load.
    pub fn prefetch(&mut self, books: &crate::books::Books) {
        let root = books.blob_root();
        for a in &books.attachments {
            self.thumb(&a.blob_sha, &a.mime_type, root.clone());
        }
    }

    /// Drop every thumbnail held in memory (their directory changed).
    pub fn forget_thumbs(&mut self) {
        self.thumbs.clear();
        self.generation += 1;
    }

    /// A drawn thumbnail's size in pixels.
    pub fn thumb_size(&self, sha: &str) -> Option<[usize; 2]> {
        match self.thumbs.get(sha)? {
            State::Ready(pages) => pages.first().map(|t| t.size()),
            _ => None,
        }
    }

    /// Turn finished renders into textures. Call once per frame.
    pub fn pump(&mut self, ctx: &Context) {
        while let Ok(done) = self.done.try_recv() {
            self.accept(ctx, done);
        }
    }

    fn accept(&mut self, ctx: &Context, done: Done) {
        if done.thumb && done.generation != self.generation {
            return;
        }
        let state = match done.result {
            Ok(Rendered::Text { text, markdown }) => State::Text { text, markdown },
            Ok(Rendered::Pages(images)) => State::Ready(
                images
                    .into_iter()
                    .enumerate()
                    .map(|(i, img)| {
                        let img = fit_side(img, ctx.input(|i| i.max_texture_side));
                        ctx.load_texture(
                            format!(
                                "{}-{}-{i}",
                                if done.thumb { "thumb" } else { "blob" },
                                done.sha
                            ),
                            img,
                            TextureOptions::LINEAR,
                        )
                    })
                    .collect(),
            ),
            Err(e) => State::Failed(e),
        };
        if done.thumb {
            self.thumbs.insert(done.sha, state);
        } else {
            self.cache.insert(done.sha, state);
        }
    }

    /// Wait until nothing is rendering (tests and screenshots).
    pub fn settle(&mut self, ctx: &Context) {
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(20);
        while self
            .cache
            .values()
            .chain(self.thumbs.values())
            .any(|s| matches!(s, State::Loading))
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

/// Workers taking jobs from one queue, sending what they render to
/// `done`; the queue's sender.
fn workers_for(ctx: &Context, done_tx: &Sender<Done>, name: &str, n: usize) -> Sender<Job> {
    let (jobs, job_rx) = channel::<Job>();
    let job_rx = std::sync::Arc::new(std::sync::Mutex::new(job_rx));
    for i in 0..n {
        let ctx = ctx.clone();
        let job_rx = job_rx.clone();
        let done_tx = done_tx.clone();
        std::thread::Builder::new()
            .name(format!("{name}-{i}"))
            .spawn(move || {
                loop {
                    let job = match job_rx.lock() {
                        Ok(rx) => rx.recv(),
                        Err(_) => return,
                    };
                    let Ok(job) = job else {
                        return;
                    };
                    // A file that crashes its renderer is that file's
                    // error, not the end of the worker (and every file
                    // queued after it).
                    let result =
                        std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                            match &job.thumb {
                                Some(dir) => render_thumb(&job, dir.as_deref()),
                                None => render(&job),
                            }
                        }))
                        .unwrap_or_else(|_| Err("the renderer crashed on this file".into()));
                    let _ = done_tx.send(Done {
                        sha: job.sha,
                        thumb: job.thumb.is_some(),
                        generation: job.generation,
                        result,
                    });
                    ctx.request_repaint();
                }
            })
            .expect("spawn renderer");
    }
    jobs
}

/// A thumbnail: the top of the first page, [`THUMB_PX`] wide (never
/// scaled up) and at most [`THUMB_MAX_H`] tall. The blob is read through
/// the verifying store every time, so a corrupt or missing file is an
/// error, never a picture; its thumbnail is then read from `dir` when it
/// is there, else drawn (a PDF's first page, an image) and kept in `dir`.
/// A text file's thumbnail is the start of its text, kept nowhere: it is
/// cheaper to read than a picture. A thumbnail that can't be kept is
/// still shown.
fn render_thumb(job: &Job, dir: Option<&std::path::Path>) -> Result<Rendered, String> {
    let bytes = BlobStore::open_read_only(&job.root)
        .get(&job.sha)
        .map_err(|e| e.to_string())?;
    if let Some(markdown) = text_kind(&job.mime)
        && !bytes.starts_with(b"%PDF")
    {
        let mut text = text_of(bytes)?;
        if text.len() > THUMB_TEXT_BYTES {
            let mut cut = THUMB_TEXT_BYTES;
            while !text.is_char_boundary(cut) {
                cut -= 1;
            }
            text.truncate(cut);
        }
        return Ok(Rendered::Text { text, markdown });
    }
    // A blob's hash names the file; anything else never names a path.
    let named = job.sha.len() == 64 && job.sha.bytes().all(|b| b.is_ascii_hexdigit());
    let kept = dir
        .filter(|_| named)
        .map(|d| d.join(format!("{}.png", job.sha)));
    if let Some(path) = &kept
        && let Ok(png) = std::fs::read(path)
        && let Ok(img) = image::load_from_memory(&png)
    {
        return Ok(Rendered::Pages(vec![to_color_image(img.to_rgba8())]));
    }
    let img = if job.mime == "application/pdf" || bytes.starts_with(b"%PDF") {
        let png = pdf_first_page_png(bytes)?;
        image::load_from_memory(&png).map_err(|e| e.to_string())?
    } else if job.mime.starts_with("image/") {
        let img = image::load_from_memory(&bytes).map_err(|e| e.to_string())?;
        if img.width() as f32 > THUMB_PX {
            let h = (img.height() as f32 * THUMB_PX / img.width() as f32).max(1.0);
            img.resize_exact(
                THUMB_PX as u32,
                h as u32,
                image::imageops::FilterType::Triangle,
            )
        } else {
            img
        }
    } else {
        return Err(format!("{} can't be previewed", job.mime));
    };
    let img = img.crop_imm(0, 0, img.width(), img.height().min(THUMB_MAX_H));
    if let Some(path) = &kept {
        let mut png = Vec::new();
        if img
            .write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png)
            .is_ok()
        {
            keep(path, &png);
        }
    }
    Ok(Rendered::Pages(vec![to_color_image(img.to_rgba8())]))
}

/// Write a thumbnail into the cache: its directory made private to the
/// user (it pictures receipts), the bytes written to a temporary name
/// and renamed into place. A failure only means it is drawn again.
fn keep(path: &std::path::Path, png: &[u8]) {
    let Some(dir) = path.parent() else {
        return;
    };
    let mut builder = std::fs::DirBuilder::new();
    builder.recursive(true);
    #[cfg(unix)]
    std::os::unix::fs::DirBuilderExt::mode(&mut builder, 0o700);
    if builder.create(dir).is_err() {
        return;
    }
    // Made before, it may be looser: the thumbnails are private all the same.
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(dir, std::fs::Permissions::from_mode(0o700));
    }
    // One name per write, across viewers (the pid) and this one's workers.
    static WRITES: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let tmp = dir.join(format!(
        ".{}.{}.{}.tmp",
        path.file_name().and_then(|n| n.to_str()).unwrap_or("thumb"),
        std::process::id(),
        WRITES.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
    ));
    if std::fs::write(&tmp, png).is_ok() && std::fs::rename(&tmp, path).is_err() {
        let _ = std::fs::remove_file(&tmp);
    }
}

/// A PDF's first page as a PNG, [`THUMB_PX`] wide.
fn pdf_first_page_png(bytes: Vec<u8>) -> Result<Vec<u8>, String> {
    use hayro::hayro_interpret::InterpreterSettings;
    use hayro::hayro_syntax::Pdf;
    use hayro::vello_cpu::color::palette::css::WHITE;
    use hayro::{PixmapSettings, RenderCache, RenderSettings, render};
    let pdf = Pdf::new(bytes).map_err(|e| format!("not a readable PDF: {e:?}"))?;
    let pages = pdf.pages();
    let page = pages.first().ok_or("the PDF has no pages")?;
    let (w, h) = page.render_dimensions();
    // THUMB_PX wide, but never past a texture's side on a long page (the
    // picture is cut to THUMB_MAX_H after), and never nothing.
    let scale = (THUMB_PX / w.max(1.0))
        .min(MAX_SIDE / h.max(1.0))
        .max(1.0 / w.max(h).max(1.0));
    let px = PixmapSettings {
        x_scale: scale,
        y_scale: scale,
        bg_color: WHITE,
    };
    let pixmap = render(
        page,
        &RenderCache::new(),
        &InterpreterSettings::default(),
        &RenderSettings::default(),
        &px,
    );
    pixmap.into_png().map_err(|e| format!("{e:?}"))
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
        // Past a texture's side, scaled down to fit (aspect kept).
        let img = if img.width().max(img.height()) as f32 > MAX_SIDE {
            img.resize(
                MAX_SIDE as u32,
                MAX_SIDE as u32,
                image::imageops::FilterType::Triangle,
            )
        } else {
            img
        };
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
    let mut out = Vec::new();
    for page in pdf.pages().iter().take(30) {
        // Twice the page's size, but never past a texture's side.
        let (w, h) = page.render_dimensions();
        let scale = 2.0f32.min(MAX_SIDE / w.max(h).max(1.0));
        let px = PixmapSettings {
            x_scale: scale,
            y_scale: scale,
            bg_color: WHITE,
        };
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

/// A picture no side of which is past `max` pixels (the device's
/// largest texture: egui panics on a larger one), scaled down with its
/// aspect kept; most pictures are already within it and pass untouched.
fn fit_side(img: ColorImage, max: usize) -> ColorImage {
    let [w, h] = img.size;
    if w.max(h) <= max || max == 0 {
        return img;
    }
    let rgba: Vec<u8> = img.pixels.iter().flat_map(|c| c.to_array()).collect();
    let Some(buf) = image::RgbaImage::from_raw(w as u32, h as u32, rgba) else {
        return img;
    };
    let scaled = image::DynamicImage::ImageRgba8(buf).resize(
        max as u32,
        max as u32,
        image::imageops::FilterType::Triangle,
    );
    let size = [scaled.width() as usize, scaled.height() as usize];
    ColorImage::from_rgba_premultiplied(size, scaled.to_rgba8().as_raw())
}

fn to_color_image(img: image::RgbaImage) -> ColorImage {
    let size = [img.width() as usize, img.height() as usize];
    ColorImage::from_rgba_unmultiplied(size, img.as_raw())
}
