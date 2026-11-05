// Desktop archiver: start page, archive browser, extract / new / add / delete / test.
// No console window on Windows release builds.
#![cfg_attr(all(windows, not(debug_assertions)), windows_subsystem = "windows")]
mod archive;
mod i18n;

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::sync::atomic::Ordering;
use std::sync::mpsc;

use archive::{Entry, Error, Kind, NewOpts, Res, PROGRESS};
use eframe::egui::{self, Align2, Color32, FontId, Pos2, Rect, Sense, Stroke, Vec2};
use i18n::{tr, trf, Lang};

/// OS-level app id; the visible title is translated (tr("tailzip")).
const APP_NAME: &str = "tailzip";
/// Accent colour: fox-tail orange (tailzip brand).
const ACCENT: Color32 = Color32::from_rgb(0xD9, 0x5F, 0x1E);
const START_TOP: Color32 = Color32::from_rgb(0xF4, 0xA8, 0x62);
const START_BOTTOM: Color32 = Color32::from_rgb(0xD9, 0x5F, 0x1E);
const HELP_BAR: Color32 = Color32::from_rgb(0x5C, 0x2E, 0x14);
/// Compression level names; index = level 0..3.
fn level_name(level: u8) -> &'static str {
    [
        tr("0 - No compression"),
        tr("1 - Fast compression"),
        tr("2 - Normal compression"),
        tr("3 - Maximum compression"),
    ][level as usize]
}
/// Default level: Normal = deflate 6.
const DEFAULT_LEVEL: u8 = 2;
/// Column widths: Name, Compressed, Original, Type, Modified, Method, CRC.
const COL_W: [f32; 7] = [300.0, 100.0, 100.0, 150.0, 150.0, 90.0, 80.0];
fn col_names() -> [&'static str; 7] {
    [
        tr("Name"),
        tr("Compressed"),
        tr("Original"),
        tr("Type"),
        tr("Modified"),
        tr("Method"),
        tr("CRC"),
    ]
}
const ROW_H: f32 = 22.0;
const SHOT_FRAMES: u32 = 120;

fn main() -> eframe::Result {
    i18n::init();
    // Probing every graphics backend at once crashed on a test machine; use one native backend per OS.
    if std::env::var_os("WGPU_BACKEND").is_none() {
        let native = if cfg!(target_os = "macos") {
            "metal"
        } else if cfg!(windows) {
            "dx12"
        } else {
            "vulkan"
        };
        std::env::set_var("WGPU_BACKEND", native);
    }
    let mut args = std::env::args().skip(1);
    let mut shot = None;
    let mut open = None;
    while let Some(a) = args.next() {
        if a == "--screenshot" {
            shot = args.next().map(PathBuf::from);
        } else {
            open = Some(PathBuf::from(a));
        }
    }
    let opts = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title(tr("tailzip"))
            .with_inner_size([930.0, 660.0])
            .with_min_inner_size([520.0, 360.0])
            .with_drag_and_drop(true),
        ..Default::default()
    };
    // macOS takes the icon from the app bundle and masks it; elsewhere set the window icon.
    #[cfg(not(target_os = "macos"))]
    let opts = {
        let mut opts = opts;
        if let Ok(icon) =
            eframe::icon_data::from_png_bytes(include_bytes!("../assets/icon_256.png"))
        {
            opts.viewport = opts.viewport.with_icon(std::sync::Arc::new(icon));
        }
        opts
    };
    eframe::run_native(
        APP_NAME,
        opts,
        Box::new(move |cc| {
            setup_style(&cc.egui_ctx);
            let mut app = App {
                shot,
                ..App::default()
            };
            if let Some(p) = open {
                app.open(p);
            }
            Ok(Box::new(app))
        }),
    )
}

fn setup_style(ctx: &egui::Context) {
    setup_fonts(ctx);
    let mut vis = egui::Visuals::light();
    vis.selection.bg_fill = Color32::from_rgb(0xFB, 0xD3, 0xB4);
    vis.selection.stroke = Stroke::new(1.0, ACCENT);
    vis.hyperlink_color = ACCENT;
    ctx.set_visuals(vis);
}

/// Add a system font with Korean/Chinese/Japanese glyphs as a fallback so such file names render.
fn setup_fonts(ctx: &egui::Context) {
    let candidates = [
        "C:\\Windows\\Fonts\\malgun.ttf",
        "C:\\Windows\\Fonts\\msyh.ttc",
        "/System/Library/Fonts/AppleSDGothicNeo.ttc",
        "/System/Library/Fonts/Supplemental/Arial Unicode.ttf",
        "/System/Library/Fonts/Hiragino Sans GB.ttc",
    ];
    let Some(bytes) = candidates.iter().find_map(|p| std::fs::read(p).ok()) else {
        return;
    };
    let mut fonts = egui::FontDefinitions::default();
    fonts
        .font_data
        .insert("cjk".into(), egui::FontData::from_owned(bytes).into());
    for fam in [egui::FontFamily::Proportional, egui::FontFamily::Monospace] {
        fonts.families.entry(fam).or_default().push("cjk".into());
    }
    ctx.set_fonts(fonts);
}

struct Opened {
    path: PathBuf,
    kind: Kind,
    entries: Vec<Entry>,
    /// Kept in memory only while this archive is open; never saved or logged.
    password: Option<String>,
    /// Bytes on disk, all volumes (RAR does not report packed sizes per file).
    disk_size: u64,
}

struct NewDlg {
    out: String,
    kind: Kind,
    level: u8,
    inputs: Vec<PathBuf>,
    /// true = "Add" to the open archive instead of creating a new one.
    add_to_open: bool,
    use_pw: bool,
    pw: String,
    pw2: String,
    show_pw: bool,
    zip_crypto: bool,
    encrypt_names: bool,
    split: bool,
    /// Index into VOLUME_PRESETS.
    preset: usize,
    custom_mb: u32,
}

/// Volume sizes in MiB; 0 = custom.
const VOLUME_PRESETS: [(&str, u64); 5] = [
    ("10 MB", 10),
    ("100 MB", 100),
    ("700 MB (CD)", 700),
    ("4 GB (FAT32)", 4095),
    ("Custom", 0),
];

/// Retried with the password the user typed.
type Retry = Box<dyn FnOnce(&mut App, String)>;
/// Runs on the UI thread with a finished job's result.
type Then = Box<dyn FnOnce(&mut App, Res<usize>)>;

/// A long operation on a worker thread; `then` runs on the UI thread with its result.
struct Job {
    title: String,
    rx: mpsc::Receiver<Res<usize>>,
    then: Then,
}

enum Dialog {
    None,
    Msg(String, String),
    New(NewDlg),
    Extract {
        dest: String,
        only: Option<Vec<String>>,
    },
    ConfirmDelete(Vec<String>),
    Password {
        input: String,
        show: bool,
        wrong: bool,
        retry: Option<Retry>,
    },
    Settings,
    About,
}

/// Deferred dialog result, run after the dialog closes.
type Action = Box<dyn FnOnce(&mut App)>;

struct App {
    opened: Option<Opened>,
    cwd: String,
    selected: BTreeSet<usize>,
    anchor: Option<usize>,
    dialog: Dialog,
    recent: Vec<PathBuf>,
    default_kind: Kind,
    default_level: u8,
    filter: String,
    show_find: bool,
    show_toolbar: bool,
    show_status: bool,
    frame_no: u32,
    shot: Option<PathBuf>,
    job: Option<Job>,
}

impl Default for App {
    fn default() -> Self {
        Self {
            opened: None,
            cwd: String::new(),
            selected: BTreeSet::new(),
            anchor: None,
            dialog: Dialog::None,
            recent: Vec::new(),
            default_kind: Kind::Zip,
            default_level: DEFAULT_LEVEL,
            filter: String::new(),
            show_find: false,
            show_toolbar: true,
            show_status: true,
            frame_no: 0,
            shot: None,
            job: None,
        }
    }
}

fn human(n: u64) -> String {
    let units = ["B", "KB", "MB", "GB", "TB"];
    let mut v = n as f64;
    let mut i = 0;
    while v >= 1024.0 && i < units.len() - 1 {
        v /= 1024.0;
        i += 1;
    }
    if i == 0 {
        format!("{n} B")
    } else {
        format!("{v:.1} {}", units[i])
    }
}

fn type_name(e: &Entry) -> String {
    if e.is_dir {
        return tr("File folder").into();
    }
    match e.name().rsplit_once('.') {
        Some((_, ext)) if !ext.is_empty() => trf("{ext} File", &[("ext", &ext.to_uppercase())]),
        _ => tr("File").into(),
    }
}

/// Open a file or folder with the system's default app.
fn open_with_system(p: &Path) {
    #[cfg(target_os = "windows")]
    let r = std::process::Command::new("explorer").arg(p).spawn();
    #[cfg(target_os = "macos")]
    let r = std::process::Command::new("open").arg(p).spawn();
    #[cfg(not(any(target_os = "windows", target_os = "macos")))]
    let r = std::process::Command::new("xdg-open").arg(p).spawn();
    let _ = r;
}

impl App {
    fn msg(&mut self, title: &str, text: impl Into<String>) {
        self.dialog = Dialog::Msg(title.into(), text.into());
    }

    fn open(&mut self, path: PathBuf) {
        self.open_pw(path, None);
    }

    fn open_pw(&mut self, path: PathBuf, password: Option<String>) {
        match archive::list(&path, password.as_deref()) {
            Ok((kind, entries)) => {
                self.recent.retain(|p| p != &path);
                self.recent.insert(0, path.clone());
                self.recent.truncate(10);
                let disk_size = archive::volumes(&path)
                    .unwrap_or_default()
                    .iter()
                    .filter_map(|v| std::fs::metadata(v).ok())
                    .map(|m| m.len())
                    .sum();
                self.opened = Some(Opened {
                    path,
                    kind,
                    entries,
                    password,
                    disk_size,
                });
                self.cwd.clear();
                self.selected.clear();
                self.anchor = None;
                self.filter.clear();
            }
            Err(e) => {
                let p = path.clone();
                if !self.ask_password(&e, move |app, pw| app.open_pw(p, Some(pw))) {
                    self.msg(
                        tr("Open Archive"),
                        trf(
                            "Cannot open {path}:\n{e}",
                            &[("path", &path.display()), ("e", &e)],
                        ),
                    );
                }
            }
        }
    }

    /// Show the password prompt for password errors; `retry` runs with the typed password.
    fn ask_password(&mut self, e: &Error, retry: impl FnOnce(&mut App, String) + 'static) -> bool {
        let wrong = match e {
            Error::NeedPassword => false,
            Error::WrongPassword => true,
            _ => return false,
        };
        self.dialog = Dialog::Password {
            input: String::new(),
            show: false,
            wrong,
            retry: Some(Box::new(retry)),
        };
        true
    }

    fn set_password(&mut self, pw: String) {
        if let Some(o) = &mut self.opened {
            o.password = Some(pw);
        }
    }

    /// Path and password of the open archive.
    fn target(&self) -> Option<(PathBuf, Option<String>)> {
        self.opened
            .as_ref()
            .map(|o| (o.path.clone(), o.password.clone()))
    }

    /// Why Add/Delete are unavailable for the open archive, if they are.
    fn edit_block(&self) -> Option<&'static str> {
        self.opened
            .as_ref()
            .and_then(|o| archive::edit_block(&o.path, o.kind))
    }

    /// Run `work` on a worker thread with a progress window; `then` gets the result.
    fn run_job(
        &mut self,
        title: &str,
        total: usize,
        work: impl FnOnce() -> Res<usize> + Send + 'static,
        then: impl FnOnce(&mut App, Res<usize>) + 'static,
    ) {
        if self.job.is_some() {
            return;
        }
        PROGRESS.reset(total);
        let (tx, rx) = mpsc::channel();
        std::thread::spawn(move || {
            let _ = tx.send(work());
        });
        self.job = Some(Job {
            title: title.into(),
            rx,
            then: Box::new(then),
        });
    }

    fn poll_job(&mut self, ctx: &egui::Context) {
        let Some(job) = &self.job else { return };
        let r = match job.rx.try_recv() {
            Ok(r) => r,
            // The worker died (a library panicked): report instead of crashing.
            Err(mpsc::TryRecvError::Disconnected) => {
                Err(Error::Msg(tr("The operation failed unexpectedly.").into()))
            }
            Err(mpsc::TryRecvError::Empty) => {
                let (done, total) = (
                    PROGRESS.done.load(Ordering::Relaxed),
                    PROGRESS.total.load(Ordering::Relaxed),
                );
                egui::Modal::new(egui::Id::new("job")).show(ctx, |ui| {
                    ui.set_min_width(360.0);
                    ui.heading(job.title.as_str());
                    ui.add_space(6.0);
                    let frac = if total > 0 {
                        done.min(total) as f32 / total as f32
                    } else {
                        0.0
                    };
                    ui.add(
                        egui::ProgressBar::new(frac)
                            .text(trf(
                                "{done} / {total} file(s)",
                                &[("done", &done), ("total", &total)],
                            ))
                            .animate(true),
                    );
                    ui.add_space(8.0);
                    if ui.button(format!("   {}   ", tr("Cancel"))).clicked() {
                        PROGRESS.cancel.store(true, Ordering::Relaxed);
                    }
                });
                ctx.request_repaint_after(std::time::Duration::from_millis(100));
                return;
            }
        };
        if let Some(job) = self.job.take() {
            (job.then)(self, r);
        }
    }

    fn reload(&mut self) {
        let Some(o) = &self.opened else { return };
        let (path, cwd, pw) = (o.path.clone(), self.cwd.clone(), o.password.clone());
        self.open_pw(path, pw);
        if let Some(o) = &self.opened {
            if o.entries.iter().any(|e| e.is_dir && e.path == cwd) {
                self.cwd = cwd;
            }
        }
    }

    fn close(&mut self) {
        self.opened = None;
        self.selected.clear();
        self.cwd.clear();
    }

    fn pick_open(&mut self) {
        if let Some(p) = rfd::FileDialog::new()
            .add_filter(
                tr("Archives"),
                &[
                    "zip", "zipx", "7z", "rar", "alz", "egg", "tar", "tgz", "gz", "001", "z01",
                    "a00", "r00",
                ],
            )
            .add_filter(tr("All files"), &["*"])
            .pick_file()
        {
            self.open(p);
        }
    }

    fn start_new(&mut self, inputs: Vec<PathBuf>) {
        let out = inputs
            .first()
            .map(|p| {
                p.with_extension("")
                    .with_extension(self.default_kind.ext())
                    .display()
                    .to_string()
            })
            .unwrap_or_default();
        self.dialog = Dialog::New(NewDlg {
            out,
            kind: self.default_kind,
            level: self.default_level,
            inputs,
            add_to_open: false,
            use_pw: false,
            pw: String::new(),
            pw2: String::new(),
            show_pw: false,
            zip_crypto: false,
            encrypt_names: false,
            split: false,
            preset: 0,
            custom_mb: 50,
        });
    }

    fn start_add(&mut self, inputs: Vec<PathBuf>) {
        if self.opened.is_none() {
            return;
        }
        if let Some(why) = self.edit_block() {
            self.msg(tr("Add"), why);
            return;
        }
        self.start_new(inputs);
        if let Dialog::New(d) = &mut self.dialog {
            d.out.clear();
            d.kind = Kind::Zip;
            d.add_to_open = true;
        }
    }

    fn start_delete(&mut self) {
        if let Some(why) = self.edit_block() {
            self.msg(tr("Delete"), why);
        } else if !self.selected.is_empty() {
            self.dialog = Dialog::ConfirmDelete(self.selected_paths());
        }
    }

    fn start_extract(&mut self, only: Option<Vec<String>>) {
        let Some(o) = &self.opened else { return };
        // Default destination: a folder named after the archive, next to it.
        let dest = o
            .path
            .parent()
            .unwrap_or(Path::new("."))
            .join(archive::base_name(&o.path));
        self.dialog = Dialog::Extract {
            dest: dest.display().to_string(),
            only,
        };
    }

    fn selected_paths(&self) -> Vec<String> {
        let Some(o) = &self.opened else { return vec![] };
        self.selected
            .iter()
            .filter_map(|&i| o.entries.get(i))
            .map(|e| e.path.clone())
            .collect()
    }

    /// Files under the given archive paths (all files when None), for the progress bar.
    fn file_count(&self, only: Option<&[String]>) -> usize {
        let Some(o) = &self.opened else { return 0 };
        o.entries
            .iter()
            .filter(|e| !e.is_dir)
            .filter(|e| {
                only.is_none_or(|sel| {
                    sel.iter()
                        .any(|s| e.path == *s || e.path.starts_with(&format!("{s}/")))
                })
            })
            .count()
    }

    fn test_archive(&mut self) {
        let Some((path, pw)) = self.target() else {
            return;
        };
        let total = self.file_count(None);
        self.run_job(
            tr("Test"),
            total,
            move || archive::test(&path, pw.as_deref()),
            |app, r| match r {
                Ok(n) => app.msg(
                    tr("Test"),
                    trf(
                        "No errors found in this archive.\n{n} file(s) tested.",
                        &[("n", &n)],
                    ),
                ),
                Err(e) => {
                    if !app.ask_password(&e, |app, pw| {
                        app.set_password(pw);
                        app.test_archive();
                    }) {
                        app.msg(
                            tr("Test"),
                            trf("Errors were found in this archive:\n{e}", &[("e", &e)]),
                        );
                    }
                }
            },
        );
    }

    fn do_extract(&mut self, dest: PathBuf, only: Option<Vec<String>>) {
        let Some((path, pw)) = self.target() else {
            return;
        };
        let total = self.file_count(only.as_deref());
        let (d, o) = (dest.clone(), only.clone());
        self.run_job(
            tr("Extract"),
            total,
            move || archive::extract(&path, &d, o.as_deref(), pw.as_deref()),
            move |app, r| match r {
                Ok(n) => {
                    app.msg(
                        tr("Extract"),
                        trf(
                            "Extraction completed successfully.\n{n} file(s) extracted.",
                            &[("n", &n)],
                        ),
                    );
                    open_with_system(&dest);
                }
                Err(e) => {
                    if !app.ask_password(&e, move |app, pw| {
                        app.set_password(pw);
                        app.do_extract(dest, only);
                    }) {
                        app.msg(
                            tr("Extract"),
                            trf("Extraction has failed.\n({e})", &[("e", &e)]),
                        );
                    }
                }
            },
        );
    }

    fn do_delete(&mut self, sel: Vec<String>) {
        let Some((path, pw)) = self.target() else {
            return;
        };
        self.run_job(
            tr("Delete"),
            0,
            {
                let sel = sel.clone();
                move || archive::delete(&path, &sel, pw.as_deref()).map(|_| 0)
            },
            |app, r| match r {
                Ok(_) => app.reload(),
                Err(e) => {
                    if !app.ask_password(&e, |app, pw| {
                        app.set_password(pw);
                        app.do_delete(sel);
                    }) {
                        app.msg(tr("Delete"), e);
                    }
                }
            },
        );
    }

    fn do_add(&mut self, inputs: Vec<PathBuf>, level: u8) {
        let Some((path, pw)) = self.target() else {
            return;
        };
        let cwd = self.cwd.clone();
        self.run_job(
            tr("Add"),
            0,
            {
                let inputs = inputs.clone();
                move || archive::add(&path, &inputs, &cwd, level, pw.as_deref()).map(|_| 0)
            },
            move |app, r| match r {
                Ok(_) => app.reload(),
                Err(e) => {
                    if !app.ask_password(&e, move |app, pw| {
                        app.set_password(pw);
                        app.do_add(inputs, level);
                    }) {
                        app.msg(tr("Add"), e);
                    }
                }
            },
        );
    }

    fn do_create(
        &mut self,
        out: PathBuf,
        kind: Kind,
        inputs: Vec<PathBuf>,
        level: u8,
        opts: NewOpts,
    ) {
        let split = opts.volume_size.is_some();
        let o = out.clone();
        self.run_job(
            tr("New Archive"),
            0,
            move || archive::create(&o, kind, &inputs, level, &opts),
            move |app, r| match r {
                Ok(_) => {
                    app.open(if split {
                        archive::volume_path(&out, 0)
                    } else {
                        out
                    });
                    app.msg(tr("New Archive"), tr("Archiving completed successfully."));
                }
                Err(e) => app.msg(tr("New Archive"), e),
            },
        );
    }

    /// Rows shown in the list: children of the current folder, or all matches while searching.
    fn rows(&self) -> Vec<usize> {
        let Some(o) = &self.opened else { return vec![] };
        let f = self.filter.to_lowercase();
        let mut v: Vec<usize> = (0..o.entries.len())
            .filter(|&i| {
                let e = &o.entries[i];
                if f.is_empty() {
                    e.parent() == self.cwd
                } else {
                    !e.is_dir && e.name().to_lowercase().contains(&f)
                }
            })
            .collect();
        v.sort_by(|&a, &b| {
            let (a, b) = (&o.entries[a], &o.entries[b]);
            b.is_dir
                .cmp(&a.is_dir)
                .then_with(|| a.name().to_lowercase().cmp(&b.name().to_lowercase()))
        });
        v
    }

    /// Double-click: enter a folder, or extract a file to a temp folder and open it.
    fn activate(&mut self, idx: usize) {
        let Some(o) = &self.opened else { return };
        let Some(e) = o.entries.get(idx) else { return };
        if e.is_dir {
            self.cwd = e.path.clone();
            self.selected.clear();
            self.filter.clear();
            return;
        }
        let tmp = std::env::temp_dir().join(format!("archiver-{}", std::process::id()));
        let (path, inner, pw) = (o.path.clone(), e.path.clone(), o.password.clone());
        match archive::extract(
            &path,
            &tmp,
            Some(std::slice::from_ref(&inner)),
            pw.as_deref(),
        ) {
            Ok(_) => open_with_system(&tmp.join(&inner)),
            Err(e) => {
                if !self.ask_password(&e, move |app, pw| {
                    app.set_password(pw);
                    app.activate(idx);
                }) {
                    self.msg(tr("Open"), e);
                }
            }
        }
    }

    fn go_up(&mut self) {
        self.cwd = self
            .cwd
            .rsplit_once('/')
            .map(|(a, _)| a.to_string())
            .unwrap_or_default();
        self.selected.clear();
    }

    // ---------------- UI ----------------

    fn menu_bar(&mut self, ui: &mut egui::Ui) {
        egui::MenuBar::new().ui(ui, |ui| {
            let has = self.opened.is_some();
            let item = |label: &str, key: &str| egui::Button::new(label).shortcut_text(key);
            ui.menu_button(tr("File"), |ui| {
                if ui.add(item(tr("Open Archive..."), "Ctrl+O")).clicked() {
                    self.pick_open();
                }
                if ui.add(item(tr("New Archive..."), "Ctrl+N")).clicked() {
                    self.start_new(vec![]);
                }
                ui.menu_button(tr("Recent Files"), |ui| {
                    if self.recent.is_empty() {
                        ui.label(tr("(none)"));
                    }
                    for p in self.recent.clone() {
                        if ui.button(p.display().to_string()).clicked() {
                            self.open(p);
                        }
                    }
                });
                if ui
                    .add_enabled(has, egui::Button::new(tr("Close Archive")))
                    .clicked()
                {
                    self.close();
                }
                ui.separator();
                if ui.add(item(tr("Exit"), "Ctrl+W")).clicked() {
                    ui.ctx().send_viewport_cmd(egui::ViewportCommand::Close);
                }
            });
            let block = self.edit_block();
            let why = block.unwrap_or_default();
            ui.menu_button(tr("Edit"), |ui| {
                if ui
                    .add_enabled(
                        has && block.is_none(),
                        egui::Button::new(tr("Add Files...")),
                    )
                    .on_disabled_hover_text(why)
                    .clicked()
                {
                    self.pick_add();
                }
                if ui
                    .add_enabled(
                        has && block.is_none() && !self.selected.is_empty(),
                        item(tr("Delete Files"), "Del"),
                    )
                    .on_disabled_hover_text(why)
                    .clicked()
                {
                    self.start_delete();
                }
                if ui
                    .add_enabled(has, item(tr("Select All"), "Ctrl+A"))
                    .clicked()
                {
                    self.selected = self.rows().into_iter().collect();
                }
            });
            ui.menu_button(tr("Find"), |ui| {
                if ui
                    .add_enabled(has, item(tr("Find Files..."), "Ctrl+F"))
                    .clicked()
                {
                    self.show_find = true;
                }
            });
            ui.menu_button(tr("Options"), |ui| {
                if ui.button(tr("Settings...")).clicked() {
                    self.dialog = Dialog::Settings;
                }
            });
            ui.menu_button(tr("View"), |ui| {
                ui.checkbox(&mut self.show_toolbar, tr("Toolbar"));
                ui.checkbox(&mut self.show_status, tr("Status Bar"));
                ui.separator();
                ui.menu_button(tr("Language"), |ui| {
                    for l in Lang::ALL {
                        if ui.radio(i18n::lang() == l, l.name()).clicked() {
                            i18n::choose(l);
                        }
                    }
                });
            });
            ui.menu_button(tr("Tools"), |ui| {
                if ui
                    .add_enabled(has, egui::Button::new(tr("Test Archive")))
                    .clicked()
                {
                    self.test_archive();
                }
                if ui
                    .add_enabled(has, egui::Button::new(tr("Extract All...")))
                    .clicked()
                {
                    self.start_extract(None);
                }
            });
            ui.menu_button(tr("Help"), |ui| {
                if ui.add(item(tr("About..."), "F1")).clicked() {
                    self.dialog = Dialog::About;
                }
            });
        });
    }

    fn pick_add(&mut self) {
        if let Some(files) = rfd::FileDialog::new().pick_files() {
            self.start_add(files);
        }
    }

    fn toolbar(&mut self, ui: &mut egui::Ui) {
        // Toolbar buttons in order: Open, Extract, New, Add, Delete, Test.
        let sel = !self.selected.is_empty();
        let block = self.edit_block();
        let buttons: [(&str, &str, bool); 6] = [
            ("🗁", tr("Open"), true),
            ("📤", tr("Extract"), true),
            ("🗋", tr("New"), true),
            ("➕", tr("Add"), block.is_none()),
            ("🗑", tr("Delete"), sel && block.is_none()),
            ("✔", tr("Test"), true),
        ];
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 2.0;
            for (i, (icon, label, enabled)) in buttons.into_iter().enumerate() {
                // Wider than 64 px when the label needs it (Korean labels are wider).
                let text_w = ui
                    .painter()
                    .layout_no_wrap(
                        label.to_string(),
                        FontId::proportional(13.0),
                        Color32::BLACK,
                    )
                    .size()
                    .x;
                let (rect, resp) = ui.allocate_exact_size(
                    Vec2::new((text_w + 14.0).max(64.0), 56.0),
                    Sense::click(),
                );
                // Disabled Add/Delete explain why (read-only format or split archive).
                let resp = match block {
                    Some(why) if !enabled && (i == 3 || i == 4) => resp.on_hover_text(why),
                    _ => resp.on_hover_text(label),
                };
                let col = if enabled {
                    Color32::from_rgb(0x30, 0x30, 0x30)
                } else {
                    Color32::from_gray(180)
                };
                if enabled && resp.hovered() {
                    ui.painter()
                        .rect_filled(rect, 3.0, Color32::from_rgb(0xFD, 0xEE, 0xE2));
                }
                let p = ui.painter();
                p.text(
                    rect.center_top() + Vec2::new(0.0, 18.0),
                    Align2::CENTER_CENTER,
                    icon,
                    FontId::proportional(22.0),
                    if enabled { ACCENT } else { col },
                );
                p.text(
                    rect.center_bottom() - Vec2::new(0.0, 12.0),
                    Align2::CENTER_CENTER,
                    label,
                    FontId::proportional(13.0),
                    col,
                );
                if enabled && resp.clicked() {
                    match i {
                        0 => self.pick_open(),
                        1 => self.start_extract(sel.then(|| self.selected_paths())),
                        2 => self.start_new(vec![]),
                        3 => self.pick_add(),
                        4 => self.start_delete(),
                        _ => self.test_archive(),
                    }
                }
            }
            if self.show_find {
                ui.add_space(12.0);
                ui.label("🔍");
                let r = ui.add(
                    egui::TextEdit::singleline(&mut self.filter)
                        .hint_text(tr("Find files"))
                        .desired_width(180.0),
                );
                if r.changed() {
                    self.selected.clear();
                }
                if ui.small_button("✖").clicked() {
                    self.filter.clear();
                    self.show_find = false;
                }
            }
        });
    }

    fn start_page(&mut self, ui: &mut egui::Ui) {
        let full = ui.max_rect();
        let help_h = 46.0;
        let body = Rect::from_min_max(full.min, Pos2::new(full.max.x, full.max.y - help_h));
        let p = ui.painter();
        // Diagonal gradient (lighter top-right, darker bottom-left).
        let mut mesh = egui::Mesh::default();
        let mid = Color32::from_rgb(0xE8, 0x83, 0x3E);
        for (pos, c) in [
            (body.left_top(), mid),
            (body.right_top(), START_TOP),
            (body.right_bottom(), mid),
            (body.left_bottom(), START_BOTTOM),
        ] {
            mesh.colored_vertex(pos, c);
        }
        mesh.add_triangle(0, 1, 2);
        mesh.add_triangle(0, 2, 3);
        p.add(egui::Shape::mesh(mesh));
        let help = Rect::from_min_max(Pos2::new(full.min.x, body.max.y), full.max);
        p.rect_filled(help, 0.0, HELP_BAR);

        // Two white tiles: Open Archive / New Archive.
        let tile = Vec2::new(148.0, 125.0);
        let gap = 3.0;
        let c = body.center();
        let open_r = Rect::from_min_size(
            Pos2::new(c.x - tile.x - gap / 2.0, c.y - tile.y / 2.0),
            tile,
        );
        let new_r = Rect::from_min_size(Pos2::new(c.x + gap / 2.0, c.y - tile.y / 2.0), tile);
        let open_resp = ui.interact(open_r, ui.id().with("open_tile"), Sense::click());
        let new_resp = ui.interact(new_r, ui.id().with("new_tile"), Sense::click());
        let p = ui.painter();
        for (r, resp, label, is_open) in [
            (open_r, &open_resp, tr("Open Archive"), true),
            (new_r, &new_resp, tr("New Archive"), false),
        ] {
            let fill = if resp.hovered() {
                Color32::from_rgb(0xFF, 0xF6, 0xEF)
            } else {
                Color32::WHITE
            };
            p.rect_filled(r, 0.0, fill);
            p.rect_stroke(
                r,
                0.0,
                Stroke::new(1.0, Color32::from_rgb(0xEC, 0xCF, 0xB8)),
                egui::StrokeKind::Inside,
            );
            let ic = Pos2::new(r.center().x, r.min.y + 48.0);
            draw_tile_icon(p, ic, is_open);
            p.text(
                Pos2::new(r.center().x, r.max.y - 30.0),
                Align2::CENTER_CENTER,
                label,
                FontId::proportional(15.0),
                Color32::from_gray(20),
            );
        }
        // Recent-files chevron under the Open tile.
        let chev = Rect::from_center_size(
            Pos2::new(open_r.center().x, open_r.max.y + 12.0),
            Vec2::new(24.0, 16.0),
        );
        let chev_resp = ui.interact(chev, ui.id().with("recent"), Sense::click());
        let cc = chev.center();
        ui.painter().line(
            vec![
                cc + Vec2::new(-5.0, -2.5),
                cc + Vec2::new(0.0, 2.5),
                cc + Vec2::new(5.0, -2.5),
            ],
            Stroke::new(1.3, Color32::WHITE),
        );
        egui::Popup::menu(&chev_resp).show(|ui| {
            if self.recent.is_empty() {
                ui.label(tr("No recent archives"));
            }
            for p in self.recent.clone() {
                if ui
                    .button(p.file_name().unwrap_or_default().to_string_lossy())
                    .clicked()
                {
                    self.open(p);
                }
            }
        });
        // Help bar.
        let help_resp = ui.interact(help, ui.id().with("help"), Sense::click());
        let hc = help.center();
        let p = ui.painter();
        // Center the "(i) label" pair on the bar whatever the label width.
        let label = p.layout_no_wrap(
            tr("Help Page").to_string(),
            FontId::proportional(16.0),
            Color32::WHITE,
        );
        let left = hc.x - (20.0 + 12.0 + label.size().x) / 2.0;
        let ic = Pos2::new(left + 10.0, hc.y);
        p.circle_filled(ic, 10.0, Color32::from_gray(235));
        p.text(
            ic,
            Align2::CENTER_CENTER,
            "i",
            FontId::proportional(14.0),
            HELP_BAR,
        );
        p.galley(
            Pos2::new(left + 32.0, hc.y - label.size().y / 2.0),
            label,
            Color32::WHITE,
        );

        if open_resp.clicked() {
            self.pick_open();
        }
        if new_resp.clicked() {
            self.start_new(vec![]);
        }
        if help_resp.clicked() {
            self.dialog = Dialog::About;
        }
    }

    fn folder_tree(&mut self, ui: &mut egui::Ui) {
        let Some(o) = &self.opened else { return };
        let root = o
            .path
            .file_name()
            .unwrap_or_default()
            .to_string_lossy()
            .to_string();
        let mut dirs: Vec<&str> = o
            .entries
            .iter()
            .filter(|e| e.is_dir)
            .map(|e| e.path.as_str())
            .collect();
        dirs.sort_by_key(|d| d.to_lowercase());
        let mut go = None;
        egui::ScrollArea::both().auto_shrink(false).show(ui, |ui| {
            if ui
                .selectable_label(
                    self.cwd.is_empty() && self.filter.is_empty(),
                    format!("🗄 {root}"),
                )
                .clicked()
            {
                go = Some(String::new());
            }
            for d in dirs {
                let depth = d.matches('/').count() + 1;
                let name = d.rsplit('/').next().unwrap_or(d);
                ui.horizontal(|ui| {
                    ui.add_space(depth as f32 * 14.0);
                    if ui
                        .selectable_label(
                            self.cwd == d && self.filter.is_empty(),
                            format!("🗀 {name}"),
                        )
                        .clicked()
                    {
                        go = Some(d.to_string());
                    }
                });
            }
        });
        if let Some(d) = go {
            self.cwd = d;
            self.filter.clear();
            self.selected.clear();
        }
    }

    fn file_list(&mut self, ui: &mut egui::Ui) {
        let rows = self.rows();
        let editable = self.edit_block().is_none();
        let Some(o) = &self.opened else { return };
        let text = Color32::from_gray(25);
        let font = FontId::proportional(13.0);
        let total_w: f32 = COL_W.iter().sum::<f32>().max(ui.available_width());

        // Header.
        let (hr, _) = ui.allocate_exact_size(Vec2::new(total_w, ROW_H + 2.0), Sense::hover());
        let p = ui.painter();
        p.rect_filled(hr, 0.0, Color32::from_gray(248));
        p.line_segment(
            [hr.left_bottom(), hr.right_bottom()],
            Stroke::new(1.0, Color32::from_gray(220)),
        );
        let mut x = hr.min.x;
        for (name, w) in col_names().into_iter().zip(COL_W) {
            p.text(
                Pos2::new(x + 6.0, hr.center().y),
                Align2::LEFT_CENTER,
                name,
                font.clone(),
                Color32::from_gray(80),
            );
            x += w;
            p.line_segment(
                [
                    Pos2::new(x - 1.0, hr.min.y + 4.0),
                    Pos2::new(x - 1.0, hr.max.y - 4.0),
                ],
                Stroke::new(1.0, Color32::from_gray(225)),
            );
        }

        let mut clicked: Option<(usize, egui::Modifiers)> = None;
        let mut dbl = None;
        let mut ctx_extract = false;
        let mut ctx_delete = false;
        let mut up = false;
        let show_up = !self.cwd.is_empty() && self.filter.is_empty();
        let n = rows.len() + show_up as usize;
        egui::ScrollArea::vertical()
            .auto_shrink(false)
            .show_rows(ui, ROW_H, n, |ui, range| {
                for r in range {
                    let (rect, resp) =
                        ui.allocate_exact_size(Vec2::new(total_w, ROW_H), Sense::click());
                    let p = ui.painter();
                    if show_up && r == 0 {
                        p.text(
                            Pos2::new(rect.min.x + 6.0, rect.center().y),
                            Align2::LEFT_CENTER,
                            "⮪  ..",
                            font.clone(),
                            text,
                        );
                        if resp.double_clicked() {
                            up = true;
                        }
                        continue;
                    }
                    let idx = rows[r - show_up as usize];
                    let e = &o.entries[idx];
                    let sel = self.selected.contains(&idx);
                    if sel {
                        p.rect_filled(rect, 0.0, Color32::from_rgb(0xFB, 0xDC, 0xC4));
                    } else if resp.hovered() {
                        p.rect_filled(rect, 0.0, Color32::from_rgb(0xFE, 0xF0, 0xE6));
                    }
                    let icon = if e.is_dir {
                        "🗀"
                    } else if e.encrypted {
                        "🔒"
                    } else {
                        "🗋"
                    };
                    let name = if self.filter.is_empty() {
                        e.name().to_string()
                    } else {
                        e.path.clone()
                    };
                    let cells = [
                        format!("{icon} {name}"),
                        // RAR does not report packed sizes per file.
                        if e.is_dir || (e.packed == 0 && e.size > 0) {
                            String::new()
                        } else {
                            human(e.packed)
                        },
                        if e.is_dir {
                            String::new()
                        } else {
                            human(e.size)
                        },
                        type_name(e),
                        e.modified.clone(),
                        tr(&e.method).to_string(),
                        e.crc.map(|c| format!("{c:08X}")).unwrap_or_default(),
                    ];
                    let mut x = rect.min.x;
                    for (w, s) in COL_W.iter().zip(cells) {
                        let clip = Rect::from_min_size(
                            Pos2::new(x, rect.min.y),
                            Vec2::new(w - 4.0, ROW_H),
                        );
                        p.with_clip_rect(clip).text(
                            Pos2::new(x + 6.0, rect.center().y),
                            Align2::LEFT_CENTER,
                            s,
                            font.clone(),
                            text,
                        );
                        x += w;
                    }
                    if resp.clicked() || (resp.secondary_clicked() && !sel) {
                        clicked = Some((idx, ui.input(|i| i.modifiers)));
                    }
                    if resp.double_clicked() {
                        dbl = Some(idx);
                    }
                    resp.context_menu(|ui| {
                        if ui.button(tr("Extract selected files...")).clicked() {
                            ctx_extract = true;
                        }
                        if ui
                            .add_enabled(editable, egui::Button::new(tr("Delete")))
                            .clicked()
                        {
                            ctx_delete = true;
                        }
                    });
                }
            });

        if let Some((idx, m)) = clicked {
            if m.shift {
                if let (Some(pa), Some(pb)) = (
                    rows.iter().position(|&x| Some(x) == self.anchor),
                    rows.iter().position(|&x| x == idx),
                ) {
                    let (lo, hi) = (pa.min(pb), pa.max(pb));
                    self.selected = rows[lo..=hi].iter().copied().collect();
                }
            } else if m.command {
                if !self.selected.remove(&idx) {
                    self.selected.insert(idx);
                }
                self.anchor = Some(idx);
            } else {
                self.selected = [idx].into();
                self.anchor = Some(idx);
            }
        }
        if up {
            self.go_up();
        }
        if let Some(i) = dbl {
            self.activate(i);
        }
        if ctx_extract {
            self.start_extract(Some(self.selected_paths()));
        }
        if ctx_delete {
            self.start_delete();
        }
    }

    fn status_bar(&self, ui: &mut egui::Ui) {
        let Some(o) = &self.opened else { return };
        ui.horizontal(|ui| {
            if self.selected.is_empty() {
                let files = o.entries.iter().filter(|e| !e.is_dir).count();
                let size: u64 = o.entries.iter().map(|e| e.size).sum();
                let packed: u64 = match o.entries.iter().map(|e| e.packed).sum() {
                    0 => o.disk_size.min(size),
                    p => p,
                };
                let ratio = if size > 0 {
                    100.0 - packed as f64 * 100.0 / size as f64
                } else {
                    0.0
                };
                ui.label(trf(
                    "{kind} - {files} file(s), {size}  ·  Compression ratio {ratio}%",
                    &[
                        ("kind", &o.kind.label()),
                        ("files", &files),
                        ("size", &human(size)),
                        ("ratio", &format!("{ratio:.1}")),
                    ],
                ));
            } else {
                let sel: Vec<&Entry> = self
                    .selected
                    .iter()
                    .filter_map(|&i| o.entries.get(i))
                    .collect();
                let files = sel.iter().filter(|e| !e.is_dir).count();
                let dirs = sel.len() - files;
                let size: u64 = sel.iter().map(|e| e.size).sum();
                ui.label(trf(
                    "{files} file(s), {dirs} folder(s) selected  ·  Selected file size: {size}",
                    &[("files", &files), ("dirs", &dirs), ("size", &human(size))],
                ));
            }
            if o.password.is_some() || o.entries.iter().any(|e| e.encrypted) {
                ui.separator();
                ui.label(format!("🔒 {}", tr("Encrypted")));
            }
            if archive::is_split(&o.path) {
                ui.separator();
                ui.label(tr("Split archive"));
            }
            if o.kind.read_only() {
                ui.separator();
                ui.label(tr("Read-only format"));
            }
        });
    }

    fn dialogs(&mut self, ctx: &egui::Context) {
        let mut close = false;
        let mut action: Option<Action> = None;
        match &mut self.dialog {
            Dialog::None => {}
            Dialog::Msg(title, text) => {
                egui::Modal::new(egui::Id::new("msg")).show(ctx, |ui| {
                    ui.set_min_width(320.0);
                    ui.heading(title.as_str());
                    ui.add_space(6.0);
                    ui.label(text.as_str());
                    ui.add_space(10.0);
                    if ui.button(format!("   {}   ", tr("OK"))).clicked() {
                        close = true;
                    }
                });
            }
            Dialog::About => {
                egui::Modal::new(egui::Id::new("about")).show(ctx, |ui| {
                    ui.set_min_width(340.0);
                    ui.heading(tr("tailzip"));
                    ui.label(tr("Create ZIP, 7Z, TAR and TGZ archives; open and extract RAR, ZIPX, ALZ and EGG too."));
                    ui.label(tr("Runs on Windows and macOS."));
                    ui.add_space(6.0);
                    ui.label(tr("Tip: drag files onto the window to compress them, or drop an archive to open it."));
                    ui.add_space(10.0);
                    if ui.button(format!("   {}   ", tr("OK"))).clicked() {
                        close = true;
                    }
                });
            }
            Dialog::Password {
                input,
                show,
                wrong,
                retry,
            } => {
                egui::Modal::new(egui::Id::new("password")).show(ctx, |ui| {
                    ui.set_min_width(360.0);
                    ui.heading(tr("Enter Password"));
                    ui.add_space(4.0);
                    if *wrong {
                        ui.colored_label(
                            Color32::from_rgb(0xC0, 0x30, 0x20),
                            tr("The password is incorrect. Try again."),
                        );
                    } else {
                        ui.label(tr("This archive is encrypted. Enter the password."));
                    }
                    ui.add_space(6.0);
                    let r = ui.horizontal(|ui| {
                        ui.label(tr("Password:"));
                        ui.add(
                            egui::TextEdit::singleline(input)
                                .password(!*show)
                                .desired_width(240.0),
                        )
                    });
                    let r = r.inner;
                    if input.is_empty() && !r.has_focus() {
                        r.request_focus();
                    }
                    ui.checkbox(show, tr("Show password"));
                    ui.add_space(10.0);
                    let enter = r.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
                    ui.horizontal(|ui| {
                        let ok = ui
                            .add_enabled(
                                !input.is_empty(),
                                egui::Button::new(format!("   {}   ", tr("OK"))),
                            )
                            .clicked();
                        if (ok || (enter && !input.is_empty())) && retry.is_some() {
                            let pw = std::mem::take(input);
                            if let Some(f) = retry.take() {
                                action = Some(Box::new(move |app: &mut App| f(app, pw)));
                            }
                        } else if ok {
                            close = true;
                        }
                        if ui.button(format!("   {}   ", tr("Cancel"))).clicked() {
                            close = true;
                        }
                    });
                });
            }
            Dialog::Settings => {
                let (mut kind, mut level) = (self.default_kind, self.default_level);
                egui::Modal::new(egui::Id::new("settings")).show(ctx, |ui| {
                    ui.set_min_width(340.0);
                    ui.heading(tr("Settings"));
                    ui.add_space(6.0);
                    format_level_pickers(ui, &mut kind, &mut level);
                    ui.add_space(10.0);
                    if ui.button(format!("   {}   ", tr("OK"))).clicked() {
                        close = true;
                    }
                });
                self.default_kind = kind;
                self.default_level = level;
            }
            Dialog::ConfirmDelete(sel) => {
                let sel = sel.clone();
                egui::Modal::new(egui::Id::new("del")).show(ctx, |ui| {
                    ui.set_min_width(320.0);
                    ui.heading(tr("Delete"));
                    ui.label(tr("Do you want to delete the selected file(s)?"));
                    ui.label(trf("{n} item(s)", &[("n", &sel.len())]));
                    ui.add_space(10.0);
                    ui.horizontal(|ui| {
                        if ui.button(format!("   {}   ", tr("Yes"))).clicked() {
                            action = Some(Box::new(move |app: &mut App| app.do_delete(sel)));
                        }
                        if ui.button(format!("   {}   ", tr("No"))).clicked() {
                            close = true;
                        }
                    });
                });
            }
            Dialog::Extract { dest, only } => {
                egui::Modal::new(egui::Id::new("extract")).show(ctx, |ui| {
                    ui.set_min_width(460.0);
                    ui.heading(tr("Extract"));
                    ui.label(match only {
                        Some(s) => trf("Extract {n} selected item(s) to:", &[("n", &s.len())]),
                        None => tr("Extract all files to:").into(),
                    });
                    ui.horizontal(|ui| {
                        ui.add(egui::TextEdit::singleline(dest).desired_width(360.0));
                        if ui.button(tr("Browse...")).clicked() {
                            if let Some(p) = rfd::FileDialog::new().pick_folder() {
                                *dest = p.display().to_string();
                            }
                        }
                    });
                    ui.add_space(10.0);
                    ui.horizontal(|ui| {
                        if ui.button(format!("   {}   ", tr("Extract"))).clicked() {
                            let (dest, only) = (PathBuf::from(dest.clone()), only.clone());
                            action =
                                Some(Box::new(move |app: &mut App| app.do_extract(dest, only)));
                        }
                        if ui.button(format!("   {}   ", tr("Cancel"))).clicked() {
                            close = true;
                        }
                    });
                });
            }
            Dialog::New(d) => {
                egui::Modal::new(egui::Id::new("new")).show(ctx, |ui| {
                    ui.set_min_width(500.0);
                    ui.heading(if d.add_to_open {
                        tr("Add Files")
                    } else {
                        tr("New Archive")
                    });
                    ui.add_space(4.0);
                    if !d.add_to_open {
                        ui.label(tr("Archive name:"));
                        ui.horizontal(|ui| {
                            ui.add(egui::TextEdit::singleline(&mut d.out).desired_width(400.0));
                            if ui.button(tr("Browse...")).clicked() {
                                if let Some(p) = rfd::FileDialog::new()
                                    .add_filter(d.kind.label(), &[d.kind.ext()])
                                    .save_file()
                                {
                                    d.out = p.display().to_string();
                                }
                            }
                        });
                    }
                    let old = d.kind;
                    if d.add_to_open {
                        ui.horizontal(|ui| {
                            ui.label(tr("Compression level:"));
                            level_picker(ui, &mut d.level);
                        });
                    } else {
                        format_level_pickers(ui, &mut d.kind, &mut d.level);
                        if d.kind != old && !d.out.is_empty() {
                            d.out = Path::new(&d.out)
                                .with_extension(d.kind.ext())
                                .display()
                                .to_string();
                        }
                        ui.add_space(6.0);
                        new_options(ui, d);
                    }
                    ui.add_space(6.0);
                    ui.label(trf("Files to add ({n}):", &[("n", &d.inputs.len())]));
                    egui::Frame::group(ui.style()).show(ui, |ui| {
                        egui::ScrollArea::vertical()
                            .max_height(140.0)
                            .auto_shrink([false, true])
                            .show(ui, |ui| {
                                let mut remove = None;
                                for (i, p) in d.inputs.iter().enumerate() {
                                    ui.horizontal(|ui| {
                                        if ui.small_button("✖").clicked() {
                                            remove = Some(i);
                                        }
                                        ui.label(format!(
                                            "{} {}",
                                            if p.is_dir() { "🗀" } else { "🗋" },
                                            p.display()
                                        ));
                                    });
                                }
                                if let Some(i) = remove {
                                    d.inputs.remove(i);
                                }
                                if d.inputs.is_empty() {
                                    ui.weak(tr(
                                        "Add files or folders, or drop them on the window.",
                                    ));
                                }
                            });
                    });
                    ui.horizontal(|ui| {
                        if ui.button(tr("Add files...")).clicked() {
                            if let Some(f) = rfd::FileDialog::new().pick_files() {
                                d.inputs.extend(f);
                            }
                        }
                        if ui.button(tr("Add folder...")).clicked() {
                            if let Some(f) = rfd::FileDialog::new().pick_folder() {
                                d.inputs.push(f);
                            }
                        }
                    });
                    if !d.add_to_open && d.out.is_empty() {
                        if let Some(first) = d.inputs.first() {
                            d.out = first.with_extension(d.kind.ext()).display().to_string();
                        }
                    }
                    ui.add_space(10.0);
                    ui.horizontal(|ui| {
                        let ok = !d.inputs.is_empty()
                            && (d.add_to_open || (!d.out.trim().is_empty() && d.pw_ok()));
                        if ui
                            .add_enabled(ok, egui::Button::new(format!("   {}   ", tr("Start"))))
                            .clicked()
                        {
                            let (out, kind, level, inputs, add, opts) = (
                                PathBuf::from(d.out.trim()),
                                d.kind,
                                d.level,
                                d.inputs.clone(),
                                d.add_to_open,
                                d.opts(),
                            );
                            action = Some(Box::new(move |app: &mut App| {
                                if add {
                                    app.do_add(inputs, level);
                                } else {
                                    app.do_create(out, kind, inputs, level, opts);
                                }
                            }));
                        }
                        if ui.button(format!("   {}   ", tr("Cancel"))).clicked() {
                            close = true;
                        }
                    });
                });
            }
        }
        if close || action.is_some() {
            self.dialog = Dialog::None;
        }
        if let Some(a) = action {
            a(self);
        }
    }

    fn shortcuts(&mut self, ctx: &egui::Context) {
        if !matches!(self.dialog, Dialog::None) || self.job.is_some() {
            return;
        }
        let (o, n, w, f, del, all, back, f1) = ctx.input(|i| {
            let c = i.modifiers.command;
            (
                c && i.key_pressed(egui::Key::O),
                c && i.key_pressed(egui::Key::N),
                c && i.key_pressed(egui::Key::W),
                c && i.key_pressed(egui::Key::F),
                i.key_pressed(egui::Key::Delete),
                c && i.key_pressed(egui::Key::A),
                i.key_pressed(egui::Key::Backspace),
                i.key_pressed(egui::Key::F1),
            )
        });
        let typing = ctx.memory(|m| m.focused().is_some());
        if o {
            self.pick_open();
        }
        if n {
            self.start_new(vec![]);
        }
        if w {
            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
        }
        if f1 {
            self.dialog = Dialog::About;
        }
        if self.opened.is_none() || typing {
            return;
        }
        if f {
            self.show_find = true;
        }
        if del && !self.selected.is_empty() {
            self.start_delete();
        }
        if all {
            self.selected = self.rows().into_iter().collect();
        }
        if back && !self.cwd.is_empty() {
            self.go_up();
        }
    }

    /// Dropped files: an archive opens; other files go into the open archive or a new one.
    fn handle_drops(&mut self, ctx: &egui::Context) {
        let dropped: Vec<PathBuf> = ctx.input(|i| {
            i.raw
                .dropped_files
                .iter()
                .map(|f| f.path().to_path_buf())
                .collect()
        });
        if dropped.is_empty() {
            return;
        }
        if let Dialog::New(d) = &mut self.dialog {
            d.inputs.extend(dropped);
            return;
        }
        if self.opened.is_none()
            && dropped.len() == 1
            && archive::Kind::detect(&dropped[0]).is_some()
            && dropped[0].is_file()
        {
            self.open(dropped[0].clone());
        } else if self.opened.is_some() {
            self.start_add(dropped);
        } else {
            self.start_new(dropped);
        }
    }

    /// --screenshot: after SHOT_FRAMES frames, capture the window, save it and quit.
    fn screenshot(&mut self, ctx: &egui::Context) {
        let Some(out) = self.shot.clone() else { return };
        self.frame_no += 1;
        ctx.request_repaint();
        if self.frame_no == SHOT_FRAMES {
            ctx.send_viewport_cmd(egui::ViewportCommand::Screenshot(Default::default()));
        }
        let img = ctx.input(|i| {
            i.raw.events.iter().find_map(|e| match e {
                egui::Event::Screenshot { image, .. } => Some(image.clone()),
                _ => None,
            })
        });
        if let Some(img) = img {
            let [w, h] = img.size;
            let bytes: Vec<u8> = img.pixels.iter().flat_map(|c| c.to_array()).collect();
            if let Some(dir) = out.parent() {
                let _ = std::fs::create_dir_all(dir);
            }
            if let Err(e) =
                image::save_buffer(&out, &bytes, w as u32, h as u32, image::ColorType::Rgba8)
            {
                eprintln!("screenshot failed: {e}");
            }
            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
        }
    }
}

fn level_picker(ui: &mut egui::Ui, level: &mut u8) {
    egui::ComboBox::from_id_salt("level")
        .selected_text(level_name(*level))
        .width(220.0)
        .show_ui(ui, |ui| {
            for i in 0..4 {
                ui.selectable_value(level, i, level_name(i));
            }
        });
}

fn format_level_pickers(ui: &mut egui::Ui, kind: &mut Kind, level: &mut u8) {
    egui::Grid::new("fmt")
        .num_columns(2)
        .spacing([12.0, 8.0])
        .show(ui, |ui| {
            ui.label(tr("Archive format:"));
            egui::ComboBox::from_id_salt("format")
                .selected_text(kind.label())
                .width(220.0)
                .show_ui(ui, |ui| {
                    for k in Kind::ALL {
                        ui.selectable_value(kind, k, k.label());
                    }
                });
            ui.end_row();
            ui.label(tr("Compression level:"));
            ui.add_enabled_ui(*kind != Kind::Tar, |ui| level_picker(ui, level));
            ui.end_row();
        });
}

impl NewDlg {
    fn can_encrypt(&self) -> bool {
        matches!(self.kind, Kind::Zip | Kind::SevenZ)
    }

    fn pw_ok(&self) -> bool {
        !(self.use_pw && self.can_encrypt()) || (!self.pw.is_empty() && self.pw == self.pw2)
    }

    fn opts(&self) -> NewOpts {
        let mb = match VOLUME_PRESETS.get(self.preset) {
            Some(&(_, 0)) | None => self.custom_mb.max(1) as u64,
            Some(&(_, mb)) => mb,
        };
        NewOpts {
            password: (self.use_pw && self.can_encrypt()).then(|| self.pw.clone()),
            zip_crypto: self.zip_crypto,
            encrypt_names: self.encrypt_names,
            volume_size: (self.split && self.can_encrypt()).then_some(mb * 1024 * 1024),
        }
    }
}

/// New Archive: password and split options (ZIP and 7Z only).
fn new_options(ui: &mut egui::Ui, d: &mut NewDlg) {
    let can = d.can_encrypt();
    ui.add_enabled_ui(can, |ui| {
        ui.checkbox(&mut d.use_pw, tr("Set password"))
            .on_disabled_hover_text(tr("Passwords and volumes are available for ZIP and 7Z."));
        if d.use_pw && can {
            egui::Grid::new("pw")
                .num_columns(2)
                .spacing([12.0, 6.0])
                .show(ui, |ui| {
                    ui.label(tr("Password:"));
                    ui.add(
                        egui::TextEdit::singleline(&mut d.pw)
                            .password(!d.show_pw)
                            .desired_width(220.0),
                    );
                    ui.end_row();
                    ui.label(tr("Confirm password:"));
                    ui.add(
                        egui::TextEdit::singleline(&mut d.pw2)
                            .password(!d.show_pw)
                            .desired_width(220.0),
                    );
                    ui.end_row();
                    if d.kind == Kind::Zip {
                        ui.label(tr("Encryption:"));
                        let name = |zc: bool| {
                            if zc {
                                tr("ZipCrypto (compatible)")
                            } else {
                                tr("AES-256 (recommended)")
                            }
                        };
                        egui::ComboBox::from_id_salt("enc")
                            .selected_text(name(d.zip_crypto))
                            .width(220.0)
                            .show_ui(ui, |ui| {
                                ui.selectable_value(&mut d.zip_crypto, false, name(false));
                                ui.selectable_value(&mut d.zip_crypto, true, name(true));
                            });
                        ui.end_row();
                    }
                });
            ui.checkbox(&mut d.show_pw, tr("Show password"));
            if d.kind == Kind::SevenZ {
                ui.checkbox(&mut d.encrypt_names, tr("Encrypt file names"));
            }
            if !d.pw2.is_empty() && d.pw != d.pw2 {
                ui.colored_label(
                    Color32::from_rgb(0xC0, 0x30, 0x20),
                    tr("The passwords do not match."),
                );
            }
        }
        ui.add_space(4.0);
        ui.horizontal(|ui| {
            ui.checkbox(&mut d.split, tr("Split into volumes"));
            if d.split && can {
                ui.label(tr("Volume size:"));
                egui::ComboBox::from_id_salt("vol")
                    .selected_text(tr(VOLUME_PRESETS[d.preset.min(4)].0))
                    .width(130.0)
                    .show_ui(ui, |ui| {
                        for (i, (name, _)) in VOLUME_PRESETS.iter().enumerate() {
                            ui.selectable_value(&mut d.preset, i, tr(name));
                        }
                    });
                if VOLUME_PRESETS[d.preset.min(4)].1 == 0 {
                    ui.add(
                        egui::DragValue::new(&mut d.custom_mb)
                            .range(1..=1_000_000)
                            .suffix(" MB"),
                    );
                }
            }
        });
    });
}

/// Line icons on the start tiles: stacked pages with an arrow (open) or a zipper (new).
fn draw_tile_icon(p: &egui::Painter, c: Pos2, is_open: bool) {
    let s = Stroke::new(1.4, Color32::from_gray(60));
    let back = Rect::from_center_size(c + Vec2::new(-3.0, -3.0), Vec2::new(30.0, 38.0));
    let front = back.translate(Vec2::new(6.0, 6.0));
    p.rect_stroke(back, 1.0, s, egui::StrokeKind::Middle);
    p.rect_filled(front, 1.0, Color32::WHITE);
    p.rect_stroke(front, 1.0, s, egui::StrokeKind::Middle);
    let fc = front.center();
    if is_open {
        p.line_segment([fc + Vec2::new(-8.0, 0.0), fc + Vec2::new(8.0, 0.0)], s);
        p.line(
            vec![
                fc + Vec2::new(3.0, -5.0),
                fc + Vec2::new(8.0, 0.0),
                fc + Vec2::new(3.0, 5.0),
            ],
            s,
        );
    } else {
        let x = fc.x;
        p.rect_stroke(
            Rect::from_center_size(Pos2::new(x, front.min.y + 6.0), Vec2::new(6.0, 6.0)),
            0.0,
            s,
            egui::StrokeKind::Middle,
        );
        let mut y = front.min.y + 11.0;
        while y < front.max.y - 4.0 {
            p.line_segment([Pos2::new(x - 3.0, y), Pos2::new(x, y)], s);
            p.line_segment([Pos2::new(x, y + 3.0), Pos2::new(x + 3.0, y + 3.0)], s);
            y += 6.0;
        }
        p.line_segment(
            [
                Pos2::new(x, front.min.y + 9.0),
                Pos2::new(x, front.max.y - 3.0),
            ],
            s,
        );
    }
}

impl eframe::App for App {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        self.handle_drops(&ctx);
        self.shortcuts(&ctx);
        let title = match &self.opened {
            Some(o) => format!(
                "{} - {}",
                o.path.file_name().unwrap_or_default().to_string_lossy(),
                tr("tailzip")
            ),
            None => tr("tailzip").to_string(),
        };
        ctx.send_viewport_cmd(egui::ViewportCommand::Title(title));

        egui::Panel::top("menu")
            .frame(
                egui::Frame::new()
                    .fill(Color32::WHITE)
                    .inner_margin(egui::Margin::symmetric(6, 3)),
            )
            .show(ui, |ui| {
                self.menu_bar(ui);
            });

        if self.opened.is_none() {
            egui::CentralPanel::default()
                .frame(egui::Frame::new())
                .show(ui, |ui| self.start_page(ui));
        } else {
            if self.show_toolbar {
                egui::Panel::top("toolbar")
                    .frame(
                        egui::Frame::new()
                            .fill(Color32::WHITE)
                            .inner_margin(egui::Margin::symmetric(6, 4))
                            .stroke(Stroke::new(1.0, Color32::from_gray(225))),
                    )
                    .show(ui, |ui| self.toolbar(ui));
            }
            if self.show_status {
                egui::Panel::bottom("status")
                    .frame(
                        egui::Frame::new()
                            .fill(Color32::from_gray(245))
                            .inner_margin(egui::Margin::symmetric(8, 4)),
                    )
                    .show(ui, |ui| self.status_bar(ui));
            }
            egui::Panel::left("tree")
                .resizable(true)
                .default_size(200.0)
                .frame(
                    egui::Frame::new()
                        .fill(Color32::from_rgb(0xFC, 0xF9, 0xF7))
                        .inner_margin(6),
                )
                .show(ui, |ui| self.folder_tree(ui));
            egui::CentralPanel::default()
                .frame(egui::Frame::new().fill(Color32::WHITE))
                .show(ui, |ui| {
                    egui::ScrollArea::horizontal()
                        .auto_shrink(false)
                        .show(ui, |ui| self.file_list(ui));
                });
        }

        if self.job.is_some() {
            self.poll_job(&ctx);
        } else {
            self.dialogs(&ctx);
        }
        self.screenshot(&ctx);
    }
}

#[cfg(test)]
mod ui_tests {
    use super::*;

    /// Renders the UI offscreen (no window or display needed) to `$TAILZIP_SHOTS/v2_<lang>_<view>.png`.
    #[test]
    #[ignore = "writes screenshots: TAILZIP_SHOTS=<dir> cargo test --release -- --ignored"]
    fn ui_screenshots() {
        let Some(dir) = std::env::var_os("TAILZIP_SHOTS").map(PathBuf::from) else {
            return;
        };
        let fx = |n: &str| {
            Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("tests/fixtures")
                .join(n)
        };
        std::fs::create_dir_all(&dir).unwrap();
        for lang in Lang::ALL {
            i18n::set(lang);
            for view in ["new", "rar", "password"] {
                let mut app = App::default();
                match view {
                    "new" => {
                        app.start_new(vec![PathBuf::from("Documents"), PathBuf::from("photo.jpg")]);
                        if let Dialog::New(d) = &mut app.dialog {
                            d.kind = Kind::SevenZ;
                            d.out = "Documents.7z".into();
                            (d.use_pw, d.encrypt_names, d.split) = (true, true, true);
                            d.pw = "Test1234".into();
                            d.pw2 = "Test1234".into();
                        }
                    }
                    "rar" => app.open(fx("rar5_enc.rar")),
                    _ => app.open(fx("rar5_hdr.rar")),
                }
                let mut h = egui_kittest::Harness::builder()
                    .with_size([930.0, 660.0])
                    .wgpu()
                    .build_eframe(|cc| {
                        setup_style(&cc.egui_ctx);
                        app
                    });
                h.run_steps(8);
                let img = h.render().unwrap();
                img.save(dir.join(format!("v2_{}_{view}.png", lang.code())))
                    .unwrap();
            }
        }
    }
}
