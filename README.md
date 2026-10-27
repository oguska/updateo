# updateo

[English](README.en.md) · Türkçe

Arch tabanlı sistemler (Arch, CachyOS, Manjaro…) için tek arayüzden güncelleme
yöneticisi: **pacman (repo) + AUR (yay) + Flatpak** güncellemelerini tek TUI'de
listeler, paket paket seçmene olanak tanır ve güncellemeleri doğrudan uygular.

<img width="1062" height="604" alt="image" src="https://github.com/user-attachments/assets/5780f430-a5d0-4d42-98e4-75578e709082" />
## Özellikler

- Üç sekme: `pacman`, `AUR`, `Flatpak` — her birinde seçili/toplam rozeti
- Flatpak'ta uygulamalar ile runtime/uzantılar ayrılır: runtime'lar sonra
  gelir, `(runtime)` etiketi ve kurulu sürümle (`eski -> yeni`) gösterilir
- Dördüncü sekme `geçmiş`: uygulanan güncellemelerin zaman damgalı kaydı
  (`~/.local/share/updateo/history.db` — SQLite; eski `history.jsonl` ilk
  açılışta bir kez içe aktarılır)
- Paket paket seçim (`space`), sekmede tümünü seç/kaldır (`a`)
- `/` ile geçerli sekmede canlı filtre: ad, görünen ad ve sürüm alanlarında
  anında daraltır; Enter filtreyi korur, Esc temizler
- `s` ile paket arama: geçerli kaynağın `pacman -Ss` / `yay -Ss` /
  `flatpak search` sonuçları popup'ta; Enter ayrıntı, Ctrl+I seçili sonucu kurar
- `d` ile paket ayrıntısı: geniş uçbirimde sağ panel, dar uçbirimde popup
  (`pacman -Qi/-Si`, `yay -Si`, `flatpak info`)
- Görsel yenileme: `tui-scrollbar` ile liste/geçmiş kaydırma çubuğu, geniş
  uçbirimde big-text başlık, kontrol sırasında braille spinner + `tamamlanan/toplam`
  + kalan süre tahmini
- `u`/`Enter` ile uygulama: önce kaynak bazında plan onay penceresi açılır
  (`y`/Enter uygular); onaydan sonra TUI geçici olarak askıya alınır;
  `sudo pacman -Syu`, `yay -S` ve `flatpak update` gerçek TTY'de, tüm
  etkileşimli onaylarıyla (parola sorusu, pacman çatışma onayları, yay derleme
  akışı) çalışır
- Tek paket kurma: `i` alt barda isim girişi açar; Enter mevcut sekmenin
  kurucusunu (`sudo pacman -S --noconfirm` / `yay -S --noconfirm` /
  `flatpak install -y`) gerçek TTY'de çalıştırır — kurulum onayı (Y) sorulmaz,
  yalnız sudo parolası gerekebilir; sonuç geçmişe yazılır
- Kontroller paralel thread'lerde; ağ hatası tek kaynağı düşürür, arayüzü kilitlemez
- Eylemli masaüstü bildirimi: güncelleme varsa “Şimdi güncelle” düğmesi TUI'yi
  masaüstü uçbiriminde açar; `notify-send` yoksa journal'a düşer
- `~/.config/updateo/config.json` ile bildirim, onaysız güncelleme, günlük
  kontrol saati ve açılış kontrolü kalıcı olarak ayarlanır
- Tek statik binary, runtime bağımlılığı yok (SQLite gömülü)
- Arayüz dili sistem dilinden seçilir: `LC_ALL` → `LC_MESSAGES` → `LANG`;
  `tr` öneki Türkçe, diğer her şey İngilizce

## Komutlar

```
updateo                        TUI
updateo check [--notify]       TUI'siz kontrol; --notify eylemli masaüstü bildirimi
updateo update --yes [--notify]  tüm güncellemeleri onaysız uygula (otomasyon)
updateo configure [--notify yes|no] [--unattended yes|no]
                  [--check-time SS:DD] [--autostart yes|no]
updateo install-timer          systemd-user günlük zamanlayıcısı (config saati)
updateo uninstall-timer        zamanlayıcıyı kaldır
updateo install-autostart      açılışta arka plan kontrolü kur
updateo uninstall-autostart    açılış kontrolünü kaldır
updateo --version              sürümü yazdır
```

Zamanlayıcı kurulunca günlük `check --notify` çalışır; güncelleme varsa
`notify-send` ile bildirim gider (yoksa systemd journal'a düşer). Bildirimdeki
**Şimdi güncelle** düğmesi updateo TUI'yi masaüstü uçbiriminde açar; uçbirim
otomatik bulunamazsa `UPDATEO_TERMINAL=/yol/uçbirim` ile belirtilebilir.
Mevcut bir zamanlayıcıyı güncellemek için `updateo install-timer` yeniden
çalıştırılmalıdır (birim `KillMode=process` ile açılan TUI'yi korur).

## Tuşlar

```
j / k / ↑ / ↓   hareket          PageUp/PageDown  10 atla
space           seçim            a                sekmede tümünü seç
1 / 2 / 3 / 4   sekmeler (4 = geçmiş)   Tab / Ok tuşları  sekme değiştir
r               yeniden kontrol  R                db tazele (sudo pacman -Sy)
/               canlı filtre (Enter koru, Esc temizle)
s               paket ara (yaz, Enter ara/ayrıntı, ↑/↓ seç, Ctrl+I kur, Esc kapat)
d               ayrıntı paneli (j/k kaydır, d/Esc kapat)
u / Enter       seçili paketleri güncelle (önce plan onayı; y/Enter uygula)
q / Esc         çık
i               tek paket kur (sudo pacman -S --noconfirm /
                yay -S --noconfirm / flatpak install -y; onay sormaz,
                Enter başlatır, Esc vazgeçer)
x               kaldırılacak ad/ları yaz (Enter onay penceresini açar)
X               seçili satırları (seçili yoksa imleçtekini) kaldırmaya hedefle
```

Filter yalnız görünümü daraltır; gizlenen satırların seçimi korunur. Arama
popup'ında yazılan sorgu `pacman -Ss` / `yay -Ss` / `flatpak search` ile
çalışır; `Enter` sorgu değiştiyse arar, değişmediyse seçili sonucun ayrıntısını
açar. `Ctrl+I` seçili sonucu geldiği kaynağın kurucusuyla kurar.

Kaldırma onay penceresi: her hedef için kurulu sürüm, `pactree -r` ters
bağımlıları (Flatpak'ta o runtime'ı kullanan uygulamalar), uyarılar ve
engelleme gerekçeleri listelenir; `base`, `base-devel`, `filesystem`,
`pacman`, `mkinitcpio`, `systemd` gibi korunan paketler ile kurulu olmayan
adlar seçime kapalıdır. Üstte
sayaç, altta çalıştırılacak komut sabit kalır; `space` seçim, `a` tümünü,
`y` kaldırma (TUI askıya alınır, `sudo pacman -Rns` / `flatpak uninstall`
gerçek TTY'de parola/onaylarıyla koşar), `Esc` vazgeçme. Sonuç geçmişe yazılır.

## Derleme

```sh
cargo build --release
./target/release/updateo
```

Kurulum: `sudo install -Dm755 target/release/updateo /usr/local/bin/updateo`

## Bootstrap

`./bootstrap.sh` etkileşimli olarak dört soru sorar, release binary'yi derler,
`/usr/local/bin/updateo` altına kurar, `config.json` yazar ve systemd
zamanlayıcısını/açılış kontrolünü etkinleştirir:

1. Masaüstü bildirimi gösterilsin mi?
2. Güncellemeler onaysız uygulanmalı mı? (sudo NOPASSWD gerekir)
3. Günlük kontrol saati (varsayılan `09:30`)
4. Oturum açılışında arka plan kontrolü çalışsın mı?

Etkileşimsiz kurulum için ortam değişkenleri kullanılabilir:

```sh
UPDATEO_ASSUME_YES=1 UPDATEO_NOTIFY=yes UPDATEO_UNATTENDED=no \
UPDATEO_CHECK_TIME=09:30 UPDATEO_AUTOSTART=yes ./bootstrap.sh
```

Hedef dizin `UPDATEO_BINDIR` ile değiştirilebilir. Sürüm `Cargo.toml` içindeki
`package.version` değerinden gelir; `updateo --version` ve TUI başlığı aynı
değeri gösterir. Değişiklikler `CHANGELOG.md` içinde tutulur.

## Nasıl çalışıyor

| Kaynak  | Kontrol                          | Uygulama                 | Arama / ayrıntı |
|---------|----------------------------------|--------------------------|-----------------|
| pacman  | `checkupdates` (pacman-contrib), yoksa `pacman -Qu` | `sudo pacman -Syu <pkgler>` | `pacman -Ss` / `pacman -Qi`, `pacman -Si depo/ad` |
| AUR     | `yay -Qua`                       | `yay -S <pkgler>`        | `yay -Ss` / `yay -Si` |
| Flatpak | `flatpak remote-ls --updates`    | `flatpak update <refler>` | `flatpak search` / `flatpak info` |

Notlar:
- Repo listesi önbellekteki veritabanlarına bakar; en taze sonuç için `R` ile
  `sudo pacman -Sy` yapıp yeniden kontrol edebilirsiniz (dağıtımın mirror
  güncelleyicisi de kullanılabilir).
- Kök yetkisi gereken komutlar varsayılan olarak `sudo` ile koşar;
  `UPDATEO_SUDO=doas updateo` ile değiştirilebilir (`UPDATEO_SUDO="doas -n"`
  gibi önek argümanlar da desteklenir).
- AUR sorgusu ağ ister (yay, AUR RPC API'sine bakar).
- `s` araması geçerli sekmenin kaynağında çalışır; AUR sonuçlarında repo
  `aur` değilse sonuç pacman paketi sayılır. `d`/Enter ayrıntısı seçili
  sürümün kaynağına göre `-Si`, `-Qi` veya `flatpak info` kullanır.
- `u` planı seçili paketleri kaynak bazında gruplar; onay penceresinde her
  kaynağın çalıştırılacak komutu görünür. Filter yalnız görünümü daralttığı
  için gizlenen seçili paketler de plana girer.
- `i` ile kurulumda pacman/yay `--noconfirm`, flatpak `-y` kullanır: kurulum
  onayı sorulmaz, yalnız sudo parolası gerekebilir.
- Bildirim eylemi 45 saniye görünür kalır; tıklanınca TUI açılır (sudo/pacman
  onayları yine gerçek uçbirimde sorulur). Masaüstü bildirim servisi eylemleri
  desteklemiyorsa bildirim yalnız bilgi olarak kalır.
- `notify=false` iken `check --notify` sessiz kalır. `unattended=true` ise
  zamanlayıcı `update --yes --notify` çalıştırır; TTY olmadığı için sudo'nun
  parolasız olması (NOPASSWD veya polkit) gerekir. AUR derlemeleri de onaysız
  akar; riskli olduğundan varsayılan kapalıdır.
- `autostart=true` TUI'yi açmaz; oturum açılışında arka planda `check --notify`
  çalıştırır. Ayar `~/.config/autostart/updateo.desktop` dosyasına yazılır.
- Flatpak uygulamaları sistem kuruluma göre polkit/parola sorabilir.

## Yol haritası

- [x] `r` sırasında db tazeleme seçeneği (`sudo pacman -Sy`) — `R` tuşu
- [x] Güncelleme geçmişi (log) sekmesi — `4` tuşu, JSONL kaydı
- [x] flatpak runtime/uzantı ayrımı — `(runtime)` etiketi, sıralamada sonra,
  kurulu sürüm gösterimi (`flatpak list` eşleşmesi ref'in ilk bölümüyle)
- [x] systemd timer ile arka plan bildirimi — `updateo install-timer` + `check --notify`;
      bildirimdeki **Şimdi güncelle** düğmesi TUI'yi uçbirimde açar
- [x] Geçmişin JSONL yerine SQLite'a taşınması (`history.db`, otomatik içe aktarma)
- [x] İngilizce arayüz — sistem diline göre otomatik (TR/EN)
- [x] Paket kurma: `i` → isim girişi (tui-input) → `pacman -S --noconfirm` /
      `yay -S --noconfirm` / `flatpak install -y` mevcut askıya-alma akışıyla;
      sonuç geçmişe yazılır
- [x] Paket kaldırma: `x` / `X` → onay penceresi (tui-popup) + `pactree -r`
      ters-bağımlılık özeti + `base`/`base-devel`/`systemd` koruması +
      kurulu-olmayan engeli → `sudo pacman -Rns` / `flatpak uninstall`
- [x] Paket arama ve `/` canlı filtre: `pacman -Ss` / `yay -Ss` çıktıları,
      detay paneli (`pacman -Qi/-Si`, `flatpak info`)
- [x] Görsel yenileme: tui-scrollbar ile akıcı kaydırma, kontrol sırasında
      animasyonlu spinner + eta, popup'lı onay akışı, big-text başlık
      (hepsi ratatui 0.30.2 ile uyumu test edildi)
- [x] Etkileşimli bootstrap (`bootstrap.sh`), kalıcı `config.json`, yapılandırılabilir
      kontrol saati, onaysız güncelleme ve açılış kontrolü
- [x] Sürüm sistemi: `Cargo.toml` → `--version` / TUI başlığı, `CHANGELOG.md`
- [x] İngilizce README (`README.en.md`)

## Lisans

MIT
