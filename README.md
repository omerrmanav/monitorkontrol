# Monitör Kontrol

Linux'ta monitörünüzün **parlaklık** ve **kontrastını** (DDC/CI ile), **yazı ölçeğini**, **koyu/açık temayı** ve CPU/GPU/RAM durumunu GNOME üst barından yönetin. Monitörün fiziksel tuşlarına gerek kalmaz.

- Monitör adı ve yenileme hızı otomatik algılanır (sabit model yok)
- Parlaklık / Kontrast kaydırıcıları, yazı ölçeği (Tweaks'teki *Scaling Factor*), koyu tema anahtarı
- CPU / GPU (NVIDIA) / RAM kullanımı ve sıcaklığı, eşiklere göre yeşil-sarı-kırmızı

## Gereksinimler

| Gereksinim | Not |
|---|---|
| GNOME Shell 45 / 46 / 47 | Ubuntu 24.04+, Fedora 39+ vb. |
| `ddcutil` | Monitörle konuşan araç |
| Rust (`cargo`) **veya** Docker | Derleme için |
| DDC/CI destekli monitör | Çoğu DisplayPort/HDMI monitör destekler; OSD menüsünde DDC/CI açık olmalı |

## Hızlı kurulum

```bash
git clone <repo-adresi>
cd monitorkontrol
./install.sh
```

Betik sırasıyla: gereksinimleri denetler → derler → **monitörün DDC/CI desteğini sorgular** → eklentiyi kurar → etkinleştirir. Kurulumdan sonra oturumu bir kez kapatıp açın (Wayland).

Konum bağımsızdır; klasörü nereye klonlarsanız oraya göre çalışır.

### Bir kerelik sistem ayarı (izinler)

```bash
sudo apt install ddcutil                 # Fedora: dnf, Arch: pacman
sudo modprobe i2c-dev
echo i2c-dev | sudo tee /etc/modules-load.d/i2c-dev.conf
sudo usermod -aG i2c $USER               # sonra oturumu kapat/aç
```

### Monitörünüz destekleniyor mu?

```bash
./target/release/monitorkontrol --detect   # çıkış kodu 0: destekli, 1: yok
```

Destek yoksa eklenti menüsünde kaydırıcılar pasif görünür ve durum mesajı yazar.

## Docker hakkında

Docker bu projenin **çalışma** ortamı için uygun değildir: DDC/CI ana makinedeki `/dev/i2c-*` aygıtlarına, GNOME eklentisi ise çalışan masaüstü kabuğuna erişmek zorundadır. Docker yalnızca **derleme** için sağlanır (Rust kurmak istemeyenler için):

```bash
docker build --target export --output type=local,dest=target/release .
./install.sh
```

`install.sh`, `cargo` yoksa ve Docker varsa bunu otomatik yapar.

## Komut satırı

```
monitorkontrol --detect          DDC/CI desteğini sorgula
monitorkontrol --sync            Monitörden değerleri oku
monitorkontrol --brightness 75   Parlaklık %75
monitorkontrol --contrast 80     Kontrast %80
monitorkontrol --scale 1.25      Yazı ölçeği
```

## Kaldırma

```bash
gnome-extensions disable monitorkontrol@github.monitorkontrol
rm -rf ~/.local/share/gnome-shell/extensions/monitorkontrol@github.monitorkontrol
```

## Sorun giderme

- **Kaydırıcılar çalışmıyor:** `ddcutil detect` çıktısı boş mu? `i2c-dev` modülü ve `i2c` grubunu kontrol edin; monitör OSD'sinde DDC/CI'yi açın.
- **Eski arayüz görünüyor:** GNOME 45+ eklenti kodunu bellekte tutar; oturumu kapatıp açın.
- **GPU alanı `--`:** `nvidia-smi` yoksa (AMD/Intel) GPU değeri gösterilmez.
- Aynı anda birden fazla monitörde yalnızca ilk algılanan monitör yönetilir.

## Lisans

MIT
