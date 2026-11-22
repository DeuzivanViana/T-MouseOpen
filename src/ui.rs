//! User interface (egui): dark monochrome theme, card layout with stat tiles,
//! a segmented polling-rate control and a DPI stage list.

use crate::protocol::{self as p, Command, DpiConfig, Status, DPI_STAGES};
use crate::state::Saved;
use crate::worker::{Event, Request, Worker};
use eframe::egui::{self, pos2, vec2, Align2, Color32, CornerRadius, FontId, Frame, Margin, Rect, RichText, Shadow, Stroke, StrokeKind, Vec2};
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
const FAINT: Color32 = Color32::from_rgb(0x55, 0x55, 0x55);
const ACCENT: Color32 = Color32::from_rgb(0xed, 0xed, 0xed);
const ON_ACCENT: Color32 = Color32::from_rgb(0x05, 0x05, 0x05);
const OK: Color32 = Color32::from_rgb(0x4a, 0xb8, 0x86);
const WARN: Color32 = Color32::from_rgb(0xd9, 0xa4, 0x41);
const BAD: Color32 = Color32::from_rgb(0xe0, 0x5a, 0x52);
const GAP: f32 = 12.0;
const RADIUS: u8 = 12;

pub fn apply_theme(ctx: &egui::Context) {
    let mut v = egui::Visuals::dark();
    v.panel_fill = BG;
    v.window_fill = CARD;
    v.window_stroke = Stroke::new(1.0, BORDER_HI);
    v.window_corner_radius = CornerRadius::same(14);
    v.window_shadow = Shadow { offset: [0, 12], blur: 32, spread: 0, color: Color32::from_black_alpha(200) };
    v.popup_shadow = Shadow { offset: [0, 6], blur: 18, spread: 0, color: Color32::from_black_alpha(180) };
    v.menu_corner_radius = CornerRadius::same(10);
    v.extreme_bg_color = WELL;
    v.faint_bg_color = CARD_HI;
    v.selection.bg_fill = Color32::from_rgb(0xbd, 0xbd, 0xbd);
    v.selection.stroke = Stroke::new(1.0, ON_ACCENT);
    v.hyperlink_color = ACCENT;
    v.override_text_color = Some(TEXT);
    for w in [&mut v.widgets.noninteractive, &mut v.widgets.inactive, &mut v.widgets.hovered, &mut v.widgets.active, &mut v.widgets.open] {
        w.corner_radius = CornerRadius::same(8);
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
    v.widgets.open.bg_fill = HOVER;
    v.widgets.open.weak_bg_fill = HOVER;
    v.widgets.open.bg_stroke = Stroke::new(1.0, BORDER_HI);
    ctx.set_visuals(v);
    ctx.global_style_mut(|s| {
        s.spacing.item_spacing = Vec2::new(8.0, 6.0);
        s.spacing.button_padding = Vec2::new(14.0, 6.0);
        s.spacing.interact_size.y = 28.0;
        s.spacing.slider_rail_height = 4.0;
        use egui::TextStyle::*;
        s.text_styles.insert(Body, FontId::proportional(13.5));
        s.text_styles.insert(Button, FontId::proportional(13.5));
        s.text_styles.insert(Small, FontId::proportional(11.5));
        s.text_styles.insert(Heading, FontId::proportional(18.0));
        s.text_styles.insert(Monospace, FontId::monospace(12.5));
    });
}

// ------------------------------------------------------------------ building blocks

/// Card frame: inner margin + border = 2 * 16 + 2.
const CARD_CHROME: f32 = 34.0;

fn card_frame() -> Frame {
    Frame::NONE
        .fill(CARD)
        .stroke(Stroke::new(1.0, BORDER))
        .corner_radius(CornerRadius::same(RADIUS))
        .inner_margin(Margin::same(16))
}

/// Card title row: bold title, optional muted hint on the right.
fn card_title(ui: &mut egui::Ui, title: &str, hint: Option<&str>) {
    ui.horizontal(|ui| {
        ui.label(RichText::new(title).size(14.5).strong().color(TEXT));
        if let Some(h) = hint {
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                pill_label(ui, h, MUTED);
            });
        }
    });
}

/// Draws a card at least `min_height` tall (outer). Returns its natural outer height,
/// so columns can be equalized on the next frame.
fn card(ui: &mut egui::Ui, title: &str, hint: Option<&str>, min_height: f32, add: impl FnOnce(&mut egui::Ui)) -> f32 {
    let mut natural = 0.0;
    card_frame().show(ui, |ui| {
        ui.set_width(ui.available_width());
        card_title(ui, title, hint);
        ui.add_space(8.0);
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

/// Small rounded chip with text.
fn pill_label(ui: &mut egui::Ui, text: &str, color: Color32) {
    Frame::NONE
        .fill(CARD_HI)
        .stroke(Stroke::new(1.0, BORDER))
        .corner_radius(CornerRadius::same(10))
        .inner_margin(Margin::symmetric(8, 2))
        .show(ui, |ui| {
            ui.label(RichText::new(text).size(11.0).color(color));
        });
}

/// Key/value row with a hairline under it.
fn kv(ui: &mut egui::Ui, k: &str, v: impl Into<RichText>) {
    ui.horizontal(|ui| {
        ui.set_min_height(26.0);
        ui.label(RichText::new(k).color(MUTED));
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| ui.label(v.into()));
    });
    let r = ui.min_rect();
    let y = ui.cursor().top() - 2.0;
    ui.painter().line_segment([pos2(r.left(), y), pos2(r.right(), y)], Stroke::new(1.0, BORDER));
}

fn status_dot(ui: &mut egui::Ui, col: Color32) {
    let (r, _) = ui.allocate_exact_size(Vec2::splat(10.0), egui::Sense::hover());
    ui.painter().circle_filled(r.center(), 5.0, col.gamma_multiply(0.25));
    ui.painter().circle_filled(r.center(), 3.2, col);
}

/// Phone-style battery: rounded body, terminal nub, level fill and a bolt while charging.
fn battery_icon(ui: &mut egui::Ui, pct: u8, charging: bool, level: Color32) {
    use egui::Shape;
    let (rect, _) = ui.allocate_exact_size(vec2(40.0, 19.0), egui::Sense::hover());
    let painter = ui.painter();
    let body = Rect::from_min_size(rect.min, vec2(36.0, 19.0));
    let nub = Rect::from_center_size(pos2(body.right() + 2.2, body.center().y), vec2(3.0, 7.0));
    painter.rect_stroke(body, CornerRadius::same(5), Stroke::new(1.4, MUTED), StrokeKind::Inside);
    painter.rect_filled(nub, CornerRadius { nw: 0, sw: 0, ne: 2, se: 2 }, MUTED);
    let inner = body.shrink(2.8);
    let w = inner.width() * (pct.min(100) as f32 / 100.0);
    if w > 0.5 {
        let fill = Rect::from_min_size(inner.min, vec2(w.max(2.0), inner.height()));
        painter.rect_filled(fill, CornerRadius::same(3), level);
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

/// App mark: accent tile with a mouse outline.
fn logo(ui: &mut egui::Ui) {
    let (r, _) = ui.allocate_exact_size(Vec2::splat(34.0), egui::Sense::hover());
    let p = ui.painter();
    p.rect_filled(r, CornerRadius::same(9), ACCENT);
    let body = Rect::from_center_size(r.center() + vec2(0.0, 1.0), vec2(14.0, 21.0));
    p.rect_stroke(body, CornerRadius::same(7), Stroke::new(2.0, ON_ACCENT), StrokeKind::Middle);
    p.line_segment([pos2(body.center().x, body.top() + 4.0), pos2(body.center().x, body.top() + 8.5)], Stroke::new(2.0, ON_ACCENT));
}

/// On/off switch. Returns true when toggled.
fn toggle(ui: &mut egui::Ui, on: &mut bool) -> bool {
    let size = vec2(34.0, 18.0);
    let (rect, mut resp) = ui.allocate_exact_size(size, egui::Sense::click());
    if resp.clicked() {
        *on = !*on;
        resp.mark_changed();
    }
    let t = ui.ctx().animate_bool_responsive(resp.id, *on);
    let track = CARD_HI.lerp_to_gamma(ACCENT, t);
    let stroke = if resp.hovered() { BORDER_HI } else { BORDER };
    let p = ui.painter();
    p.rect_filled(rect, CornerRadius::same(9), track);
    p.rect_stroke(rect, CornerRadius::same(9), Stroke::new(1.0, stroke.lerp_to_gamma(ACCENT, t)), StrokeKind::Inside);
    let x = egui::lerp((rect.left() + 9.0)..=(rect.right() - 9.0), t);
    p.circle_filled(pos2(x, rect.center().y), 6.0, MUTED.lerp_to_gamma(ON_ACCENT, t));
    resp.changed()
}

/// Round stage number; filled when it is the active stage. Clickable when `enabled`.
fn stage_badge(ui: &mut egui::Ui, n: usize, active: bool, enabled: bool) -> egui::Response {
    let sense = if enabled { egui::Sense::click() } else { egui::Sense::hover() };
    let (rect, resp) = ui.allocate_exact_size(Vec2::splat(26.0), sense);
    let p = ui.painter();
    let c = rect.center();
    if active {
        p.circle_filled(c, 13.0, ACCENT);
        p.text(c, Align2::CENTER_CENTER, n.to_string(), FontId::proportional(13.0), ON_ACCENT);
    } else {
        let border = if enabled && resp.hovered() { MUTED } else { BORDER_HI };
        p.circle_filled(c, 13.0, if enabled && resp.hovered() { HOVER } else { CARD_HI });
        p.circle_stroke(c, 12.5, Stroke::new(1.0, border));
        p.text(c, Align2::CENTER_CENTER, n.to_string(), FontId::proportional(13.0), if enabled { TEXT } else { FAINT });
    }
    if enabled && !active {
        resp.on_hover_text("Make this the active stage")
    } else {
        resp
    }
}

/// Thin horizontal meter (0..=1).
fn meter(ui: &mut egui::Ui, width: f32, frac: f32, col: Color32) {
    let (rect, _) = ui.allocate_exact_size(vec2(width, 6.0), egui::Sense::hover());
    let p = ui.painter();
    p.rect_filled(rect, CornerRadius::same(3), CARD_HI);
    let w = (rect.width() * frac.clamp(0.0, 1.0)).max(6.0);
    p.rect_filled(Rect::from_min_size(rect.min, vec2(w, rect.height())), CornerRadius::same(3), col);
}

fn primary(text: &str) -> egui::Button<'_> {
    egui::Button::new(RichText::new(text).color(ON_ACCENT).strong())
        .fill(ACCENT)
        .corner_radius(CornerRadius::same(8))
        .min_size(Vec2::new(96.0, 32.0))
}

fn secondary(text: &str) -> egui::Button<'_> {
    egui::Button::new(RichText::new(text).color(TEXT))
        .fill(CARD_HI)
        .stroke(Stroke::new(1.0, BORDER))
        .corner_radius(CornerRadius::same(8))
        .min_size(Vec2::new(0.0, 32.0))
}

/// Stat tile: caption, big value with unit, and a sub line.
fn tile(ui: &mut egui::Ui, caption: &str, value: &str, unit: &str, add_sub: impl FnOnce(&mut egui::Ui)) {
    card_frame().inner_margin(Margin::symmetric(16, 14)).show(ui, |ui| {
        ui.set_width(ui.available_width());
        ui.label(RichText::new(caption.to_uppercase()).size(10.5).color(MUTED).strong());
        ui.add_space(2.0);
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 5.0;
            ui.label(RichText::new(value).size(30.0).strong().color(TEXT));
            if !unit.is_empty() {
                ui.label(RichText::new(unit).size(13.0).color(MUTED));
            }
        });
        ui.horizontal(|ui| {
            ui.set_min_height(20.0);
            add_sub(ui);
        });
    });
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
    /// Height of the stat-tile row on the previous frame.
    tiles_h: f32,
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
            tiles_h: 0.0,
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
            logo(ui);
            ui.add_space(4.0);
            ui.vertical(|ui| {
                ui.spacing_mut().item_spacing.y = 1.0;
                ui.label(RichText::new("TMouseOpen").heading().strong());
                ui.label(RichText::new("Titorion YDX-GM009  ·  2.4G receiver 04f3:026f").small().color(MUTED));
            });
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                let (txt, col) = match &self.conn {
                    Conn::Connected { name, .. } if self.status.is_some() => (format!("Connected · {name}"), OK),
                    Conn::Connected { name, .. } => (format!("Waiting for the mouse · {name}"), WARN),
                    Conn::Searching(m) => (m.clone(), BAD),
                };
                Frame::NONE
                    .fill(col.gamma_multiply(0.08))
                    .stroke(Stroke::new(1.0, col.gamma_multiply(0.35)))
                    .corner_radius(CornerRadius::same(14))
                    .inner_margin(Margin::symmetric(12, 5))
                    .show(ui, |ui| {
                        ui.horizontal(|ui| {
                            status_dot(ui, col);
                            ui.label(RichText::new(txt).size(12.0).color(col));
                        });
                    });
            });
        });
    }

    /// Top row: polling, DPI, battery and sensor at a glance.
    fn tiles(&mut self, ui: &mut egui::Ui) {
        let Some(st) = self.status else {
            card_frame().show(ui, |ui| {
                ui.set_width(ui.available_width());
                ui.horizontal(|ui| {
                    let searching = matches!(self.conn, Conn::Searching(_));
                    let col = if searching { BAD } else { WARN };
                    ui.add(egui::Spinner::new().size(18.0).color(col));
                    ui.add_space(4.0);
                    ui.vertical(|ui| {
                        if searching {
                            ui.label(RichText::new("No receiver connected").color(BAD));
                            ui.label(RichText::new("Plug in the 2.4G receiver. If it is plugged in, check the permissions (udev rule or sudo -E).").small().color(MUTED));
                        } else {
                            ui.label(RichText::new("Waiting for the first status report from the mouse (up to ~5 s).").color(WARN));
                            ui.label(RichText::new("If it takes longer, move the mouse to wake it up.").small().color(MUTED));
                        }
                    });
                    if let Some(hz) = self.saved.polling_hz {
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            pill_label(ui, &format!("Last applied polling rate · {hz} Hz"), MUTED);
                        });
                    }
                });
            });
            return;
        };
        ui.spacing_mut().item_spacing.x = GAP;
        ui.columns_const(|[c1, c2, c3, c4]| {
            let hz = st.polling_hz().map(|h| h.to_string()).unwrap_or_else(|| format!("0x{:02x}", st.polling_code()));
            tile(c1, "Polling rate", &hz, "Hz", |ui| {
                let max = st.max_polling().map(|h| format!("receiver max {h} Hz")).unwrap_or_else(|| "receiver max ?".into());
                ui.label(RichText::new(max).small().color(MUTED));
            });
            let stage = st.dpi_stage();
            let dpi = self.saved.dpi.map(|d| d.stages[stage.min(6) as usize]);
            let dpi_txt = dpi.map(|d| d.to_string()).unwrap_or_else(|| "—".into());
            tile(c2, "DPI", &dpi_txt, "DPI", |ui| {
                ui.spacing_mut().item_spacing.x = 3.0;
                for i in 0..DPI_STAGES as u8 {
                    let (r, _) = ui.allocate_exact_size(vec2(8.0, 8.0), egui::Sense::hover());
                    let on = self.saved.dpi.map(|d| d.enabled_mask & (1 << i) != 0).unwrap_or(true);
                    let col = if i == stage { ACCENT } else if on { BORDER_HI } else { CARD_HI };
                    ui.painter().circle_filled(r.center(), 3.5, col);
                }
                ui.add_space(4.0);
                ui.label(RichText::new(format!("stage {}", stage + 1)).small().color(MUTED));
            });
            let b = st.battery();
            let col = if b <= 15 { BAD } else if b <= 35 { WARN } else { OK };
            tile(c3, "Battery", &b.to_string(), "%", |ui| {
                battery_icon(ui, b, st.charge_state() == 1, col);
                ui.label(RichText::new(st.charge_text()).small().color(MUTED));
            });
            let fw = match &self.conn {
                Conn::Connected { version, .. } => format!("firmware {version}"),
                _ => "firmware ?".into(),
            };
            tile(c4, "Sensor", &st.sensor_name(), "", |ui| {
                ui.label(RichText::new(fw).small().color(MUTED));
            });
        });
    }

    fn polling_card(&mut self, ui: &mut egui::Ui, min_height: f32) -> f32 {
        card(ui, "Polling rate", None, min_height, |ui| {
            ui.label(RichText::new("How often the mouse reports its position to the PC.").small().color(MUTED));
            ui.add_space(6.0);
            // Segmented control.
            Frame::NONE
                .fill(WELL)
                .stroke(Stroke::new(1.0, BORDER))
                .corner_radius(CornerRadius::same(10))
                .inner_margin(Margin::same(3))
                .show(ui, |ui| {
                    ui.spacing_mut().item_spacing.x = 3.0;
                    let n = p::POLLING.len() as f32;
                    let w = ((ui.available_width() - 3.0 * (n - 1.0)) / n).floor();
                    ui.horizontal(|ui| {
                        for (hz, _) in p::POLLING {
                            let tested = p::POLLING_TESTED.contains(&hz);
                            let label = if tested { format!("{hz}") } else { format!("{hz}*") };
                            let sel = self.polling_choice == hz;
                            let btn = egui::Button::new(RichText::new(label).strong().color(if sel { ON_ACCENT } else { MUTED }))
                                .fill(if sel { ACCENT } else { Color32::TRANSPARENT })
                                .stroke(Stroke::NONE)
                                .corner_radius(CornerRadius::same(8))
                                .min_size(Vec2::new(w, 32.0));
                            if ui.add(btn).clicked() {
                                self.polling_choice = hz;
                            }
                        }
                    });
                });
            ui.add_space(8.0);
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

    fn device_card(&mut self, ui: &mut egui::Ui, min_height: f32) -> f32 {
        card(ui, "Device", None, min_height, |ui| {
            let link = match &self.conn {
                Conn::Connected { name, .. } => RichText::new(name.as_str()),
                Conn::Searching(_) => RichText::new("not connected").color(BAD),
            };
            kv(ui, "Receiver", link);
            match self.status {
                Some(st) => {
                    kv(ui, "Sensor", st.sensor_name());
                    if let Conn::Connected { version, .. } = &self.conn {
                        kv(ui, "Firmware (likely)", version.as_str());
                    }
                    kv(ui, "Receiver max rate", st.max_polling().map(|h| format!("{h} Hz")).unwrap_or_else(|| "?".into()));
                }
                None => {
                    kv(ui, "Sensor", RichText::new("—").color(MUTED));
                    let hz = self.saved.polling_hz.map(|h| format!("{h} Hz")).unwrap_or_else(|| "—".into());
                    kv(ui, "Last applied polling rate", RichText::new(hz).color(MUTED));
                }
            }
        })
    }

    fn dpi_card(&mut self, ui: &mut egui::Ui, min_height: f32) -> f32 {
        let sensor_ok = self.status.map(|s| s.sensor_code() == p::PAW3311);
        card(ui, "DPI stages", Some("PAW3311"), min_height, |ui| {
            ui.label(RichText::new("The mouse DPI button only cycles through enabled stages.").small().color(MUTED));
            ui.add_space(6.0);
            egui::Grid::new("dpi").num_columns(4).spacing(Vec2::new(14.0, 7.0)).show(ui, |ui| {
                ui.label(RichText::new("ON").size(10.5).strong().color(MUTED));
                ui.label(RichText::new("STAGE").size(10.5).strong().color(MUTED));
                ui.label(RichText::new("DPI").size(10.5).strong().color(MUTED));
                ui.label("");
                ui.end_row();
                for i in 0..DPI_STAGES {
                    let bit = 1u8 << i;
                    let mut on = self.dpi.enabled_mask & bit != 0;
                    if toggle(ui, &mut on) {
                        if on {
                            self.dpi.enabled_mask |= bit;
                        } else if self.dpi.enabled_mask & !bit & 0x7F != 0 {
                            self.dpi.enabled_mask &= !bit;
                            if self.dpi.active as usize == i {
                                self.dpi.active = self.dpi.enabled_mask.trailing_zeros() as u8;
                            }
                        }
                    }
                    let on = self.dpi.enabled_mask & bit != 0;
                    let active = self.dpi.active as usize == i;
                    if stage_badge(ui, i + 1, active, on).clicked() && on {
                        self.dpi.active = i as u8;
                    }
                    ui.add_enabled_ui(on, |ui| {
                        let v = &mut self.dpi.stages[i];
                        egui::ComboBox::from_id_salt(("dpi-stage", i))
                            .selected_text(RichText::new(format!("{v} DPI")).monospace())
                            .width(130.0)
                            .height(280.0)
                            .show_ui(ui, |ui| {
                                for d in (p::PAW3311_DPI_MIN..=p::PAW3311_DPI_MAX).step_by(p::PAW3311_DPI_STEP as usize) {
                                    ui.selectable_value(v, d, RichText::new(format!("{d}")).monospace());
                                }
                            });
                    });
                    let frac = self.dpi.stages[i] as f32 / p::PAW3311_DPI_MAX as f32;
                    let col = if !on { CARD_HI } else if active { ACCENT } else { FAINT };
                    let w = ui.available_width().clamp(40.0, 160.0);
                    meter(ui, w, frac, col);
                    ui.end_row();
                }
            });
            ui.add_space(10.0);
            ui.horizontal(|ui| {
                let enabled = self.ready() && !self.busy && sensor_ok == Some(true);
                if ui.add_enabled(enabled, primary("Apply DPI")).clicked() {
                    match self.dpi.validate() {
                        Ok(()) => self.send(Command::Dpi(self.dpi)),
                        Err(e) => self.toast = Some((e.to_string(), BAD)),
                    }
                }
                if ui.add(secondary("Defaults")).clicked() {
                    self.dpi = DpiConfig::default();
                }
                if sensor_ok == Some(false) {
                    ui.label(RichText::new("sensor is not a PAW3311: DPI disabled").small().color(WARN));
                }
            });
        })
    }

    fn footer(&mut self, ui: &mut egui::Ui) {
        let r = ui.max_rect();
        ui.painter().line_segment([pos2(r.left() - 20.0, r.top() - 8.0), pos2(r.right() + 20.0, r.top() - 8.0)], Stroke::new(1.0, BORDER));
        ui.horizontal(|ui| {
            let label = if self.show_log { "Hide log" } else { "Show log" };
            if ui.add(egui::Button::new(RichText::new(label).color(MUTED)).fill(CARD_HI).stroke(Stroke::new(1.0, BORDER)).corner_radius(CornerRadius::same(8))).clicked() {
                self.show_log = !self.show_log;
            }
            ui.label(RichText::new(format!("{} entries", self.log.len())).small().color(FAINT));
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if self.busy {
                    ui.label(RichText::new("sending…").color(MUTED));
                    ui.add(egui::Spinner::new().size(14.0));
                } else if let Some((t, c)) = &self.toast {
                    Frame::NONE
                        .fill(c.gamma_multiply(0.08))
                        .stroke(Stroke::new(1.0, c.gamma_multiply(0.35)))
                        .corner_radius(CornerRadius::same(14))
                        .inner_margin(Margin::symmetric(12, 4))
                        .show(ui, |ui| {
                            ui.horizontal(|ui| {
                                status_dot(ui, *c);
                                ui.label(RichText::new(t).color(*c));
                            });
                        });
                }
            });
        });
        if self.show_log {
            ui.add_space(4.0);
            Frame::NONE.fill(WELL).stroke(Stroke::new(1.0, BORDER))
                .corner_radius(CornerRadius::same(10)).inner_margin(Margin::same(10))
                .show(ui, |ui| {
                    egui::ScrollArea::vertical().max_height(110.0).stick_to_bottom(true).show(ui, |ui| {
                        ui.set_width(ui.available_width());
                        for l in &self.log {
                            ui.label(RichText::new(l).monospace().color(MUTED));
                        }
                    });
                });
        }
    }

    fn confirm_modal(&mut self, ctx: &egui::Context) {
        let Some(hz) = self.confirm_polling else { return };
        let mut close = false;
        egui::Modal::new(egui::Id::unique("confirm_polling"))
            .frame(card_frame().inner_margin(Margin::same(22)).stroke(Stroke::new(1.0, BORDER_HI)).corner_radius(CornerRadius::same(16)))
            .show(ctx, |ui| {
                ui.set_width(340.0);
                ui.horizontal(|ui| {
                    let (r, _) = ui.allocate_exact_size(Vec2::splat(30.0), egui::Sense::hover());
                    ui.painter().circle_filled(r.center(), 15.0, WARN.gamma_multiply(0.15));
                    ui.painter().text(r.center(), Align2::CENTER_CENTER, "!", FontId::proportional(17.0), WARN);
                    ui.label(RichText::new(format!("Apply {hz} Hz?")).heading().strong());
                });
                ui.add_space(8.0);
                ui.label(RichText::new("This value comes from the official software's code, but it has not been tested on your mouse yet.").color(MUTED));
                ui.add_space(16.0);
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if ui.add(primary("Apply")).clicked() {
                        self.send(Command::Polling(hz));
                        close = true;
                    }
                    if ui.add(secondary("Cancel")).clicked() {
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
            .frame(Frame::NONE.fill(BG).inner_margin(Margin { left: 20, right: 20, top: 16, bottom: 12 }))
            .show_separator_line(false)
            .show(ui, |ui| self.header(ui));

        egui::Panel::bottom("footer")
            .frame(Frame::NONE.fill(BG).inner_margin(Margin { left: 20, right: 20, top: 10, bottom: 10 }))
            .show_separator_line(false)
            .show(ui, |ui| self.footer(ui));

        egui::CentralPanel::default()
            .frame(Frame::NONE.fill(BG).inner_margin(Margin { left: 20, right: 20, top: 4, bottom: 10 }))
            .show(ui, |ui| {
                // Both columns get the same height: the larger of the free space (minus the
                // tiles row measured last frame) and what either column needs, so the cards
                // always end on the same line. A few px of slack so rounding never triggers the scroll bar.
                let col_h = (ui.available_height() - self.tiles_h - GAP - 14.0).max(self.natural.0).max(self.natural.1);
                egui::ScrollArea::vertical().show(ui, |ui| {
                    let top = ui.cursor().top();
                    self.tiles(ui);
                    self.tiles_h = ui.cursor().top() - top - ui.spacing().item_spacing.y;
                    ui.add_space(GAP - ui.spacing().item_spacing.y);
                    // add_space() is followed by the regular item spacing before the next widget.
                    let gap = GAP;
                    ui.spacing_mut().item_spacing.x = GAP;
                    ui.columns_const(|[left, right]| {
                        let poll_h = self.polling_card(left, 0.0);
                        left.add_space(gap - left.spacing().item_spacing.y);
                        let dev_nat = self.device_card(left, col_h - poll_h - gap);
                        let dpi_nat = self.dpi_card(right, col_h);
                        self.natural = (poll_h + gap + dev_nat, dpi_nat);
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
