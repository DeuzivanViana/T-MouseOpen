//! User interface (egui): dark, compact theme with slightly rounded corners.

use crate::protocol::{self as p, Command, DpiConfig, Status, DPI_STAGES};
use crate::state::Saved;
use crate::worker::{Event, Request, Worker};
use eframe::egui::{self, Color32, CornerRadius, Frame, Margin, RichText, Stroke, Vec2};
use std::collections::VecDeque;
use std::sync::mpsc::{channel, Receiver, Sender};

// ------------------------------------------------------------------ theme
// Monochrome palette: blacks and grays, near-white accent.
const BG: Color32 = Color32::from_rgb(0x05, 0x05, 0x05);
const CARD: Color32 = Color32::from_rgb(0x0e, 0x0e, 0x0e);
const CARD_HI: Color32 = Color32::from_rgb(0x18, 0x18, 0x18);
const HOVER: Color32 = Color32::from_rgb(0x24, 0x24, 0x24);
const BORDER: Color32 = Color32::from_rgb(0x22, 0x22, 0x22);
const BORDER_HI: Color32 = Color32::from_rgb(0x3a, 0x3a, 0x3a);
const WELL: Color32 = Color32::from_rgb(0x00, 0x00, 0x00);
const TEXT: Color32 = Color32::from_rgb(0xed, 0xed, 0xed);
const MUTED: Color32 = Color32::from_rgb(0x86, 0x86, 0x86);
const ACCENT: Color32 = Color32::from_rgb(0xed, 0xed, 0xed);
const ON_ACCENT: Color32 = Color32::from_rgb(0x05, 0x05, 0x05);
const OK: Color32 = Color32::from_rgb(0x4a, 0xb8, 0x86);
const WARN: Color32 = Color32::from_rgb(0xd9, 0xa4, 0x41);
const BAD: Color32 = Color32::from_rgb(0xe0, 0x5a, 0x52);
const COL_GAP: f32 = 10.0;

pub fn apply_theme(ctx: &egui::Context) {
    let mut v = egui::Visuals::dark();
    v.panel_fill = BG;
    v.window_fill = CARD;
    v.window_stroke = Stroke::new(1.0, BORDER);
    v.window_corner_radius = CornerRadius::same(10);
    v.extreme_bg_color = WELL;
    v.faint_bg_color = CARD_HI;
    v.selection.bg_fill = Color32::from_rgb(0xbd, 0xbd, 0xbd);
    v.selection.stroke = Stroke::new(1.0, ON_ACCENT);
    v.hyperlink_color = ACCENT;
    v.override_text_color = Some(TEXT);
    for w in [&mut v.widgets.noninteractive, &mut v.widgets.inactive, &mut v.widgets.hovered, &mut v.widgets.active, &mut v.widgets.open] {
        w.corner_radius = CornerRadius::same(6);
    }
    v.widgets.noninteractive.bg_stroke = Stroke::new(1.0, BORDER);
    v.widgets.inactive.bg_fill = CARD_HI;
    v.widgets.inactive.weak_bg_fill = CARD_HI;
    v.widgets.inactive.bg_stroke = Stroke::new(1.0, BORDER);
    v.widgets.inactive.fg_stroke = Stroke::new(1.0, TEXT);
    v.widgets.hovered.bg_fill = HOVER;
    v.widgets.hovered.weak_bg_fill = HOVER;
    v.widgets.hovered.bg_stroke = Stroke::new(1.0, BORDER_HI);
    v.widgets.hovered.fg_stroke = Stroke::new(1.5, TEXT);
    // "active" also sets the strong (bold) text color, so it must stay light.
    v.widgets.active.bg_fill = BORDER_HI;
    v.widgets.active.weak_bg_fill = BORDER_HI;
    v.widgets.active.bg_stroke = Stroke::new(1.0, MUTED);
    v.widgets.active.fg_stroke = Stroke::new(1.5, TEXT);
    ctx.set_visuals(v);
    ctx.global_style_mut(|s| {
        s.spacing.item_spacing = Vec2::new(8.0, 6.0);
        s.spacing.button_padding = Vec2::new(12.0, 5.0);
        s.spacing.interact_size.y = 26.0;
        s.spacing.slider_rail_height = 4.0;
        use egui::{FontId, TextStyle::*};
        s.text_styles.insert(Body, FontId::proportional(13.5));
        s.text_styles.insert(Button, FontId::proportional(13.5));
        s.text_styles.insert(Small, FontId::proportional(11.5));
        s.text_styles.insert(Heading, FontId::proportional(17.0));
        s.text_styles.insert(Monospace, FontId::monospace(12.0));
    });
}

/// Card frame: outer margin + border = 2 * 14 + 2.
const CARD_CHROME: f32 = 30.0;

/// Draws a card at least `min_height` tall (outer). Returns its natural outer height,
/// so columns can be equalized on the next frame.
fn card(ui: &mut egui::Ui, title: &str, min_height: f32, add: impl FnOnce(&mut egui::Ui)) -> f32 {
    let mut natural = 0.0;
    Frame::NONE
        .fill(CARD)
        .stroke(Stroke::new(1.0, BORDER))
        .corner_radius(CornerRadius::same(8))
        .inner_margin(Margin::same(14))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.label(RichText::new(title.to_uppercase()).size(11.0).color(MUTED).strong());
            ui.add_space(6.0);
            add(ui);
            let used = ui.min_rect().height();
            natural = used + CARD_CHROME;
            // set_min_height reserves space from the cursor (after the content), so ask only for what is missing.
            let missing = min_height - CARD_CHROME - used;
            if missing > 0.0 {
                ui.set_min_height(missing);
            }
        });
    natural
}

fn kv(ui: &mut egui::Ui, k: &str, v: impl Into<RichText>) {
    ui.horizontal(|ui| {
        ui.label(RichText::new(k).color(MUTED));
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| ui.label(v.into()));
    });
}

/// Phone-style battery: rounded body, terminal nub, level fill and a bolt while charging.
fn battery_icon(ui: &mut egui::Ui, pct: u8, charging: bool, level: Color32) {
    use egui::{pos2, vec2, Rect, Shape};
    let (rect, _) = ui.allocate_exact_size(vec2(40.0, 19.0), egui::Sense::hover());
    let painter = ui.painter();
    let body = Rect::from_min_size(rect.min, vec2(36.0, 19.0));
    let nub = Rect::from_center_size(pos2(body.right() + 2.2, body.center().y), vec2(3.0, 7.0));
    painter.rect_stroke(body, CornerRadius::same(4), Stroke::new(1.4, MUTED), egui::StrokeKind::Inside);
    painter.rect_filled(nub, CornerRadius { nw: 0, sw: 0, ne: 2, se: 2 }, MUTED);
    let inner = body.shrink(2.8);
    let w = inner.width() * (pct.min(100) as f32 / 100.0);
    if w > 0.5 {
        let fill = Rect::from_min_size(inner.min, vec2(w.max(2.0), inner.height()));
        painter.rect_filled(fill, CornerRadius::same(2), level);
    }
    if charging {
        let c = body.center();
        // Bolt as two convex triangles (a single bolt polygon is concave).
        let upper = vec![pos2(c.x + 2.0, c.y - 7.0), pos2(c.x - 4.5, c.y + 1.2), pos2(c.x + 0.9, c.y + 1.2)];
        let lower = vec![pos2(c.x - 0.9, c.y - 1.2), pos2(c.x + 4.5, c.y - 1.2), pos2(c.x - 2.0, c.y + 7.0)];
        painter.add(Shape::convex_polygon(upper, TEXT, Stroke::NONE));
        painter.add(Shape::convex_polygon(lower, TEXT, Stroke::NONE));
    }
}

fn primary(text: &str) -> egui::Button<'_> {
    egui::Button::new(RichText::new(text).color(ON_ACCENT).strong())
        .fill(ACCENT)
        .corner_radius(CornerRadius::same(6))
        .min_size(Vec2::new(0.0, 28.0))
}

// ------------------------------------------------------------------ app

enum Conn {
    Searching(String),
    Connected { name: String, version: String },
}

pub struct App {
    tx: Sender<Request>,
    rx: Receiver<Event>,
    conn: Conn,
    status: Option<Status>,
    saved: Saved,
    polling_choice: u16,
    dpi: DpiConfig,
    busy: bool,
    toast: Option<(String, Color32)>,
    log: VecDeque<String>,
    show_log: bool,
    confirm_polling: Option<u16>,
    first_status: bool,
    pending: Option<Command>,
    /// Natural heights measured on the previous frame (left column, right column).
    natural: (f32, f32),
}

impl App {
    pub fn new(
        cc: &eframe::CreationContext<'_>,
        demo: bool,
        initial: Option<Box<dyn crate::device::Backend>>,
        sudo_mode: bool,
        note: Option<String>,
    ) -> Self {
        apply_theme(&cc.egui_ctx);
        let (req_tx, req_rx) = channel();
        let (ev_tx, ev_rx) = channel();
        let ctx = cc.egui_ctx.clone();
        Worker::spawn(req_rx, ev_tx, Box::new(move || ctx.request_repaint()), demo, initial, sudo_mode);
        let saved = Saved::load();
        let mut app = Self {
            tx: req_tx,
            rx: ev_rx,
            conn: Conn::Searching("Looking for the receiver…".into()),
            status: None,
            polling_choice: saved.polling_hz.unwrap_or(1000),
            dpi: saved.dpi.unwrap_or_default(),
            saved,
            busy: false,
            toast: None,
            log: VecDeque::new(),
            show_log: false,
            confirm_polling: None,
            first_status: true,
            pending: None,
            natural: (0.0, 0.0),
        };
        if let Some(n) = note {
            app.push_log(n);
        }
        app
    }

    fn push_log(&mut self, s: String) {
        let t = time_hms();
        self.log.push_back(format!("{t}  {s}"));
        while self.log.len() > 300 {
            self.log.pop_front();
        }
    }

    fn handle_events(&mut self) {
        while let Ok(ev) = self.rx.try_recv() {
            match ev {
                Event::Searching(msg) => {
                    if !matches!(&self.conn, Conn::Searching(m) if *m == msg) {
                        self.push_log(msg.clone());
                    }
                    self.conn = Conn::Searching(msg);
                }
                Event::Connected { name, version } => {
                    self.push_log(format!("connected: {name} (version {version})"));
                    self.conn = Conn::Connected { name, version };
                    self.first_status = true;
                }
                Event::Disconnected(msg) => {
                    self.push_log(format!("disconnected: {msg}"));
                    self.conn = Conn::Searching(format!("Disconnected: {msg}"));
                    self.status = None;
                    self.busy = false;
                }
                Event::Status(st) => {
                    if self.status.map(|s| s.raw) != Some(st.raw) {
                        self.push_log(format!("status 04: {}", p::hex(&st.raw)));
                    }
                    if self.first_status {
                        self.first_status = false;
                        if let Some(hz) = st.polling_hz() {
                            self.polling_choice = hz;
                        }
                        if (st.dpi_stage() as usize) < DPI_STAGES {
                            self.dpi.active = st.dpi_stage();
                            self.dpi.enabled_mask |= 1 << st.dpi_stage();
                        }
                    }
                    self.status = Some(st);
                }
                Event::Sent { label, frames, ok } => {
                    for (f, a) in &frames {
                        self.push_log(format!("SET 06: {}", p::hex(&f[..23])));
                        self.push_log(format!("ack 06: {}", p::hex(&a[..4])));
                    }
                    self.busy = false;
                    if ok {
                        self.toast = Some((format!("{label} applied"), OK));
                        match self.pending.take() {
                            Some(Command::Polling(hz)) => self.saved.polling_hz = Some(hz),
                            Some(Command::Dpi(c)) => self.saved.dpi = Some(c),
                            None => {}
                        }
                        self.saved.store();
                    } else {
                        self.toast = Some((format!("{label}: the mouse did not acknowledge"), BAD));
                    }
                }
                Event::Log(s) => self.push_log(s),
                Event::Error(s) => {
                    self.busy = false;
                    self.toast = Some((s.clone(), BAD));
                    self.push_log(format!("error: {s}"));
                }
            }
        }
    }

    fn send(&mut self, cmd: Command) {
        self.pending = Some(cmd.clone());
        self.busy = true;
        self.toast = None;
        let _ = self.tx.send(Request::Send(cmd));
    }

    /// Connected and the mouse has reported its status at least once (2.4G link is up).
    fn ready(&self) -> bool {
        matches!(self.conn, Conn::Connected { .. }) && self.status.is_some()
    }

    // -------------------------------------------------------------- sections
    fn header(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            ui.label(RichText::new("TMouseOpen").heading().strong());
            ui.label(RichText::new("Titorion YDX-GM009").color(MUTED));
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                let (txt, col) = match &self.conn {
                    Conn::Connected { name, .. } if self.status.is_some() => (format!("Connected · {name}"), OK),
                    Conn::Connected { name, .. } => (format!("Waiting for the mouse · {name}"), WARN),
                    Conn::Searching(m) => (m.clone(), BAD),
                };
                Frame::NONE
                    .fill(CARD_HI)
                    .stroke(Stroke::new(1.0, BORDER))
                    .corner_radius(CornerRadius::same(12))
                    .inner_margin(Margin::symmetric(10, 3))
                    .show(ui, |ui| {
                        ui.horizontal(|ui| {
                            let (r, _) = ui.allocate_exact_size(Vec2::splat(8.0), egui::Sense::hover());
                            ui.painter().circle_filled(r.center(), 4.0, col);
                            ui.label(RichText::new(txt).size(12.0).color(col));
                        });
                    });
            });
        });
    }

    fn status_card(&mut self, ui: &mut egui::Ui) -> f32 {
        card(ui, "Status", 0.0, |ui| {
            match self.status {
                None => {
                    ui.label(RichText::new("Waiting for the first status report from the mouse (up to ~5 s).").color(WARN));
                    ui.label(RichText::new("If it takes longer, move the mouse to wake it up.").small().color(MUTED));
                    if let Some(hz) = self.saved.polling_hz {
                        ui.add_space(4.0);
                        kv(ui, "Last applied polling rate", RichText::new(format!("{hz} Hz")).color(MUTED));
                    }
                }
                Some(st) => {
                    ui.horizontal(|ui| {
                        let hz = st.polling_hz().map(|h| h.to_string()).unwrap_or_else(|| format!("0x{:02x}", st.polling_code()));
                        ui.label(RichText::new(hz).size(28.0).strong());
                        ui.label(RichText::new("Hz").color(MUTED));
                        ui.add_space(16.0);
                        let dpi = self.saved.dpi.map(|d| d.stages[st.dpi_stage().min(6) as usize]);
                        ui.label(RichText::new(dpi.map(|d| d.to_string()).unwrap_or_else(|| "—".into())).size(28.0).strong());
                        ui.label(RichText::new(format!("DPI · stage {}", st.dpi_stage() + 1)).color(MUTED));
                    });
                    ui.add_space(4.0);
                    let b = st.battery();
                    let col = if b <= 15 { BAD } else if b <= 35 { WARN } else { OK };
                    ui.horizontal(|ui| {
                        ui.label(RichText::new("Battery").color(MUTED));
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            ui.label(RichText::new(st.charge_text()).small().color(MUTED));
                            ui.label(RichText::new(format!("{b}%")).strong());
                            battery_icon(ui, b, st.charge_state() == 1, col);
                        });
                    });
                    ui.add_space(2.0);
                    kv(ui, "Sensor", st.sensor_name());
                    if let Conn::Connected { version, .. } = &self.conn {
                        kv(ui, "Firmware (likely)", version.as_str());
                    }
                    kv(ui, "Receiver max rate", st.max_polling().map(|h| format!("{h} Hz")).unwrap_or_else(|| "?".into()));
                }
            }
        })
    }

    fn polling_card(&mut self, ui: &mut egui::Ui, min_height: f32) -> f32 {
        card(ui, "Polling rate", min_height, |ui| {
            ui.horizontal(|ui| {
                for (hz, _) in p::POLLING {
                    let tested = p::POLLING_TESTED.contains(&hz);
                    let label = if tested { format!("{hz}") } else { format!("{hz}*") };
                    let sel = self.polling_choice == hz;
                    let btn = egui::Button::new(RichText::new(label).color(if sel { ON_ACCENT } else { TEXT }))
                        .fill(if sel { ACCENT } else { CARD_HI })
                        .corner_radius(CornerRadius::same(6))
                        .min_size(Vec2::new(58.0, 28.0));
                    if ui.add(btn).clicked() {
                        self.polling_choice = hz;
                    }
                }
            });
            ui.add_space(4.0);
            ui.horizontal(|ui| {
                let enabled = self.ready() && !self.busy;
                if ui.add_enabled(enabled, primary("Apply")).clicked() {
                    if p::POLLING_TESTED.contains(&self.polling_choice) {
                        self.send(Command::Polling(self.polling_choice));
                    } else {
                        self.confirm_polling = Some(self.polling_choice);
                    }
                }
                ui.label(RichText::new("* not yet tested on your mouse").small().color(MUTED));
            });
        })
    }

    fn dpi_card(&mut self, ui: &mut egui::Ui, min_height: f32) -> f32 {
        let sensor_ok = self.status.map(|s| s.sensor_code() == p::PAW3311);
        card(ui, "DPI · PAW3311", min_height, |ui| {
            ui.label(RichText::new("The mouse DPI button only cycles through enabled stages.").small().color(MUTED));
            ui.add_space(4.0);
            egui::Grid::new("dpi").num_columns(4).spacing(Vec2::new(10.0, 6.0)).show(ui, |ui| {
                ui.label(RichText::new("enabled").small().color(MUTED));
                ui.label(RichText::new("active").small().color(MUTED));
                ui.label(RichText::new("stage").small().color(MUTED));
                ui.label(RichText::new("DPI").small().color(MUTED));
                ui.end_row();
                for i in 0..DPI_STAGES {
                    let bit = 1u8 << i;
                    let mut on = self.dpi.enabled_mask & bit != 0;
                    if ui.checkbox(&mut on, "").changed() {
                        if on {
                            self.dpi.enabled_mask |= bit;
                        } else if self.dpi.enabled_mask & !bit & 0x7F != 0 {
                            self.dpi.enabled_mask &= !bit;
                            if self.dpi.active as usize == i {
                                self.dpi.active = self.dpi.enabled_mask.trailing_zeros() as u8;
                            }
                        }
                    }
                    ui.add_enabled_ui(on, |ui| ui.radio_value(&mut self.dpi.active, i as u8, ""));
                    ui.label(RichText::new(format!("{}", i + 1)).color(if on { TEXT } else { MUTED }));
                    ui.add_enabled_ui(on, |ui| {
                        let v = &mut self.dpi.stages[i];
                        egui::ComboBox::from_id_salt(("dpi-stage", i))
                            .selected_text(RichText::new(format!("{v} DPI")).monospace())
                            .width(150.0)
                            .height(280.0)
                            .show_ui(ui, |ui| {
                                for d in (p::PAW3311_DPI_MIN..=p::PAW3311_DPI_MAX).step_by(p::PAW3311_DPI_STEP as usize) {
                                    ui.selectable_value(v, d, RichText::new(format!("{d}")).monospace());
                                }
                            });
                    });
                    ui.end_row();
                }
            });
            ui.add_space(6.0);
            ui.horizontal(|ui| {
                let enabled = self.ready() && !self.busy && sensor_ok == Some(true);
                if ui.add_enabled(enabled, primary("Apply DPI")).clicked() {
                    match self.dpi.validate() {
                        Ok(()) => self.send(Command::Dpi(self.dpi)),
                        Err(e) => self.toast = Some((e.to_string(), BAD)),
                    }
                }
                if ui.button("Defaults").clicked() {
                    self.dpi = DpiConfig::default();
                }
                if sensor_ok == Some(false) {
                    ui.label(RichText::new("sensor is not a PAW3311: DPI disabled").small().color(WARN));
                }
            });
        })
    }

    fn confirm_modal(&mut self, ctx: &egui::Context) {
        let Some(hz) = self.confirm_polling else { return };
        let mut close = false;
        egui::Modal::new(egui::Id::unique("confirm_polling")).show(ctx, |ui| {
            ui.set_width(320.0);
            ui.label(RichText::new(format!("Apply {hz} Hz?")).heading());
            ui.add_space(4.0);
            ui.label(RichText::new("This value comes from the official software's code, but it has not been tested on your mouse yet.").color(MUTED));
            ui.add_space(10.0);
            ui.horizontal(|ui| {
                if ui.add(primary("Apply")).clicked() {
                    self.send(Command::Polling(hz));
                    close = true;
                }
                if ui.button("Cancel").clicked() {
                    close = true;
                }
            });
        });
        if close {
            self.confirm_polling = None;
        }
    }
}

impl eframe::App for App {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        self.handle_events();
        let ctx = ui.ctx().clone();

        egui::Panel::top("header")
            .frame(Frame::NONE.fill(BG).inner_margin(Margin::symmetric(16, 10)))
            .show_separator_line(false)
            .show(ui, |ui| self.header(ui));

        egui::Panel::bottom("footer")
            .frame(Frame::NONE.fill(BG).inner_margin(Margin::symmetric(16, 8)))
            .show_separator_line(false)
            .show(ui, |ui| {
                ui.horizontal(|ui| {
                    let label = if self.show_log { "Hide log" } else { "Show log" };
                    if ui.add(egui::Button::new(RichText::new(label).color(MUTED)).frame(false)).clicked() {
                        self.show_log = !self.show_log;
                    }
                    if self.busy {
                        ui.add(egui::Spinner::new().size(14.0));
                        ui.label(RichText::new("sending…").color(MUTED));
                    } else if let Some((t, c)) = &self.toast {
                        ui.label(RichText::new(t).color(*c));
                    }
                });
                if self.show_log {
                    Frame::NONE.fill(WELL).stroke(Stroke::new(1.0, BORDER))
                        .corner_radius(CornerRadius::same(6)).inner_margin(Margin::same(8))
                        .show(ui, |ui| {
                            egui::ScrollArea::vertical().max_height(96.0).stick_to_bottom(true).show(ui, |ui| {
                                ui.set_width(ui.available_width());
                                for l in &self.log {
                                    ui.label(RichText::new(l).monospace().color(MUTED));
                                }
                            });
                        });
                }
            });

        egui::CentralPanel::default()
            .frame(Frame::NONE.fill(BG).inner_margin(Margin::symmetric(16, 4)))
            .show(ui, |ui| {
                // Both columns get the same height: the larger of the free space and
                // what either column needs, so the cards always end on the same line.
                // A few px of slack so rounding never triggers the scroll bar.
                let col_h = (ui.available_height() - 8.0).max(self.natural.0).max(self.natural.1);
                // add_space() is followed by the regular item spacing before the next widget.
                let gap = COL_GAP + ui.spacing().item_spacing.y;
                egui::ScrollArea::vertical().show(ui, |ui| {
                    ui.columns_const(|[left, right]| {
                        let status_h = self.status_card(left);
                        left.add_space(COL_GAP);
                        let poll_nat = self.polling_card(left, col_h - status_h - gap);
                        let dpi_nat = self.dpi_card(right, col_h);
                        self.natural = (status_h + gap + poll_nat, dpi_nat);
                    });
                });
            });

        self.confirm_modal(&ctx);
    }

    fn on_exit(&mut self, _gl: Option<&eframe::glow::Context>) {
        let _ = self.tx.send(Request::Shutdown);
    }
}

fn time_hms() -> String {
    // local time via libc (no extra dependencies)
    let mut t: libc::time_t = 0;
    let mut tm: libc::tm = unsafe { std::mem::zeroed() };
    // SAFETY: valid pointers for time/localtime_r.
    unsafe {
        libc::time(&mut t);
        libc::localtime_r(&t, &mut tm);
    }
    format!("{:02}:{:02}:{:02}", tm.tm_hour, tm.tm_min, tm.tm_sec)
}
