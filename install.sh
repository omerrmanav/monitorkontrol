#!/usr/bin/env bash
# Monitör Kontrol - tek komutla kurulum (GNOME 45+)
# Kullanım: ./install.sh
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
UUID="monitorkontrol@github.monitorkontrol"
DEST="$HOME/.local/share/gnome-shell/extensions/$UUID"
BIN="$ROOT/target/release/monitorkontrol"

say()  { printf '\n\033[1m%s\033[0m\n' "$*"; }
warn() { printf '\033[33m⚠ %s\033[0m\n' "$*"; }
die()  { printf '\033[31m✗ %s\033[0m\n' "$*" >&2; exit 1; }

say "1/5  Gereksinimler kontrol ediliyor"
command -v ddcutil >/dev/null || die "ddcutil kurulu değil. Kur: sudo apt install ddcutil   (Fedora: sudo dnf install ddcutil, Arch: sudo pacman -S ddcutil)"
command -v gnome-extensions >/dev/null || die "GNOME Shell bulunamadı (gnome-extensions yok). Bu eklenti yalnızca GNOME içindir."

if ! lsmod | grep -q '^i2c_dev'; then
    warn "i2c-dev modülü yüklü değil. Çalıştır: sudo modprobe i2c-dev && echo i2c-dev | sudo tee /etc/modules-load.d/i2c-dev.conf"
fi
if ! id -nG | tr ' ' '\n' | grep -qx i2c; then
    warn "Kullanıcı 'i2c' grubunda değil. Çalıştır: sudo usermod -aG i2c \$USER   (sonra oturumu kapat/aç)"
fi

say "2/5  Derleniyor"
if [ ! -x "$BIN" ]; then
    if command -v cargo >/dev/null; then
        (cd "$ROOT" && cargo build --release)
    elif command -v docker >/dev/null; then
        echo "cargo yok, Docker ile derleniyor..."
        docker build --target export --output "type=local,dest=$ROOT/target/release" "$ROOT"
    else
        die "Rust (cargo) ya da Docker gerekli. Rust: https://rustup.rs"
    fi
fi
[ -x "$BIN" ] || die "Derleme başarısız."

say "3/5  Monitör DDC/CI desteği sorgulanıyor"
if "$BIN" --detect; then
    echo "✓ Monitör destekleniyor."
else
    warn "DDC/CI destekli monitör bulunamadı. Yukarıdaki izin/modül uyarılarını kontrol et;"
    warn "monitör menüsünden DDC/CI seçeneğini açmayı da dene. Kurulum yine de devam ediyor."
fi

say "4/5  Eklenti kuruluyor: $DEST"
mkdir -p "$DEST"
cp -f "$ROOT/topbar-extension/metadata.json" "$ROOT/topbar-extension/extension.js" "$DEST/"
cp -f "$BIN" "$DEST/monitorkontrol"

say "5/5  Etkinleştiriliyor"
gnome-extensions enable "$UUID" 2>/dev/null || warn "Eklenti henüz tanınmadı; oturumu kapatıp açtıktan sonra: gnome-extensions enable $UUID"

say "Bitti ✅  Wayland'da eklentinin görünmesi için oturumu bir kez kapatıp açın."
