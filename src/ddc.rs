use serde::{Deserialize, Serialize};
use std::fs;
use std::process::Command;
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::{Arc, Mutex};
use std::thread;

const CACHE_FILE: &str = "/tmp/monitorkontrol_cache.json";

fn get_config_file_path() -> std::path::PathBuf {
    if let Ok(xdg) = std::env::var("XDG_CONFIG_HOME") {
        if !xdg.is_empty() {
            return std::path::PathBuf::from(xdg).join("monitorkontrol").join("config.json");
        }
    }
    if let Ok(home) = std::env::var("HOME") {
        return std::path::PathBuf::from(home).join(".config").join("monitorkontrol").join("config.json");
    }
    std::path::PathBuf::from(CACHE_FILE)
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CachedMonitorData {
    pub i2c_bus: u32,
    pub drm_connector: String,
    pub model: String,
    pub vertical_freq: String,
    pub brightness: u16,
    pub contrast: u16,
    /// `true` only after a monitor with DDC/CI brightness support was detected.
    #[serde(default)]
    pub supported: bool,
}

impl Default for CachedMonitorData {
    fn default() -> Self {
        Self {
            i2c_bus: 0,
            drm_connector: String::new(),
            model: "Monitör aranıyor...".to_string(),
            vertical_freq: "-".to_string(),
            brightness: 50,
            contrast: 50,
            supported: false,
        }
    }
}

/// Result of `ddcutil detect`: (i2c bus number, DRM connector, monitor model).
/// The serial number is intentionally never read or stored.
pub fn detect_monitor() -> Option<(u32, String, String)> {
    let output = Command::new("ddcutil")
        .args(["detect", "--brief", "--noudf"])
        .output()
        .ok()?;
    let text = String::from_utf8_lossy(&output.stdout);

    let mut bus: Option<u32> = None;
    let mut connector = String::new();
    let mut model = String::new();

    for line in text.lines() {
        let t = line.trim();
        if t.starts_with("Invalid display") && bus.is_some() {
            break;
        }
        if let Some(rest) = t.strip_prefix("I2C bus:") {
            if bus.is_some() {
                break; // only the first valid display is used
            }
            bus = rest.trim().rsplit('-').next().and_then(|n| n.parse().ok());
        } else if let Some(rest) = t.strip_prefix("DRM connector:") {
            connector = rest.trim().to_string();
        } else if let Some(rest) = t.strip_prefix("Monitor:") {
            // format: MFG:Model:Serial -> keep only Model
            model = rest.trim().split(':').nth(1).unwrap_or("").trim().to_string();
        }
    }

    let bus = bus?;
    if model.is_empty() {
        model = "Bilinmeyen monitör".to_string();
    }
    Some((bus, connector, model))
}

impl CachedMonitorData {
    /// Instant (<0.05ms) read from persistent config or cache.
    pub fn load_instant() -> Self {
        let cfg_path = get_config_file_path();
        if let Ok(content) = fs::read_to_string(&cfg_path) {
            if let Ok(data) = serde_json::from_str::<Self>(&content) {
                return data;
            }
        }
        if let Ok(content) = fs::read_to_string(CACHE_FILE) {
            if let Ok(data) = serde_json::from_str::<Self>(&content) {
                return data;
            }
        }
        let default_data = Self::default();
        default_data.save_instant();
        default_data
    }

    pub fn save_instant(&self) {
        if let Ok(json) = serde_json::to_string(self) {
            let cfg_path = get_config_file_path();
            if let Some(parent) = cfg_path.parent() {
                let _ = fs::create_dir_all(parent);
            }
            let _ = fs::write(&cfg_path, &json);
            let _ = fs::write(CACHE_FILE, &json);
        }
    }

    pub fn target_args(&self) -> Vec<String> {
        vec![
            "--bus".to_string(),
            self.i2c_bus.to_string(),
            "--noudf".to_string(),
            "--sleep-multiplier".to_string(),
            "0.02".to_string(),
        ]
    }
}

#[derive(Debug, Clone)]
pub enum DdcCommand {
    SyncFromHardware,
    WakePendingVcp,
}

#[derive(Debug, Clone)]
pub enum DdcEvent {
    DataSynced(CachedMonitorData),
    BusyChanged(bool),
    StatusText(String),
}

#[derive(Debug, Clone)]
struct PendingVcp {
    bus: u32,
    vcp_code: u8,
    value: u16,
}

pub struct DdcController {
    cmd_tx: Sender<DdcCommand>,
    pub evt_rx: Receiver<DdcEvent>,
    pending: Arc<Mutex<Vec<PendingVcp>>>,
}

impl DdcController {
    pub fn new(ctx: eframe::egui::Context) -> Self {
        let (cmd_tx, cmd_rx) = mpsc::channel::<DdcCommand>();
        let (evt_tx, evt_rx) = mpsc::channel::<DdcEvent>();
        let pending = Arc::new(Mutex::new(Vec::new()));
        let pending_clone = Arc::clone(&pending);

        thread::spawn(move || {
            worker_loop(cmd_rx, evt_tx, pending_clone, ctx);
        });

        Self {
            cmd_tx,
            evt_rx,
            pending,
        }
    }

    pub fn send(&self, cmd: DdcCommand) {
        let _ = self.cmd_tx.send(cmd);
    }

    pub fn queue_vcp(&self, bus: u32, vcp_code: u8, value: u16) {
        // Update instant cache immediately so Top Bar & GUI stay in sync with 0ms latency
        let mut cached = CachedMonitorData::load_instant();
        if vcp_code == 0x10 {
            cached.brightness = value;
        } else if vcp_code == 0x12 {
            cached.contrast = value;
        }
        cached.save_instant();

        if let Ok(mut q) = self.pending.lock() {
            if let Some(existing) = q.iter_mut().find(|item| item.vcp_code == vcp_code) {
                existing.value = value;
            } else {
                q.push(PendingVcp {
                    bus,
                    vcp_code,
                    value,
                });
            }
        }
        let _ = self.cmd_tx.send(DdcCommand::WakePendingVcp);
    }
}

fn send_evt(tx: &Sender<DdcEvent>, ctx: &eframe::egui::Context, evt: DdcEvent) {
    let _ = tx.send(evt);
    ctx.request_repaint();
}

fn worker_loop(
    cmd_rx: Receiver<DdcCommand>,
    evt_tx: Sender<DdcEvent>,
    pending: Arc<Mutex<Vec<PendingVcp>>>,
    ctx: eframe::egui::Context,
) {
    while let Ok(cmd) = cmd_rx.recv() {
        drain_pending(&evt_tx, &pending, &ctx);

        match cmd {
            DdcCommand::WakePendingVcp => {
                drain_pending(&evt_tx, &pending, &ctx);
            }
            DdcCommand::SyncFromHardware => {
                send_evt(&evt_tx, &ctx, DdcEvent::BusyChanged(true));
                let data = read_hardware_state();
                data.save_instant();
                send_evt(&evt_tx, &ctx, DdcEvent::DataSynced(data));
                send_evt(
                    &evt_tx,
                    &ctx,
                    DdcEvent::StatusText("Monitörle senkronize edildi".to_string()),
                );
                send_evt(&evt_tx, &ctx, DdcEvent::BusyChanged(false));
            }
        }
    }
}

fn drain_pending(
    evt_tx: &Sender<DdcEvent>,
    pending: &Arc<Mutex<Vec<PendingVcp>>>,
    ctx: &eframe::egui::Context,
) {
    loop {
        let item = {
            if let Ok(mut q) = pending.lock() {
                if q.is_empty() {
                    None
                } else {
                    Some(q.remove(0))
                }
            } else {
                None
            }
        };

        let Some(p) = item else {
            break;
        };

        send_evt(evt_tx, ctx, DdcEvent::BusyChanged(true));
        set_vcp_fast(p.bus, p.vcp_code, p.value);
        let label = if p.vcp_code == 0x10 {
            format!("Parlaklık %{} uygulandı", p.value)
        } else {
            format!("Kontrast %{} uygulandı", p.value)
        };
        send_evt(evt_tx, ctx, DdcEvent::StatusText(label));
        send_evt(evt_tx, ctx, DdcEvent::BusyChanged(false));
    }
}

pub fn set_vcp_fast(bus: u32, code: u8, value: u16) {
    let mut cached = CachedMonitorData::load_instant();
    if code == 0x10 {
        cached.brightness = value;
    } else if code == 0x12 {
        cached.contrast = value;
    }
    cached.save_instant();

    let args = [
        "--bus",
        &bus.to_string(),
        "--noudf",
        "--sleep-multiplier",
        "0.02",
        "--noverify",
        "setvcp",
        &format!("0x{:02x}", code),
        &value.to_string(),
    ];
    let output = Command::new("ddcutil").args(args).output();
    if let Ok(out) = output {
        if !out.status.success() {
            // Hızlı deneme başarısız olduysa 60ms sonra standart zamanlama ile bir kez daha dene
            thread::sleep(std::time::Duration::from_millis(60));
            let _ = Command::new("ddcutil")
                .args([
                    "--bus",
                    &bus.to_string(),
                    "--noudf",
                    "--noverify",
                    "setvcp",
                    &format!("0x{:02x}", code),
                    &value.to_string(),
                ])
                .output();
        }
    }
}

pub fn set_vcp_with_retry(bus: u32, code: u8, value: u16, max_retries: u32, delay_ms: u64) -> bool {
    let mut current_bus = bus;
    let code_str = format!("0x{:02x}", code);
    let val_str = value.to_string();

    for attempt in 0..=max_retries {
        let status = Command::new("ddcutil")
            .args([
                "--bus",
                &current_bus.to_string(),
                "--noudf",
                "--sleep-multiplier",
                "0.05",
                "--noverify",
                "setvcp",
                &code_str,
                &val_str,
            ])
            .status();

        if let Ok(st) = status {
            if st.success() {
                return true;
            }
        }

        // Eğer son denemeye yaklaştıysak ve hala başarısızsa I2C veri yolunu yeniden doğrula
        if attempt == max_retries.saturating_sub(1) {
            if let Some((new_bus, _, _)) = detect_monitor() {
                current_bus = new_bus;
            }
        }

        if attempt < max_retries {
            thread::sleep(std::time::Duration::from_millis(delay_ms));
        }
    }
    false
}

pub fn restore_hardware_settings(max_retries: u32, delay_ms: u64) -> bool {
    let mut data = CachedMonitorData::load_instant();
    let mut bus = data.i2c_bus;

    if bus == 0 || !data.supported {
        if let Some((detected_bus, connector, model)) = detect_monitor() {
            bus = detected_bus;
            data.i2c_bus = detected_bus;
            data.drm_connector = connector;
            data.model = model;
            data.supported = true;
            data.save_instant();
        } else {
            return false;
        }
    }

    // Parlaklığı uygula (deneme tekrarlı)
    let b_ok = set_vcp_with_retry(bus, 0x10, data.brightness, max_retries, delay_ms);
    // Monitörün I2C kontrolcüsüne komutlar arası nefes payı tanı
    thread::sleep(std::time::Duration::from_millis(80));
    // Kontrastı uygula (deneme tekrarlı)
    let c_ok = set_vcp_with_retry(bus, 0x12, data.contrast, max_retries, delay_ms);

    b_ok && c_ok
}

pub fn read_hardware_state() -> CachedMonitorData {
    let mut data = CachedMonitorData::load_instant();
    let saved_brightness = data.brightness;
    let saved_contrast = data.contrast;

    if !data.supported {
        match detect_monitor() {
            Some((bus, connector, model)) => {
                data.i2c_bus = bus;
                data.drm_connector = connector;
                data.model = model;
            }
            None => {
                data.model = "DDC/CI destekli monitör bulunamadı".to_string();
                data.vertical_freq = "-".to_string();
                data.save_instant();
                return data;
            }
        }
    }

    let mut got_brightness = false;
    let mut args = data.target_args();
    args.extend([
        "getvcp".to_string(),
        "10".to_string(),
        "12".to_string(),
        "ae".to_string(),
    ]);

    let Ok(output) = Command::new("ddcutil").args(&args).output() else {
        return data;
    };
    let text = String::from_utf8_lossy(&output.stdout);

    for line in text.lines() {
        let trimmed = line.trim();
        let Some(code_hex) = trimmed
            .strip_prefix("VCP code 0x")
            .and_then(|s| s.get(0..2))
            .and_then(|hex| u8::from_str_radix(hex, 16).ok())
        else {
            continue;
        };

        let rhs = trimmed.split("):").nth(1).unwrap_or("").trim();
        match code_hex {
            0x10 => {
                if let Some(cur) = parse_current_val(rhs) {
                    data.brightness = cur;
                    got_brightness = true;
                }
            }
            0x12 => {
                if let Some(cur) = parse_current_val(rhs) {
                    data.contrast = cur;
                }
            }
            0xAE => {
                let hz = rhs.replace("hz", "Hz");
                if !hz.is_empty() {
                    data.vertical_freq = hz;
                }
            }
            _ => {}
        }
    }

    data.supported = got_brightness;
    if !got_brightness {
        data.model = format!("{} (DDC/CI parlaklık desteği yok)", data.model);
    } else if saved_brightness > 0 {
        // Monitör uyku/kilit sonrası donanımsal olarak 100'e fırlasa bile
        // kullanıcının kaydettiği parlaklık ve kontrast tercihini koru.
        data.brightness = saved_brightness;
        data.contrast = saved_contrast;
    }
    data.save_instant();
    data
}

fn parse_current_val(rhs: &str) -> Option<u16> {
    for part in rhs.split(',') {
        let p = part.trim();
        if let Some(v) = p.strip_prefix("current value =") {
            return v.trim().parse::<u16>().ok();
        }
    }
    None
}
