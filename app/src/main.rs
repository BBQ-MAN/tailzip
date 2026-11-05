// Desktop archiver: start page, archive browser, extract / new / add / delete / test.
// No console window on Windows release builds.
#![cfg_attr(all(windows, not(debug_assertions)), windows_subsystem = "windows")]
mod archive;
mod i18n;

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use archive::{Entry, Kind};
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
            setup_fonts(&cc.egui_ctx);
            let mut vis = egui::Visuals::light();
            vis.selection.bg_fill = Color32::from_rgb(0xFB, 0xD3, 0xB4);
            vis.selection.stroke = Stroke::new(1.0, ACCENT);
            vis.hyperlink_color = ACCENT;
            cc.egui_ctx.set_visuals(vis);
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
}

struct NewDlg {
    out: String,
    kind: Kind,
    level: u8,
    inputs: Vec<PathBuf>,
    /// true = "Add" to the open archive instead of creating a new one.
    add_to_open: bool,
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
        match archive::list(&path) {
            Ok((kind, entries)) => {
                self.recent.retain(|p| p != &path);
                self.recent.insert(0, path.clone());
                self.recent.truncate(10);
                self.opened = Some(Opened {
                    path,
                    kind,
                    entries,
                });
                self.cwd.clear();
                self.selected.clear();
                self.anchor = None;
                self.filter.clear();
            }
            Err(e) => self.msg(
                tr("Open Archive"),
                trf(
                    "Cannot open {path}:\n{e}",
                    &[("path", &path.display()), ("e", &e)],
                ),
            ),
        }
    }

    fn reload(&mut self) {
        let Some(o) = &self.opened else { return };
        let (path, cwd) = (o.path.clone(), self.cwd.clone());
        self.open(path);
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
            .add_filter(tr("Archives"), &["zip", "7z", "tar", "tgz", "gz"])
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
        });
    }

    fn start_add(&mut self, inputs: Vec<PathBuf>) {
        if self.opened.is_none() {
            return;
        }
        self.dialog = Dialog::New(NewDlg {
            out: String::new(),
            kind: Kind::Zip,
            level: self.default_level,
            inputs,
            add_to_open: true,
        });
    }

    fn start_extract(&mut self, only: Option<Vec<String>>) {
        let Some(o) = &self.opened else { return };
        // Default destination: a folder named after the archive, next to it.
        let stem = o.path.file_name().unwrap_or_default().to_string_lossy();
        let stem = stem.trim_end_matches(".gz").trim_end_matches(".tar");
        let stem = stem.rsplit_once('.').map(|(a, _)| a).unwrap_or(stem);
        let dest = o.path.parent().unwrap_or(Path::new(".")).join(stem);
        self.dialog = Dialog::Extract {
            dest: dest.display().to_string(),
            only,
        };
    }

    fn selected_paths(&self) -> Vec<String> {
        let Some(o) = &self.opened else { return vec![] };
        self.selected
            .iter()
            .map(|&i| o.entries[i].path.clone())
            .collect()
    }

    fn test_archive(&mut self) {
        let Some(o) = &self.opened else { return };
        let r = archive::test(&o.path);
        match r {
            Ok(n) => self.msg(
                tr("Test"),
                trf(
                    "No errors found in this archive.\n{n} file(s) tested.",
                    &[("n", &n)],
                ),
            ),
            Err(e) => self.msg(
                tr("Test"),
                trf("Errors were found in this archive:\n{e}", &[("e", &e)]),
            ),
        }
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
        let e = &o.entries[idx];
        if e.is_dir {
            self.cwd = e.path.clone();
            self.selected.clear();
            self.filter.clear();
            return;
        }
        let tmp = std::env::temp_dir().join(format!("archiver-{}", std::process::id()));
        let (path, inner) = (o.path.clone(), e.path.clone());
        match archive::extract(&path, &tmp, Some(std::slice::from_ref(&inner))) {
            Ok(_) => open_with_system(&tmp.join(&inner)),
            Err(e) => self.msg(tr("Open"), e),
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
            ui.menu_button(tr("Edit"), |ui| {
                if ui
                    .add_enabled(has, egui::Button::new(tr("Add Files...")))
                    .clicked()
                {
                    self.pick_add();
                }
                if ui
                    .add_enabled(
                        has && !self.selected.is_empty(),
                        item(tr("Delete Files"), "Del"),
                    )
                    .clicked()
                {
                    self.dialog = Dialog::ConfirmDelete(self.selected_paths());
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
        let buttons: [(&str, &str, bool); 6] = [
            ("🗁", tr("Open"), true),
            ("📤", tr("Extract"), true),
            ("🗋", tr("New"), true),
            ("➕", tr("Add"), true),
            ("🗑", tr("Delete"), sel),
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
                let resp = resp.on_hover_text(label);
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
                        4 => self.dialog = Dialog::ConfirmDelete(self.selected_paths()),
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
                        if e.is_dir {
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
                        if ui.button(tr("Delete")).clicked() {
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
            self.dialog = Dialog::ConfirmDelete(self.selected_paths());
        }
    }

    fn status_bar(&self, ui: &mut egui::Ui) {
        let Some(o) = &self.opened else { return };
        ui.horizontal(|ui| {
            if self.selected.is_empty() {
                let files = o.entries.iter().filter(|e| !e.is_dir).count();
                let size: u64 = o.entries.iter().map(|e| e.size).sum();
                let packed: u64 = o.entries.iter().map(|e| e.packed).sum();
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
                let sel: Vec<&Entry> = self.selected.iter().map(|&i| &o.entries[i]).collect();
                let files = sel.iter().filter(|e| !e.is_dir).count();
                let dirs = sel.len() - files;
                let size: u64 = sel.iter().map(|e| e.size).sum();
                ui.label(trf(
                    "{files} file(s), {dirs} folder(s) selected  ·  Selected file size: {size}",
                    &[("files", &files), ("dirs", &dirs), ("size", &human(size))],
                ));
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
                    ui.label(tr("Open, browse, extract and create ZIP, 7Z, TAR and TGZ archives."));
                    ui.label(tr("Runs on Windows and macOS."));
                    ui.add_space(6.0);
                    ui.label(tr("Tip: drag files onto the window to compress them, or drop an archive to open it."));
                    ui.add_space(10.0);
                    if ui.button(format!("   {}   ", tr("OK"))).clicked() {
                        close = true;
                    }
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
                            action = Some(Box::new(move |app: &mut App| {
                                let path = app.opened.as_ref().map(|o| o.path.clone()).unwrap();
                                match archive::delete(&path, &sel) {
                                    Ok(()) => app.reload(),
                                    Err(e) => app.msg(tr("Delete"), e),
                                }
                            }));
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
                            action = Some(Box::new(move |app: &mut App| {
                                let path = app.opened.as_ref().map(|o| o.path.clone()).unwrap();
                                // ponytail: runs on the UI thread; move to a worker with progress if huge archives freeze it.
                                match archive::extract(&path, &dest, only.as_deref()) {
                                    Ok(n) => {
                                        app.msg(tr("Extract"), trf("Extraction completed successfully.\n{n} file(s) extracted.", &[("n", &n)]));
                                        open_with_system(&dest);
                                    }
                                    Err(e) => app.msg(tr("Extract"), trf("Extraction has failed.\n({e})", &[("e", &e)])),
                                }
                            }));
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
                        let ok =
                            !d.inputs.is_empty() && (d.add_to_open || !d.out.trim().is_empty());
                        if ui
                            .add_enabled(ok, egui::Button::new(format!("   {}   ", tr("Start"))))
                            .clicked()
                        {
                            let (out, kind, level, inputs, add) = (
                                PathBuf::from(d.out.trim()),
                                d.kind,
                                d.level,
                                d.inputs.clone(),
                                d.add_to_open,
                            );
                            action = Some(Box::new(move |app: &mut App| {
                                if add {
                                    let path = app.opened.as_ref().map(|o| o.path.clone()).unwrap();
                                    let cwd = app.cwd.clone();
                                    match archive::add(&path, &inputs, &cwd, level) {
                                        Ok(()) => app.reload(),
                                        Err(e) => app.msg(tr("Add"), e),
                                    }
                                } else {
                                    match archive::create(&out, kind, &inputs, level) {
                                        Ok(_) => {
                                            app.open(out);
                                            app.msg(
                                                tr("New Archive"),
                                                tr("Archiving completed successfully."),
                                            );
                                        }
                                        Err(e) => app.msg(tr("New Archive"), e),
                                    }
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
        if !matches!(self.dialog, Dialog::None) {
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
            self.dialog = Dialog::ConfirmDelete(self.selected_paths());
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

        self.dialogs(&ctx);
        self.screenshot(&ctx);
    }
}
