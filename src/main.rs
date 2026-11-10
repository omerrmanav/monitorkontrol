mod ddc;
mod tray;

use ddc::{CachedMonitorData, DdcCommand, DdcController, DdcEvent};
use eframe::egui::{
    self, Color32, CursorIcon, FontId, Pos2, Rect, RichText, Rounding, Sense, Stroke, Vec2,
};
use std::env;

struct MinimalMonitorApp {
    controller: DdcController,
    data: CachedMonitorData,
    is_busy: bool,
    status_text: String,
}

impl MinimalMonitorApp {
    fn new(cc: &eframe::CreationContext<'_>) -> Self {
        setup_modern_theme(&cc.egui_ctx);

        let controller = DdcController::new(cc.egui_ctx.clone());
        // Load instant cached state in <0.02ms so opening the window has ZERO I2C lag!
        let data = CachedMonitorData::load_instant();

        Self {
            controller,
            data,
            is_busy: false,
            status_text: "Hazır".to_string(),
        }
    }

    fn poll_events(&mut self) {
        while let Ok(evt) = self.controller.evt_rx.try_recv() {
            match evt {
                DdcEvent::DataSynced(data) => {
                    self.data = data;
                }
                DdcEvent::BusyChanged(busy) => {
                    self.is_busy = busy;
                }
                DdcEvent::StatusText(txt) => {
                    self.status_text = txt;
                }
            }
        }
    }

    fn render_header_card(&mut self, ui: &mut egui::Ui) {
        egui::Frame::none()
            .fill(Color32::from_rgb(22, 27, 38))
            .stroke(Stroke::new(1.0_f32, Color32::from_rgb(38, 47, 66)))
            .rounding(Rounding::same(14.0))
            .inner_margin(egui::Margin::symmetric(18.0, 14.0))
            .show(ui, |ui| {
                ui.set_width(ui.available_width());
                ui.horizontal(|ui| {
                    ui.vertical(|ui| {
                        ui.label(
                            RichText::new(&self.data.model)
                                .font(FontId::proportional(20.0))
                                .strong()
                                .color(Color32::from_rgb(243, 246, 252)),
                        );
                        ui.add_space(2.0);
                        ui.label(
                            RichText::new(format!(
                                "{}  •  DDC/CI Hızlı Mod",
                                self.data.drm_connector
                            ))
                            .font(FontId::proportional(12.5))
                            .color(Color32::from_rgb(139, 152, 175)),
                        );
                    });

                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        let refresh_btn = egui::Button::new(
                            RichText::new("↻")
                                .font(FontId::proportional(16.0))
                                .color(Color32::from_rgb(180, 195, 220)),
                        )
                        .fill(Color32::from_rgb(32, 39, 54))
                        .rounding(Rounding::same(8.0))
                        .min_size(Vec2::new(32.0, 32.0));

                        if ui
                            .add(refresh_btn)
                            .on_hover_text("Monitörden donanımsal değerleri yeniden oku")
                            .clicked()
                        {
                            self.controller.send(DdcCommand::SyncFromHardware);
                        }

                        ui.add_space(6.0);

                        // Hz Pill Badge
                        egui::Frame::none()
                            .fill(Color32::from_rgb(16, 62, 56))
                            .stroke(Stroke::new(1.0_f32, Color32::from_rgb(32, 135, 112)))
                            .rounding(Rounding::same(20.0))
                            .inner_margin(egui::Margin::symmetric(12.0, 6.0))
                            .show(ui, |ui| {
                                ui.label(
                                    RichText::new(format!("⚡ {}", self.data.vertical_freq))
                                        .font(FontId::proportional(13.5))
                                        .strong()
                                        .color(Color32::from_rgb(110, 245, 205)),
                                );
                            });
                    });
                });
            });
    }

    fn render_slider_card(
        &mut self,
        ui: &mut egui::Ui,
        icon: &str,
        title: &str,
        vcp_code: u8,
        value: &mut u16,
        accent_color: Color32,
        presets: &[u16],
    ) {
        let mut commit_to_hardware = false;

        egui::Frame::none()
            .fill(Color32::from_rgb(22, 27, 38))
            .stroke(Stroke::new(1.0_f32, Color32::from_rgb(38, 47, 66)))
            .rounding(Rounding::same(14.0))
            .inner_margin(egui::Margin::symmetric(18.0, 14.0))
            .show(ui, |ui| {
                ui.set_width(ui.available_width());

                // Header Row
                ui.horizontal(|ui| {
                    ui.label(
                        RichText::new(format!("{}  {}", icon, title))
                            .font(FontId::proportional(16.0))
                            .strong()
                            .color(Color32::from_rgb(235, 240, 250)),
                    );

                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        ui.label(
                            RichText::new(format!("%{}", *value))
                                .font(FontId::proportional(18.5))
                                .strong()
                                .color(accent_color),
                        );
                    });
                });

                ui.add_space(10.0);

                // Custom Modern Pill Slider (updates visual value at 164 FPS, commits to I2C on release/click)
                if draw_modern_slider(ui, value, 100, accent_color) {
                    commit_to_hardware = true;
                }

                ui.add_space(10.0);

                // Quick Preset Pills
                ui.columns(presets.len(), |cols| {
                    for (i, &preset) in presets.iter().enumerate() {
                        let is_active = *value == preset;
                        let fill = if is_active {
                            accent_color.gamma_multiply(0.28)
                        } else {
                            Color32::from_rgb(30, 37, 52)
                        };
                        let stroke = if is_active {
                            Stroke::new(1.2_f32, accent_color)
                        } else {
                            Stroke::new(1.0_f32, Color32::from_rgb(46, 56, 76))
                        };
                        let text_col = if is_active {
                            Color32::WHITE
                        } else {
                            Color32::from_rgb(175, 188, 210)
                        };

                        let btn = egui::Button::new(
                            RichText::new(format!("%{}", preset))
                                .font(FontId::proportional(13.0))
                                .strong()
                                .color(text_col),
                        )
                        .fill(fill)
                        .stroke(stroke)
                        .rounding(Rounding::same(8.0))
                        .min_size(Vec2::new(cols[i].available_width(), 28.0));

                        if cols[i].add(btn).clicked() {
                            *value = preset;
                            commit_to_hardware = true;
                        }
                    }
                });
            });

        if commit_to_hardware {
            self.controller
                .queue_vcp(self.data.i2c_bus, vcp_code, *value);
        }
    }
}

/// Updates `value` smoothly every frame while dragging, and returns `true` when the user
/// finishes dragging (`drag_stopped`) or clicks a point on the bar (`clicked`) so I2C never lags!
fn draw_modern_slider(
    ui: &mut egui::Ui,
    value: &mut u16,
    max_val: u16,
    accent: Color32,
) -> bool {
    let desired_size = Vec2::new(ui.available_width(), 26.0);
    let (rect, response) = ui.allocate_exact_size(desired_size, Sense::click_and_drag());

    if response.clicked() || response.dragged() {
        if let Some(pointer_pos) = response.interact_pointer_pos() {
            let ratio = ((pointer_pos.x - rect.left()) / rect.width()).clamp(0.0, 1.0);
            *value = (ratio * max_val as f32).round() as u16;
        }
    }

    if response.hovered() || response.dragged() {
        ui.ctx().set_cursor_icon(CursorIcon::PointingHand);
    }

    if ui.is_rect_visible(rect) {
        let painter = ui.painter();
        let track_height = 10.0;
        let track_rect = Rect::from_center_size(
            rect.center(),
            Vec2::new(rect.width(), track_height),
        );

        // Background track
        painter.rect_filled(
            track_rect,
            Rounding::same(5.0),
            Color32::from_rgb(35, 43, 60),
        );

        // Active filled portion
        let ratio = (*value as f32 / max_val.max(1) as f32).clamp(0.0, 1.0);
        let fill_width = (track_rect.width() * ratio).max(track_height);
        let fill_rect = Rect::from_min_size(
            track_rect.min,
            Vec2::new(fill_width.min(track_rect.width()), track_height),
        );
        painter.rect_filled(fill_rect, Rounding::same(5.0), accent);

        // Thumb circle
        let thumb_x = (track_rect.left() + track_rect.width() * ratio)
            .clamp(track_rect.left() + 8.0, track_rect.right() - 8.0);
        let thumb_center = Pos2::new(thumb_x, track_rect.center().y);
        let thumb_radius = if response.dragged() || response.hovered() {
            9.5
        } else {
            8.0
        };

        painter.circle_filled(thumb_center, thumb_radius + 2.5, accent.gamma_multiply(0.35));
        painter.circle_filled(thumb_center, thumb_radius, Color32::WHITE);
    }

    response.drag_stopped() || (response.clicked() && !response.dragged())
}

impl eframe::App for MinimalMonitorApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        self.poll_events();

        egui::CentralPanel::default()
            .frame(
                egui::Frame::none()
                    .fill(Color32::from_rgb(13, 17, 24))
                    .inner_margin(egui::Margin::same(16.0)),
            )
            .show(ctx, |ui| {
                // 1. Monitor Name & Hz Header
                self.render_header_card(ui);

                ui.add_space(12.0);

                // 2. Brightness Slider Card
                let mut b_val = self.data.brightness;
                self.render_slider_card(
                    ui,
                    "☀",
                    "Parlaklık",
                    0x10,
                    &mut b_val,
                    Color32::from_rgb(245, 175, 35),
                    &[25, 50, 75, 100],
                );
                self.data.brightness = b_val;

                ui.add_space(12.0);

                // 3. Contrast Slider Card
                let mut c_val = self.data.contrast;
                self.render_slider_card(
                    ui,
                    "◑",
                    "Kontrast",
                    0x12,
                    &mut c_val,
                    Color32::from_rgb(56, 189, 248),
                    &[50, 70, 85, 100],
                );
                self.data.contrast = c_val;

                ui.add_space(10.0);

                // 4. Minimal Footer
                ui.horizontal(|ui| {
                    if self.is_busy {
                        ui.spinner();
                        ui.label(
                            RichText::new("Uygulanıyor...")
                                .font(FontId::proportional(12.0))
                                .color(Color32::from_rgb(245, 190, 80)),
                        );
                    } else {
                        ui.label(
                            RichText::new(format!("● {}", self.status_text))
                                .font(FontId::proportional(12.0))
                                .color(Color32::from_rgb(95, 215, 145)),
                        );
                    }

                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        let hide_btn = egui::Button::new(
                            RichText::new("Üst Bar'a Küçült")
                                .font(FontId::proportional(12.0))
                                .color(Color32::from_rgb(165, 178, 200)),
                        )
                        .fill(Color32::from_rgb(24, 30, 42))
                        .rounding(Rounding::same(6.0));

                        if ui
                            .add(hide_btn)
                            .on_hover_text(
                                "Pencereyi kapatır; sağ üstteki Top Bar ikonuna tıkladığınızda anında tekrar açılır",
                            )
                            .clicked()
                        {
                            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                        }
                    });
                });
            });
    }
}

fn setup_modern_theme(ctx: &egui::Context) {
    let mut visuals = egui::Visuals::dark();
    visuals.window_rounding = Rounding::same(14.0);
    visuals.widgets.noninteractive.bg_fill = Color32::from_rgb(22, 27, 38);
    visuals.widgets.inactive.bg_fill = Color32::from_rgb(30, 37, 52);
    visuals.widgets.hovered.bg_fill = Color32::from_rgb(45, 56, 78);
    visuals.widgets.active.bg_fill = Color32::from_rgb(56, 130, 220);
    ctx.set_visuals(visuals);
}

fn run_gui_window() -> eframe::Result<()> {
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("Monitör Kontrol")
            .with_inner_size([400.0, 355.0])
            .with_min_inner_size([360.0, 340.0])
            .with_max_inner_size([480.0, 400.0])
            .with_resizable(false),
        ..Default::default()
    };

    eframe::run_native(
        "Monitör Kontrol",
        options,
        Box::new(|cc| Ok(Box::new(MinimalMonitorApp::new(cc)))),
    )
}

fn main() -> eframe::Result<()> {
    let args: Vec<String> = env::args().skip(1).collect();

    if let Some(first) = args.first().map(|s| s.as_str()) {
        match first {
            "--tray-daemon" => {
                let _ = tray::run_tray_daemon();
                return Ok(());
            }
            "--gui-window" => {
                return run_gui_window();
            }
            "--status" | "-s" => {
                let st = CachedMonitorData::load_instant();
                println!("Monitör   : {} ({})", st.model, st.vertical_freq);
                println!("Parlaklık : %{}", st.brightness);
                println!("Kontrast  : %{}", st.contrast);
                return Ok(());
            }
            "--detect" => {
                // Exit code 0: DDC/CI capable monitor found, 1: not supported / not found
                let st = ddc::read_hardware_state();
                if st.supported {
                    println!("Destekleniyor: {} (/dev/i2c-{})", st.model, st.i2c_bus);
                    std::process::exit(0);
                }
                println!("{}", st.model);
                std::process::exit(1);
            }
            "--sync" => {
                let st = ddc::read_hardware_state();
                println!("Monitör   : {} ({})", st.model, st.vertical_freq);
                println!("Parlaklık : %{}", st.brightness);
                println!("Kontrast  : %{}", st.contrast);
                return Ok(());
            }
            "--restore" | "-r" => {
                let st = CachedMonitorData::load_instant();
                if st.supported && st.i2c_bus > 0 {
                    ddc::set_vcp_fast(st.i2c_bus, 0x10, st.brightness);
                    ddc::set_vcp_fast(st.i2c_bus, 0x12, st.contrast);
                    println!("Kayıtlı değerler monitöre uygulandı: Parlaklık %{}, Kontrast %{}", st.brightness, st.contrast);
                }
                return Ok(());
            }
            "--brightness" | "-b" => {
                if let Some(v) = args.get(1).and_then(|s| s.parse::<u16>().ok()) {
                    let bus = CachedMonitorData::load_instant().i2c_bus;
                    ddc::set_vcp_fast(bus, 0x10, v.min(100));
                    println!("Parlaklık %{} yapıldı.", v.min(100));
                }
                return Ok(());
            }
            "--contrast" | "-c" => {
                if let Some(v) = args.get(1).and_then(|s| s.parse::<u16>().ok()) {
                    let bus = CachedMonitorData::load_instant().i2c_bus;
                    ddc::set_vcp_fast(bus, 0x12, v.min(100));
                    println!("Kontrast %{} yapıldı.", v.min(100));
                }
                return Ok(());
            }
            "--scale" | "--font-scale" => {
                if let Some(v) = args.get(1).and_then(|s| s.parse::<f64>().ok()) {
                    let _ = std::process::Command::new("gsettings")
                        .args(["set", "org.gnome.desktop.interface", "text-scaling-factor", &v.to_string()])
                        .status();
                    println!("Yazı / Font ölçeği (text-scaling-factor) {} yapıldı.", v);
                }
                return Ok(());
            }
            "--help" | "-h" => {
                println!("Monitör Kontrol (Sıfır Gecikmeli & Top Bar Destekli)");
                println!("  monitorkontrol                  Top Bar ikonunu başlatır ve kontrol penceresini açar");
                println!("  monitorkontrol --tray-daemon    Sadece Top Bar (Üst Bar) ikon servisini çalıştırır");
                println!("  monitorkontrol --status         Önbellekten anlık durumu (0ms) gösterir");
                println!("  monitorkontrol --detect         Monitörün DDC/CI desteğini sorgular (0: var, 1: yok)");
                println!("  monitorkontrol --sync           Donanımdan mevcut değerleri okur");
                println!("  monitorkontrol --brightness 75  Parlaklığı %75 yapar");
                println!("  monitorkontrol --contrast 80    Kontrastı %80 yapar");
                println!("  monitorkontrol --scale 1.25     Yazı / Font ölçeğini 1.25x yapar");
                return Ok(());
            }
            _ => {}
        }
    }

    // Ensure the Linux Top Bar icon daemon is running in the background
    tray::ensure_tray_daemon_running();

    // Open the sleek minimal control window
    run_gui_window()
}
