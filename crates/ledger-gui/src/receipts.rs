//! Rendering receipts and documents to textures, off the UI thread.
//!
//! PDFs are rasterized with `hayro` (pure Rust), images decoded with
//! `image`. Results are cached by blob hash: the same bytes never render
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
    Failed(String),
}

struct Job {
    sha: String,
    mime: String,
    root: PathBuf,
}

struct Done {
    sha: String,
    result: Result<Vec<ColorImage>, String>,
}

pub struct Receipts {
    cache: HashMap<String, State>,
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
            Ok(images) => State::Ready(
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

fn render(job: &Job) -> Result<Vec<ColorImage>, String> {
    let bytes = BlobStore::open_read_only(&job.root)
        .get(&job.sha)
        .map_err(|e| e.to_string())?;
    let is_pdf = job.mime == "application/pdf" || bytes.starts_with(b"%PDF");
    if is_pdf {
        render_pdf(bytes)
    } else if job.mime.starts_with("image/") {
        let img = image::load_from_memory(&bytes).map_err(|e| e.to_string())?;
        Ok(vec![to_color_image(img.to_rgba8())])
    } else {
        Err(format!("{} can't be previewed", job.mime))
    }
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
