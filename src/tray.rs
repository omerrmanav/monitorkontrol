use crate::ddc::{read_hardware_state, set_vcp_fast, CachedMonitorData};
use std::collections::HashMap;
use std::env;
use std::process::{Child, Command};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;
use zbus::interface;
use zbus::zvariant::{ObjectPath, OwnedObjectPath, OwnedValue, Value};

#[derive(Clone)]
struct TrayLauncher {
    gui_child: Arc<Mutex<Option<Child>>>,
}

impl TrayLauncher {
    fn new() -> Self {
        Self {
            gui_child: Arc::new(Mutex::new(None)),
        }
    }

    fn close_gui(&self) {
        if let Ok(mut guard) = self.gui_child.lock() {
            if let Some(ref mut child) = *guard {
                let _ = child.kill();
                let _ = child.wait();
            }
            *guard = None;
        }
    }
}

struct StatusNotifierItemIface;

#[interface(name = "org.kde.StatusNotifierItem")]
impl StatusNotifierItemIface {
    #[zbus(property)]
    fn category(&self) -> &str {
        "Hardware"
    }

    #[zbus(property)]
    fn id(&self) -> &str {
        "monitorkontrol"
    }

    #[zbus(property)]
    fn title(&self) -> String {
        let d = CachedMonitorData::load_instant();
        format!("Monitör Kontrol ({} • {})", d.model, d.vertical_freq)
    }

    #[zbus(property)]
    fn status(&self) -> &str {
        "Active"
    }

    #[zbus(property)]
    fn window_id(&self) -> i32 {
        0
    }

    #[zbus(property)]
    fn icon_theme_path(&self) -> &str {
        ""
    }

    #[zbus(property)]
    fn menu(&self) -> OwnedObjectPath {
        ObjectPath::from_static_str("/MenuBar").unwrap().into()
    }

    #[zbus(property)]
    fn item_is_menu(&self) -> bool {
        false
    }

    #[zbus(property)]
    fn icon_name(&self) -> &str {
        "video-display-symbolic"
    }

    #[zbus(property)]
    fn overlay_icon_name(&self) -> &str {
        ""
    }

    #[zbus(property)]
    fn attention_icon_name(&self) -> &str {
        ""
    }

    #[zbus(property)]
    fn attention_movie_name(&self) -> &str {
        ""
    }

    fn context_menu(&self, _x: i32, _y: i32) {}

    fn activate(&self, _x: i32, _y: i32) {}

    fn secondary_activate(&self, _x: i32, _y: i32) {}

    #[zbus(name = "XAyatanaSecondaryActivate")]
    fn x_ayatana_secondary_activate(&self, _timestamp: u32) {}

    fn provide_xdg_activation_token(&self, _token: String) {}

    fn scroll(&self, delta: i32, _orientation: String) {
        let mut cached = CachedMonitorData::load_instant();
        let step: i32 = if delta < 0 { 5 } else { -5 };
        let next = (cached.brightness as i32 + step).clamp(0, 100) as u16;
        if next != cached.brightness {
            cached.brightness = next;
            cached.save_instant();
            let bus = cached.i2c_bus;
            thread::spawn(move || {
                set_vcp_fast(bus, 0x10, next);
            });
        }
    }
}

struct DbusMenuIface {
    launcher: TrayLauncher,
    revision: u32,
}

impl DbusMenuIface {
    fn build_item(id: i32, props: HashMap<String, OwnedValue>) -> OwnedValue {
        let empty_children: Vec<OwnedValue> = Vec::new();
        Value::from((id, props, empty_children))
            .try_into()
            .expect("valid dbusmenu item tuple")
    }

    fn label_props(label: &str, enabled: bool) -> HashMap<String, OwnedValue> {
        let mut map = HashMap::new();
        map.insert("type".to_string(), Value::from("standard").try_into().unwrap());
        map.insert("label".to_string(), Value::from(label).try_into().unwrap());
        map.insert("enabled".to_string(), Value::from(enabled).try_into().unwrap());
        map.insert("visible".to_string(), Value::from(true).try_into().unwrap());
        map
    }

    fn separator_props() -> HashMap<String, OwnedValue> {
        let mut map = HashMap::new();
        map.insert("type".to_string(), Value::from("separator").try_into().unwrap());
        map.insert("visible".to_string(), Value::from(true).try_into().unwrap());
        map
    }

    /// Reads cached state in <0.02ms (zero I2C traffic, zero mutex wait).
    fn all_items(&self) -> Vec<(i32, HashMap<String, OwnedValue>)> {
        let data = CachedMonitorData::load_instant();
        let model_line = format!(
            "🖥  {}  •  ⚡ {} ({})",
            data.model, data.vertical_freq, data.drm_connector
        );
        let font_scale_str = get_current_font_scale();
        let status_line = format!(
            "☀  Parlaklık: %{}   •   ◑  Kontrast: %{}   •   🔤  Font: {}",
            data.brightness, data.contrast, font_scale_str
        );

        vec![
            (1, Self::label_props(&model_line, true)),
            (2, Self::label_props(&status_line, true)),
            (3, Self::separator_props()),
            (1100, Self::label_props("☀  Parlaklık: %100", true)),
            (1075, Self::label_props("☀  Parlaklık: %75", true)),
            (1050, Self::label_props("☀  Parlaklık: %50", true)),
            (1025, Self::label_props("☀  Parlaklık: %25", true)),
            (4, Self::separator_props()),
            (2100, Self::label_props("◑  Kontrast: %100", true)),
            (2085, Self::label_props("◑  Kontrast: %85", true)),
            (2070, Self::label_props("◑  Kontrast: %70", true)),
            (2050, Self::label_props("◑  Kontrast: %50", true)),
            (5, Self::separator_props()),
            (3100, Self::label_props("🔤  Yazı Ölçeği: 1.00x (Varsayılan)", true)),
            (3115, Self::label_props("🔤  Yazı Ölçeği: 1.15x", true)),
            (3125, Self::label_props("🔤  Yazı Ölçeği: 1.25x", true)),
            (3140, Self::label_props("🔤  Yazı Ölçeği: 1.40x", true)),
            (3150, Self::label_props("🔤  Yazı Ölçeği: 1.50x", true)),
            (6, Self::separator_props()),
            (99, Self::label_props("❌  Çıkış (Üst Bardan Kaldır)", true)),
        ]
    }

    fn item_props_by_id(&self, id: i32) -> Option<HashMap<String, OwnedValue>> {
        if id == 0 {
            let mut root_props = HashMap::new();
            root_props.insert(
                "children-display".to_string(),
                Value::from("submenu").try_into().unwrap(),
            );
            return Some(root_props);
        }
        self.all_items()
            .into_iter()
            .find(|(item_id, _)| *item_id == id)
            .map(|(_, props)| props)
    }
}

#[interface(name = "com.canonical.dbusmenu")]
impl DbusMenuIface {
    #[zbus(property)]
    fn version(&self) -> u32 {
        3
    }

    #[zbus(property)]
    fn text_direction(&self) -> &str {
        "ltr"
    }

    #[zbus(property)]
    fn status(&self) -> &str {
        "normal"
    }

    #[zbus(property)]
    fn icon_theme_path(&self) -> Vec<String> {
        Vec::new()
    }

    fn get_layout(
        &self,
        parent_id: i32,
        _recursion_depth: i32,
        _property_names: Vec<String>,
    ) -> (u32, (i32, HashMap<String, OwnedValue>, Vec<OwnedValue>)) {
        if parent_id == 0 {
            let root_props = self.item_props_by_id(0).unwrap_or_default();
            let children: Vec<OwnedValue> = self
                .all_items()
                .into_iter()
                .map(|(id, props)| Self::build_item(id, props))
                .collect();
            (self.revision, (0, root_props, children))
        } else if let Some(props) = self.item_props_by_id(parent_id) {
            (self.revision, (parent_id, props, Vec::new()))
        } else {
            (self.revision, (parent_id, HashMap::new(), Vec::new()))
        }
    }

    fn get_group_properties(
        &self,
        ids: Vec<i32>,
        _property_names: Vec<String>,
    ) -> Vec<(i32, HashMap<String, OwnedValue>)> {
        let mut out = Vec::new();
        for id in ids {
            if let Some(props) = self.item_props_by_id(id) {
                out.push((id, props));
            }
        }
        out
    }

    fn get_property(&self, id: i32, name: String) -> OwnedValue {
        if let Some(mut props) = self.item_props_by_id(id) {
            if let Some(val) = props.remove(&name) {
                return val;
            }
        }
        Value::from("").try_into().unwrap()
    }

    fn event(&mut self, id: i32, event_id: String, _data: Value<'_>, _timestamp: u32) {
        if event_id != "clicked" {
            return;
        }

        match id {
            1025 | 1050 | 1075 | 1100 => {
                let val = (id - 1000) as u16;
                let bus = CachedMonitorData::load_instant().i2c_bus;
                thread::spawn(move || {
                    set_vcp_fast(bus, 0x10, val);
                });
                self.revision = self.revision.wrapping_add(1);
            }
            2050 | 2070 | 2085 | 2100 => {
                let val = (id - 2000) as u16;
                let bus = CachedMonitorData::load_instant().i2c_bus;
                thread::spawn(move || {
                    set_vcp_fast(bus, 0x12, val);
                });
                self.revision = self.revision.wrapping_add(1);
            }
            3100 | 3115 | 3125 | 3140 | 3150 => {
                let factor = (id - 3000) as f64 / 100.0;
                thread::spawn(move || {
                    let _ = Command::new("gsettings")
                        .args(["set", "org.gnome.desktop.interface", "text-scaling-factor", &factor.to_string()])
                        .status();
                });
                self.revision = self.revision.wrapping_add(1);
            }
            99 => {
                self.launcher.close_gui();
                std::process::exit(0);
            }
            _ => {}
        }
    }

    fn event_group(&mut self, events: Vec<(i32, String, Value<'_>, u32)>) -> Vec<i32> {
        for (id, event_id, data, ts) in events {
            self.event(id, event_id, data, ts);
        }
        Vec::new()
    }

    fn about_to_show(&mut self, _id: i32) -> bool {
        false
    }

    fn about_to_show_group(&mut self, _ids: Vec<i32>) -> (Vec<i32>, Vec<i32>) {
        (Vec::new(), Vec::new())
    }
}

fn get_current_font_scale() -> String {
    if let Ok(output) = Command::new("gsettings")
        .args(["get", "org.gnome.desktop.interface", "text-scaling-factor"])
        .output()
    {
        let txt = String::from_utf8_lossy(&output.stdout).trim().to_string();
        if !txt.is_empty() {
            return format!("{}x", txt);
        }
    }
    "1.00x".to_string()
}

pub fn ensure_tray_daemon_running() {
    // Terminate any previous tray-daemon instance so the latest binary is always active
    let _ = Command::new("pkill")
        .args(["-f", "monitorkontrol --tray-daemon"])
        .status();
    thread::sleep(Duration::from_millis(80));

    if let Ok(exe) = env::current_exe() {
        let _ = Command::new(exe).arg("--tray-daemon").spawn();
    }
}

pub fn run_tray_daemon() -> zbus::Result<()> {
    // Sync hardware state once in background on daemon startup
    thread::spawn(|| {
        let _ = read_hardware_state();
    });

    let launcher = TrayLauncher::new();

    let sni = StatusNotifierItemIface;
    let menu = DbusMenuIface {
        launcher,
        revision: 1,
    };

    let bus_name = format!("org.kde.StatusNotifierItem-{}-1", std::process::id());

    let conn = zbus::blocking::connection::Builder::session()?
        .name("org.monitorkontrol.TrayDaemon")?
        .name(bus_name.as_str())?
        .serve_at("/StatusNotifierItem", sni)?
        .serve_at("/MenuBar", menu)?
        .build()?;

    let _ = conn.call_method(
        Some("org.kde.StatusNotifierWatcher"),
        "/StatusNotifierWatcher",
        Some("org.kde.StatusNotifierWatcher"),
        "RegisterStatusNotifierItem",
        &(bus_name.as_str(),),
    );

    loop {
        thread::sleep(Duration::from_secs(3600));
    }
}
