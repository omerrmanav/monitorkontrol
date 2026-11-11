import Clutter from 'gi://Clutter';
import Gio from 'gi://Gio';
import GLib from 'gi://GLib';
import GObject from 'gi://GObject';
import St from 'gi://St';

import {Extension} from 'resource:///org/gnome/shell/extensions/extension.js';
import * as Main from 'resource:///org/gnome/shell/ui/main.js';
import * as PanelMenu from 'resource:///org/gnome/shell/ui/panelMenu.js';
import * as PopupMenu from 'resource:///org/gnome/shell/ui/popupMenu.js';
import {Slider} from 'resource:///org/gnome/shell/ui/slider.js';
import * as LoginManager from 'resource:///org/gnome/shell/misc/loginManager.js';

const CONFIG_DIR = GLib.build_filenamev([GLib.get_user_config_dir(), 'monitorkontrol']);
const CONFIG_FILE = GLib.build_filenamev([CONFIG_DIR, 'config.json']);
const TMP_CACHE_FILE = '/tmp/monitorkontrol_cache.json';

const MIN_FONT_SCALE = 0.70;
const MAX_FONT_SCALE = 2.00;

function quantizeScale(sliderVal) {
    const raw = MIN_FONT_SCALE + sliderVal * (MAX_FONT_SCALE - MIN_FONT_SCALE);
    return Math.max(MIN_FONT_SCALE, Math.min(MAX_FONT_SCALE, Math.round(raw * 20) / 20));
}

function scaleToSlider(factor) {
    const clamped = Math.max(MIN_FONT_SCALE, Math.min(MAX_FONT_SCALE, factor));
    return (clamped - MIN_FONT_SCALE) / (MAX_FONT_SCALE - MIN_FONT_SCALE);
}

/**
 * System hardware reader: CPU Temp, CPU %, RAM, GPU (nvidia-smi)
 */
class SystemMonitorReader {
    constructor() {
        this._lastCpuTotal = 0;
        this._lastCpuActive = 0;
    }

    getCpuName() {
        try {
            let [ok, bytes] = GLib.file_get_contents('/proc/cpuinfo');
            if (!ok || !bytes)
                return 'İşlemci';
            let text = new TextDecoder().decode(bytes);
            for (let line of text.split('\n')) {
                if (line.includes('model name')) {
                    let m = line.split(':')[1].trim();
                    m = m.replace(/\(R\)|\(TM\)|Processor|CPU/gi, '').trim();
                    m = m.replace(/\d+-Core.*/i, '').trim();
                    return m.replace(/\s+/g, ' ');
                }
            }
        } catch (e) {}
        return 'İşlemci';
    }

    getCpuTemp() {
        for (let i = 0; i < 8; i++) {
            try {
                let [okName, nameBytes] = GLib.file_get_contents(`/sys/class/hwmon/hwmon${i}/name`);
                if (okName) {
                    let name = new TextDecoder().decode(nameBytes).trim().toLowerCase();
                    // Sadece gerçek CPU sensörlerini hedefle (nvme, gpu ve wmi aygıtlarını hariç tut)
                    if (name.includes('k10temp') || name.includes('coretemp') || name.includes('zenpower') || name.includes('cpu_thermal')) {
                        // AMD Ryzen için Tccd1 veya Tdie (gerçek silikon çekirdek sıcaklığı) ara
                        for (let s = 1; s <= 6; s++) {
                            try {
                                let [okLabel, labelBytes] = GLib.file_get_contents(`/sys/class/hwmon/hwmon${i}/temp${s}_label`);
                                if (okLabel) {
                                    let label = new TextDecoder().decode(labelBytes).trim().toLowerCase();
                                    if (label.includes('tccd') || label.includes('tdie')) {
                                        let [okVal, valBytes] = GLib.file_get_contents(`/sys/class/hwmon/hwmon${i}/temp${s}_input`);
                                        if (okVal) {
                                            return Math.round(parseInt(new TextDecoder().decode(valBytes).trim(), 10) / 1000);
                                        }
                                    }
                                }
                            } catch (e) {}
                        }
                        // Tccd bulunamazsa standart temp1_input (Tctl / Package) değerini al
                        let [okTemp, tempBytes] = GLib.file_get_contents(`/sys/class/hwmon/hwmon${i}/temp1_input`);
                        if (okTemp) {
                            return Math.round(parseInt(new TextDecoder().decode(tempBytes).trim(), 10) / 1000);
                        }
                    }
                }
            } catch (e) {}
        }
        return null;
    }

    getCpuUsage() {
        try {
            let [ok, bytes] = GLib.file_get_contents('/proc/stat');
            if (!ok || !bytes)
                return null;
            let line = new TextDecoder().decode(bytes).split('\n')[0];
            let parts = line.trim().split(/\s+/).slice(1).map(Number);
            if (parts.length >= 7) {
                let [user, nice, sys, idle, iowait, irq, softirq] = parts;
                let total = user + nice + sys + idle + iowait + irq + softirq;
                let active = total - idle - iowait;

                let pct = 0;
                if (this._lastCpuTotal > 0 && total > this._lastCpuTotal) {
                    let deltaTotal = total - this._lastCpuTotal;
                    let deltaActive = active - this._lastCpuActive;
                    pct = Math.round((deltaActive / deltaTotal) * 100);
                }
                this._lastCpuTotal = total;
                this._lastCpuActive = active;
                return Math.max(0, Math.min(100, pct));
            }
        } catch (e) {}
        return null;
    }

    getRamUsage() {
        try {
            let [ok, bytes] = GLib.file_get_contents('/proc/meminfo');
            if (!ok || !bytes)
                return null;
            let text = new TextDecoder().decode(bytes);
            let total = 0, avail = 0;
            for (let l of text.split('\n')) {
                if (l.startsWith('MemTotal:'))
                    total = parseInt(l.replace(/[^0-9]/g, ''), 10);
                else if (l.startsWith('MemAvailable:'))
                    avail = parseInt(l.replace(/[^0-9]/g, ''), 10);
            }
            if (total > 0) {
                let used = total - avail;
                return {
                    pct: Math.round((used / total) * 100),
                    usedGb: (used / 1048576).toFixed(1),
                    totalGb: (total / 1048576).toFixed(1),
                };
            }
        } catch (e) {}
        return null;
    }

    fetchGpuMetrics(callback) {
        try {
            let proc = Gio.Subprocess.new(
                ['nvidia-smi', '--query-gpu=temperature.gpu,utilization.gpu,name', '--format=csv,noheader,nounits'],
                Gio.SubprocessFlags.STDOUT_PIPE | Gio.SubprocessFlags.STDERR_SILENCE
            );
            proc.communicate_utf8_async(null, null, (p, res) => {
                try {
                    let [, stdout] = p.communicate_utf8_finish(res);
                    if (stdout) {
                        let parts = stdout.trim().split(',');
                        if (parts.length >= 2) {
                            let temp = parseInt(parts[0].trim(), 10);
                            let util = parseInt(parts[1].trim(), 10);
                            let name = 'GPU';
                            if (parts.length >= 3) {
                                name = parts[2].trim().replace(/^NVIDIA\s+/i, '').replace(/^GeForce\s+/i, '').trim();
                            }
                            callback({temp, util, name});
                        }
                    }
                } catch (e) {}
            });
        } catch (e) {}
    }
}

/**
 * Serializes all ddcutil hardware writes so that only one process runs at a time.
 * Automatically coalesces rapid slider changes so intermediate values are discarded.
 */
class DdcQueue {
    constructor(binPath) {
        this._binPath = binPath;
        this._running = false;
        this._pending = [];
    }

    push(flag, value) {
        const existing = this._pending.find(item => item.flag === flag);
        if (existing) {
            existing.value = value;
        } else {
            this._pending.push({flag, value});
        }
        this._processNext();
    }

    _processNext() {
        if (this._running || this._pending.length === 0)
            return;

        this._running = true;
        const next = this._pending.shift();

        try {
            const args = (next.flag === '--restore')
                ? [this._binPath, '--restore']
                : [this._binPath, next.flag, String(next.value)];

            const proc = Gio.Subprocess.new(
                args,
                Gio.SubprocessFlags.STDOUT_SILENCE | Gio.SubprocessFlags.STDERR_SILENCE
            );
            proc.wait_async(null, () => {
                this._running = false;
                this._processNext();
            });
        } catch (e) {
            this._running = false;
            this._processNext();
        }
    }
}

const MonitorTopBarIndicator = GObject.registerClass(
class MonitorTopBarIndicator extends PanelMenu.Button {
    _init(binPath) {
        super._init(0.5, 'Monitör Kontrol', false);
        this.menu.box.set_style('min-width: 290px;');

        // The helper binary is installed next to extension.js, so no absolute path is needed.
        this._binPath = binPath;
        this._ddcQueue = new DdcQueue(binPath);
        this._sysReader = new SystemMonitorReader();
        this._interfaceSettings = new Gio.Settings({schema_id: 'org.gnome.desktop.interface'});

        this._debounceBrightnessId = 0;
        this._debounceContrastId = 0;
        this._debounceFontId = 0;
        this._statsTimerId = 0;
        this._restoreTimerId = 0;
        this._startupRestoreTimers = [];
        this._shieldSignalId = 0;
        this._sleepSignalId = 0;
        this._monitorsSignalId = 0;
        this._updatingFromCache = false;
        this._updatingFromSettings = false;

        // Top Bar small icon
        const box = new St.BoxLayout({style_class: 'panel-status-menu-box'});
        const icon = new St.Icon({
            icon_name: 'video-display-symbolic',
            style_class: 'system-status-icon',
        });
        box.add_child(icon);
        this.add_child(box);

        // =========================================================================
        // 1. Header: Monitor Name, Hz Badge & Refresh button
        // =========================================================================
        this._headerItem = new PopupMenu.PopupBaseMenuItem({reactive: false});
        const headerBox = new St.BoxLayout({
            vertical: false,
            x_expand: true,
            y_align: Clutter.ActorAlign.CENTER,
            style: 'spacing: 8px; padding: 2px 0;',
        });

        const monitorIcon = new St.Icon({
            icon_name: 'video-display-symbolic',
            icon_size: 16,
            y_align: Clutter.ActorAlign.CENTER,
        });

        this._modelLabel = new St.Label({
            text: 'INTENSE X60',
            y_align: Clutter.ActorAlign.CENTER,
            style: 'font-weight: bold; font-size: 13.5px;',
        });

        this._hzLabel = new St.Label({
            text: '164 Hz',
            y_align: Clutter.ActorAlign.CENTER,
            x_expand: true,
            x_align: Clutter.ActorAlign.END,
            style: 'font-weight: 600; font-size: 11.5px; color: #34d399; background-color: rgba(52, 211, 153, 0.12); padding: 2px 7px; border-radius: 5px;',
        });

        const refreshBtn = new St.Button({
            can_focus: true,
            y_align: Clutter.ActorAlign.CENTER,
            style: 'padding: 4px 6px; border-radius: 6px; background-color: rgba(255, 255, 255, 0.08);',
        });
        const refreshIcon = new St.Icon({
            icon_name: 'view-refresh-symbolic',
            icon_size: 13,
        });
        refreshBtn.set_child(refreshIcon);
        refreshBtn.connect('clicked', () => {
            this._syncHardwareInBackground();
            this._refreshSystemStats();
        });

        headerBox.add_child(monitorIcon);
        headerBox.add_child(this._modelLabel);
        headerBox.add_child(this._hzLabel);
        headerBox.add_child(refreshBtn);

        const headerWrapper = new St.BoxLayout({
            vertical: true,
            x_expand: true,
            style: 'spacing: 3px;',
        });
        headerWrapper.add_child(headerBox);

        // Hardware Subtitle: CPU & GPU Name (clean & subtle)
        this._hwSubtitleLabel = new St.Label({
            text: 'Donanım taranıyor...',
            style: 'font-size: 11px; font-weight: 500; color: rgba(255, 255, 255, 0.45); margin-left: 24px;',
        });
        headerWrapper.add_child(this._hwSubtitleLabel);

        this._headerItem.add_child(headerWrapper);
        this.menu.addMenuItem(this._headerItem);

        // =========================================================================
        // 2. Hardware Metrics Dashboard (3-Column Minimal Card)
        // =========================================================================
        this._statsItem = new PopupMenu.PopupBaseMenuItem({reactive: false});
        const statsBox = new St.BoxLayout({
            vertical: false,
            x_expand: true,
            style: 'background-color: rgba(255, 255, 255, 0.05); border-radius: 8px; padding: 6px 4px;',
        });

        const createStatCol = (title) => {
            const col = new St.BoxLayout({
                vertical: true,
                x_expand: true,
                x_align: Clutter.ActorAlign.CENTER,
                style: 'spacing: 3px;',
            });
            const head = new St.BoxLayout({
                x_align: Clutter.ActorAlign.CENTER,
                style: 'spacing: 5px;',
            });
            head.add_child(new St.Label({
                text: title,
                y_align: Clutter.ActorAlign.CENTER,
                style: 'font-size: 11px; font-weight: bold; color: rgba(255, 255, 255, 0.55);',
            }));
            const pctLabel = new St.Label({
                text: '%--',
                y_align: Clutter.ActorAlign.CENTER,
                style: 'font-size: 11px; font-weight: bold;',
            });
            head.add_child(pctLabel);
            const valLabel = new St.Label({
                text: '--',
                x_align: Clutter.ActorAlign.CENTER,
                style: 'font-size: 13px; font-weight: bold;',
            });
            col.add_child(head);
            col.add_child(valLabel);
            return {col, pctLabel, valLabel};
        };

        const cpuCol = createStatCol('CPU');
        const gpuCol = createStatCol('GPU');
        const ramCol = createStatCol('RAM');

        this._cpuPctLabel = cpuCol.pctLabel;
        this._gpuPctLabel = gpuCol.pctLabel;
        this._ramPctLabel = ramCol.pctLabel;
        this._cpuStatsLabel = cpuCol.valLabel;
        this._gpuStatsLabel = gpuCol.valLabel;
        this._ramStatsLabel = ramCol.valLabel;

        statsBox.add_child(cpuCol.col);
        statsBox.add_child(gpuCol.col);
        statsBox.add_child(ramCol.col);

        this._statsItem.add_child(statsBox);
        this.menu.addMenuItem(this._statsItem);

        this.menu.addMenuItem(new PopupMenu.PopupSeparatorMenuItem());

        // =========================================================================
        // 3. Brightness Section (Parlaklık Slider)
        // =========================================================================
        this._brightnessItem = new PopupMenu.PopupBaseMenuItem({activate: false});
        const bContainer = new St.BoxLayout({
            vertical: true,
            x_expand: true,
            style: 'spacing: 4px; padding: 2px 0;',
        });

        const bHeader = new St.BoxLayout({
            vertical: false,
            x_expand: true,
            y_align: Clutter.ActorAlign.CENTER,
        });
        const bIcon = new St.Icon({
            icon_name: 'display-brightness-symbolic',
            icon_size: 15,
            y_align: Clutter.ActorAlign.CENTER,
            style: 'margin-right: 6px;',
        });
        const bTitle = new St.Label({
            text: 'Parlaklık',
            y_align: Clutter.ActorAlign.CENTER,
            style: 'font-weight: 600; font-size: 13px;',
        });
        this._brightnessValLabel = new St.Label({
            text: '%50',
            x_expand: true,
            x_align: Clutter.ActorAlign.END,
            y_align: Clutter.ActorAlign.CENTER,
            style: 'font-weight: bold; font-size: 13px; color: #fbbf24;',
        });

        bHeader.add_child(bIcon);
        bHeader.add_child(bTitle);
        bHeader.add_child(this._brightnessValLabel);

        this._brightnessSlider = new Slider(0.5);
        this._brightnessSlider.connect('notify::value', () => {
            if (this._updatingFromCache)
                return;
            const pct = Math.round(this._brightnessSlider.value * 100);
            this._brightnessValLabel.set_text(`%${pct}`);
            this._scheduleVcp('--brightness', pct, true);
        });
        this._brightnessSlider.connect('drag-end', () => {
            if (this._debounceBrightnessId) {
                GLib.source_remove(this._debounceBrightnessId);
                this._debounceBrightnessId = 0;
            }
            const pct = Math.round(this._brightnessSlider.value * 100);
            this._ddcQueue.push('--brightness', pct);
        });

        bContainer.add_child(bHeader);
        bContainer.add_child(this._brightnessSlider);
        this._brightnessItem.add_child(bContainer);
        this.menu.addMenuItem(this._brightnessItem);

        // =========================================================================
        // 4. Contrast Section (Kontrast Slider)
        // =========================================================================
        this._contrastItem = new PopupMenu.PopupBaseMenuItem({activate: false});
        const cContainer = new St.BoxLayout({
            vertical: true,
            x_expand: true,
            style: 'spacing: 4px; padding: 2px 0;',
        });

        const cHeader = new St.BoxLayout({
            vertical: false,
            x_expand: true,
            y_align: Clutter.ActorAlign.CENTER,
        });
        const cIcon = new St.Icon({
            icon_name: 'preferences-color-symbolic',
            icon_size: 15,
            y_align: Clutter.ActorAlign.CENTER,
            style: 'margin-right: 6px;',
        });
        const cTitle = new St.Label({
            text: 'Kontrast',
            y_align: Clutter.ActorAlign.CENTER,
            style: 'font-weight: 600; font-size: 13px;',
        });
        this._contrastValLabel = new St.Label({
            text: '%70',
            x_expand: true,
            x_align: Clutter.ActorAlign.END,
            y_align: Clutter.ActorAlign.CENTER,
            style: 'font-weight: bold; font-size: 13px; color: #38bdf8;',
        });

        cHeader.add_child(cIcon);
        cHeader.add_child(cTitle);
        cHeader.add_child(this._contrastValLabel);

        this._contrastSlider = new Slider(0.7);
        this._contrastSlider.connect('notify::value', () => {
            if (this._updatingFromCache)
                return;
            const pct = Math.round(this._contrastSlider.value * 100);
            this._contrastValLabel.set_text(`%${pct}`);
            this._scheduleVcp('--contrast', pct, false);
        });
        this._contrastSlider.connect('drag-end', () => {
            if (this._debounceContrastId) {
                GLib.source_remove(this._debounceContrastId);
                this._debounceContrastId = 0;
            }
            const pct = Math.round(this._contrastSlider.value * 100);
            this._ddcQueue.push('--contrast', pct);
        });

        cContainer.add_child(cHeader);
        cContainer.add_child(this._contrastSlider);
        this._contrastItem.add_child(cContainer);
        this.menu.addMenuItem(this._contrastItem);

        this.menu.addMenuItem(new PopupMenu.PopupSeparatorMenuItem());

        // =========================================================================
        // 5. Tweaks Font Scaling Factor Section (Yazı / Font Ölçeği)
        // =========================================================================
        this._fontItem = new PopupMenu.PopupBaseMenuItem({activate: false});
        const fContainer = new St.BoxLayout({
            vertical: true,
            x_expand: true,
            style: 'spacing: 4px; padding: 2px 0;',
        });

        const fHeader = new St.BoxLayout({
            vertical: false,
            x_expand: true,
            y_align: Clutter.ActorAlign.CENTER,
        });
        const fIcon = new St.Icon({
            icon_name: 'preferences-desktop-font-symbolic',
            icon_size: 15,
            y_align: Clutter.ActorAlign.CENTER,
            style: 'margin-right: 6px;',
        });
        const fTitle = new St.Label({
            text: 'Yazı Boyutu',
            x_expand: true,
            y_align: Clutter.ActorAlign.CENTER,
            style: 'font-weight: 600; font-size: 13px; margin-right: 12px;',
        });

        const fStepper = new St.BoxLayout({
            vertical: false,
            x_expand: false,
            x_align: Clutter.ActorAlign.END,
            y_align: Clutter.ActorAlign.CENTER,
            style: 'spacing: 4px;',
        });

        const stepFont = (delta) => {
            const current = this._interfaceSettings.get_double('text-scaling-factor') || 1.0;
            const next = Math.max(MIN_FONT_SCALE, Math.min(MAX_FONT_SCALE, Math.round((current + delta) * 20) / 20));
            this._interfaceSettings.set_double('text-scaling-factor', next);
            this._syncFontScaleFromSettings();
        };

        const setFontDirect = (target) => {
            this._interfaceSettings.set_double('text-scaling-factor', target);
            this._syncFontScaleFromSettings();
        };

        const minusBtn = new St.Button({
            label: ' − ',
            can_focus: true,
            style: 'padding: 1px 7px; border-radius: 4px; background-color: rgba(255, 255, 255, 0.08); font-size: 12px; font-weight: bold;',
        });
        minusBtn.connect('clicked', () => stepFont(-0.05));

        this._fontValLabel = new St.Label({
            text: '1.00x',
            y_align: Clutter.ActorAlign.CENTER,
            style: 'font-weight: bold; font-size: 13px; color: #a78bfa; margin: 0 4px;',
        });

        const plusBtn = new St.Button({
            label: ' + ',
            can_focus: true,
            style: 'padding: 1px 7px; border-radius: 4px; background-color: rgba(255, 255, 255, 0.08); font-size: 12px; font-weight: bold;',
        });
        plusBtn.connect('clicked', () => stepFont(+0.05));

        const resetBtn = new St.Button({
            label: '↺',
            can_focus: true,
            style: 'padding: 1px 5px; border-radius: 4px; background-color: rgba(255, 255, 255, 0.06); font-size: 11px; margin-left: 2px;',
        });
        resetBtn.connect('clicked', () => setFontDirect(1.00));

        fStepper.add_child(minusBtn);
        fStepper.add_child(this._fontValLabel);
        fStepper.add_child(plusBtn);
        fStepper.add_child(resetBtn);

        fHeader.add_child(fIcon);
        fHeader.add_child(fTitle);
        fHeader.add_child(fStepper);

        this._fontSlider = new Slider(scaleToSlider(1.0));
        this._fontSlider.connect('notify::value', () => {
            if (this._updatingFromSettings)
                return;
            const factor = quantizeScale(this._fontSlider.value);
            this._fontValLabel.set_text(`${factor.toFixed(2)}x`);

            if (this._debounceFontId)
                GLib.source_remove(this._debounceFontId);

            this._debounceFontId = GLib.timeout_add(GLib.PRIORITY_DEFAULT, 40, () => {
                this._debounceFontId = 0;
                this._interfaceSettings.set_double('text-scaling-factor', factor);
                return GLib.SOURCE_REMOVE;
            });
        });
        this._fontSlider.connect('drag-end', () => {
            if (this._debounceFontId) {
                GLib.source_remove(this._debounceFontId);
                this._debounceFontId = 0;
            }
            const factor = quantizeScale(this._fontSlider.value);
            this._interfaceSettings.set_double('text-scaling-factor', factor);
        });

        fContainer.add_child(fHeader);
        fContainer.add_child(this._fontSlider);
        this._fontItem.add_child(fContainer);
        this.menu.addMenuItem(this._fontItem);

        this.menu.addMenuItem(new PopupMenu.PopupSeparatorMenuItem());

        // =========================================================================
        // 6. Dark / Light Mode Switch (Koyu Tema)
        // =========================================================================
        const isDarkInitial = this._interfaceSettings.get_string('color-scheme') === 'prefer-dark';
        this._darkModeSwitchItem = new PopupMenu.PopupSwitchMenuItem('Koyu Tema', isDarkInitial);
        const darkIcon = new St.Icon({
            icon_name: 'night-light-symbolic',
            icon_size: 15,
            style: 'margin-right: 8px;',
        });
        this._darkModeSwitchItem.insert_child_at_index(darkIcon, 0);
        this._darkModeSwitchItem.label.set_style('font-weight: 600; font-size: 13px;');

        this._darkModeSwitchItem.connect('toggled', (_item, state) => {
            this._toggleDarkMode(state);
        });
        this.menu.addMenuItem(this._darkModeSwitchItem);

        // Listen to external changes to text-scaling-factor (e.g. from Tweaks app)
        this._settingsFontSignalId = this._interfaceSettings.connect(
            'changed::text-scaling-factor',
            () => this._syncFontScaleFromSettings()
        );

        // Listen to external changes to color-scheme
        this._settingsColorSignalId = this._interfaceSettings.connect(
            'changed::color-scheme',
            () => {
                const isDark = this._interfaceSettings.get_string('color-scheme') === 'prefer-dark';
                this._darkModeSwitchItem.setToggleState(isDark);
            }
        );

        // Open/Close menu lifecycle
        this.menu.connect('open-state-changed', (_menu, isOpen) => {
            if (isOpen) {
                this._loadInstantCache();
                this._syncFontScaleFromSettings();
                this._refreshSystemStats();

                // Live tick system stats every 2 seconds while menu is open
                if (!this._statsTimerId) {
                    this._statsTimerId = GLib.timeout_add_seconds(GLib.PRIORITY_DEFAULT, 2, () => {
                        this._refreshSystemStats();
                        return GLib.SOURCE_CONTINUE;
                    });
                }
            } else if (this._statsTimerId) {
                GLib.source_remove(this._statsTimerId);
                this._statsTimerId = 0;
            }
        });

        this._loadInstantCache();
        this._syncFontScaleFromSettings();

        // 1. Initial restore on login / extension start (kademeli soğuk açılış darbeleri)
        this._scheduleStartupRestore();

        // 2. Restore when screen unlocks (şifreyi girip enter'a basar basmaz anında uygulanır)
        if (Main.screenShield) {
            this._shieldSignalId = Main.screenShield.connect('active-changed', () => {
                if (!Main.screenShield.active) {
                    // Lock screen dismissed / screen unlocked!
                    this._restoreSavedHardwareSettings(30);
                }
            });
        }

        // 3. Restore when system resumes from sleep / suspend
        try {
            this._loginManager = LoginManager.getLoginManager();
            if (this._loginManager) {
                this._sleepSignalId = this._loginManager.connect('prepare-for-sleep', (_lm, aboutToSuspend) => {
                    if (!aboutToSuspend) {
                        // System just resumed from sleep!
                        this._scheduleStartupRestore();
                    }
                });
            }
        } catch (e) {}

        // 4. Restore when monitors wake or reconfigure
        this._monitorsSignalId = Main.layoutManager.connect('monitors-changed', () => {
            this._restoreSavedHardwareSettings(30);
        });
    }

    _toggleDarkMode(enabled) {
        const scheme = enabled ? 'prefer-dark' : 'default';
        this._interfaceSettings.set_string('color-scheme', scheme);

        // Also update gtk-theme if it has or needs -dark
        const currentTheme = this._interfaceSettings.get_string('gtk-theme') || '';
        if (enabled && !currentTheme.endsWith('-dark')) {
            this._interfaceSettings.set_string('gtk-theme', `${currentTheme}-dark`);
        } else if (!enabled && currentTheme.endsWith('-dark')) {
            this._interfaceSettings.set_string('gtk-theme', currentTheme.replace(/-dark$/, ''));
        }
    }

    _refreshSystemStats() {
        const cpuName = this._sysReader.getCpuName();
        const cpuTemp = this._sysReader.getCpuTemp();
        const cpuPct = this._sysReader.getCpuUsage();
        const ram = this._sysReader.getRamUsage();

        const GREEN = '#4ade80', YELLOW = '#facc15', RED = '#f87171';
        const pick = (v, warn, crit) => (v >= crit ? RED : v >= warn ? YELLOW : GREEN);
        const paint = (label, text, color, size) =>
            label.set_style(`font-size: ${size}px; font-weight: bold; color: ${color};`) || label.set_text(text);

        // Ryzen 5 5600 (Tjmax 90°C): <65 normal, 65-79 sıcak, 80+ kritik
        if (cpuTemp !== null)
            paint(this._cpuStatsLabel, `${cpuTemp}°C`, pick(cpuTemp, 65, 80), 13);
        if (cpuPct !== null)
            paint(this._cpuPctLabel, `%${cpuPct}`, pick(cpuPct, 50, 85), 11);

        // 16 GB RAM: <%60 rahat, %60-84 dolu, %85+ kritik
        if (ram) {
            paint(this._ramStatsLabel, `${ram.usedGb} GB`, pick(ram.pct, 60, 85), 13);
            paint(this._ramPctLabel, `%${ram.pct}`, pick(ram.pct, 60, 85), 11);
        }

        // RTX 3060 (throttle ~83°C): <70 normal, 70-79 sıcak, 80+ kritik
        this._sysReader.fetchGpuMetrics(gpu => {
            if (gpu) {
                paint(this._gpuStatsLabel, `${gpu.temp}°C`, pick(gpu.temp, 70, 80), 13);
                paint(this._gpuPctLabel, `%${gpu.util}`, pick(gpu.util, 50, 85), 11);
                if (gpu.name) {
                    this._hwSubtitleLabel.set_text(`${cpuName}  •  ${gpu.name}`);
                } else {
                    this._hwSubtitleLabel.set_text(cpuName);
                }
            } else {
                this._hwSubtitleLabel.set_text(cpuName);
            }
        });
    }

    _scheduleVcp(flag, pct, isBrightness) {
        try {
            const data = this._getCurrentCachedData();
            if (isBrightness) {
                data.brightness = pct;
            } else {
                data.contrast = pct;
            }
            this._saveInstantCache(data);
        } catch (e) {}

        if (isBrightness) {
            if (this._debounceBrightnessId)
                GLib.source_remove(this._debounceBrightnessId);
            this._debounceBrightnessId = GLib.timeout_add(GLib.PRIORITY_DEFAULT, 120, () => {
                this._debounceBrightnessId = 0;
                this._ddcQueue.push(flag, pct);
                return GLib.SOURCE_REMOVE;
            });
        } else {
            if (this._debounceContrastId)
                GLib.source_remove(this._debounceContrastId);
            this._debounceContrastId = GLib.timeout_add(GLib.PRIORITY_DEFAULT, 120, () => {
                this._debounceContrastId = 0;
                this._ddcQueue.push(flag, pct);
                return GLib.SOURCE_REMOVE;
            });
        }
    }

    _getCurrentCachedData() {
        try {
            let [ok, bytes] = GLib.file_get_contents(CONFIG_FILE);
            if (ok && bytes) {
                return JSON.parse(new TextDecoder().decode(bytes));
            }
            let [okTmp, bytesTmp] = GLib.file_get_contents(TMP_CACHE_FILE);
            if (okTmp && bytesTmp) {
                return JSON.parse(new TextDecoder().decode(bytesTmp));
            }
        } catch (e) {}
        return {
            brightness: 50,
            contrast: 80,
            supported: true,
        };
    }

    _saveInstantCache(data) {
        try {
            GLib.mkdir_with_parents(CONFIG_DIR, 0o755);
            const json = JSON.stringify(data);
            GLib.file_set_contents(CONFIG_FILE, json);
            GLib.file_set_contents(TMP_CACHE_FILE, json);
        } catch (e) {}
    }

    _restoreSavedHardwareSettings(delayMs = 30) {
        if (this._restoreTimerId) {
            GLib.source_remove(this._restoreTimerId);
            this._restoreTimerId = 0;
        }
        this._restoreTimerId = GLib.timeout_add(GLib.PRIORITY_DEFAULT, delayMs, () => {
            this._restoreTimerId = 0;
            try {
                this._loadInstantCache();
                const data = this._getCurrentCachedData();
                if (data && data.supported !== false) {
                    this._ddcQueue.push('--restore', 0);
                }
            } catch (e) {}
            return GLib.SOURCE_REMOVE;
        });
    }

    _scheduleStartupRestore() {
        this._clearStartupRestoreTimers();
        // Soğuk açılış veya uykudan uyanmada monitörün DDC kontrolcüsü 1.5 - 3.5 sn içinde hazır olur.
        // Kesintisiz başarı için 3 darbeli kademeli geri yükleme:
        // 1. Darbe: 150ms (ekran zaten aktifse anında uygular)
        // 2. Darbe: 2200ms (soğuk açılışta DisplayPort link training bitişini yakalar)
        // 3. Darbe: 4500ms (yavaş uyanan monitörler için kesin garanti darbesi)
        const pulses = [150, 2200, 4500];
        for (const delay of pulses) {
            const id = GLib.timeout_add(GLib.PRIORITY_DEFAULT, delay, () => {
                try {
                    this._loadInstantCache();
                    const data = this._getCurrentCachedData();
                    if (data && data.supported !== false) {
                        this._ddcQueue.push('--restore', 0);
                    }
                } catch (e) {}
                return GLib.SOURCE_REMOVE;
            });
            this._startupRestoreTimers.push(id);
        }
    }

    _clearStartupRestoreTimers() {
        if (this._startupRestoreTimers && this._startupRestoreTimers.length > 0) {
            for (const id of this._startupRestoreTimers) {
                if (id)
                    GLib.source_remove(id);
            }
            this._startupRestoreTimers = [];
        }
    }

    _syncFontScaleFromSettings() {
        try {
            const factor = this._interfaceSettings.get_double('text-scaling-factor') || 1.0;
            this._updatingFromSettings = true;
            this._fontValLabel.set_text(`${factor.toFixed(2)}x`);
            this._fontSlider.value = scaleToSlider(factor);
            this._updatingFromSettings = false;
        } catch (e) {
            this._updatingFromSettings = false;
        }
    }

    _loadInstantCache() {
        try {
            let contents = null;
            let [ok, bytes] = GLib.file_get_contents(CONFIG_FILE);
            if (ok && bytes) {
                contents = bytes;
            } else {
                let [okTmp, bytesTmp] = GLib.file_get_contents(TMP_CACHE_FILE);
                if (okTmp && bytesTmp)
                    contents = bytesTmp;
            }
            if (!contents)
                return;
            const text = new TextDecoder().decode(contents);
            const data = JSON.parse(text);
            this._updatingFromCache = true;
            const usable = data.supported !== false;
            this._brightnessSlider.reactive = usable;
            this._contrastSlider.reactive = usable;
            this._brightnessItem.opacity = usable ? 255 : 110;
            this._contrastItem.opacity = usable ? 255 : 110;
            if (data.model)
                this._modelLabel.set_text(data.model);
            if (data.vertical_freq) {
                const rawHz = parseFloat(data.vertical_freq);
                const hzStr = isNaN(rawHz) ? data.vertical_freq : `${Math.round(rawHz)} Hz`;
                this._hzLabel.set_text(hzStr);
            }
            if (typeof data.brightness === 'number') {
                this._brightnessValLabel.set_text(`%${data.brightness}`);
                this._brightnessSlider.value = Math.max(0, Math.min(1, data.brightness / 100));
            }
            if (typeof data.contrast === 'number') {
                this._contrastValLabel.set_text(`%${data.contrast}`);
                this._contrastSlider.value = Math.max(0, Math.min(1, data.contrast / 100));
            }
            this._updatingFromCache = false;
        } catch (e) {
            this._updatingFromCache = false;
        }
    }

    _syncHardwareInBackground() {
        try {
            const proc = Gio.Subprocess.new(
                [this._binPath, '--sync'],
                Gio.SubprocessFlags.STDOUT_SILENCE | Gio.SubprocessFlags.STDERR_SILENCE
            );
            proc.wait_async(null, () => {
                this._loadInstantCache();
            });
        } catch (e) {
            logError(e);
        }
    }

    destroy() {
        this._clearStartupRestoreTimers();
        if (this._restoreTimerId) {
            GLib.source_remove(this._restoreTimerId);
            this._restoreTimerId = 0;
        }
        if (this._shieldSignalId && Main.screenShield) {
            Main.screenShield.disconnect(this._shieldSignalId);
            this._shieldSignalId = 0;
        }
        if (this._sleepSignalId && this._loginManager) {
            this._loginManager.disconnect(this._sleepSignalId);
            this._sleepSignalId = 0;
        }
        if (this._monitorsSignalId) {
            Main.layoutManager.disconnect(this._monitorsSignalId);
            this._monitorsSignalId = 0;
        }
        if (this._debounceBrightnessId) {
            GLib.source_remove(this._debounceBrightnessId);
            this._debounceBrightnessId = 0;
        }
        if (this._debounceContrastId) {
            GLib.source_remove(this._debounceContrastId);
            this._debounceContrastId = 0;
        }
        if (this._debounceFontId) {
            GLib.source_remove(this._debounceFontId);
            this._debounceFontId = 0;
        }
        if (this._statsTimerId) {
            GLib.source_remove(this._statsTimerId);
            this._statsTimerId = 0;
        }
        if (this._settingsFontSignalId) {
            this._interfaceSettings.disconnect(this._settingsFontSignalId);
            this._settingsFontSignalId = 0;
        }
        if (this._settingsColorSignalId) {
            this._interfaceSettings.disconnect(this._settingsColorSignalId);
            this._settingsColorSignalId = 0;
        }
        super.destroy();
    }
});

export default class MonitorKontrolTopBarExtension extends Extension {
    enable() {
        this._indicator = new MonitorTopBarIndicator(GLib.build_filenamev([this.path, 'monitorkontrol']));
        Main.panel.addToStatusArea(this.uuid, this._indicator);
    }

    disable() {
        if (this._indicator) {
            this._indicator.destroy();
            this._indicator = null;
        }
    }
}
