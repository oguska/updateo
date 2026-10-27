# updateo

Arch tabanlı sistemler (Arch, CachyOS, Manjaro…) için tek arayüzden güncelleme
yöneticisi: **pacman (repo) + AUR (yay) + Flatpak** güncellemelerini tek TUI'de
listeler, paket paket seçmene olanak tanır ve güncellemeleri doğrudan uygular.

## Özellikler

- Üç sekme: `pacman`, `AUR`, `Flatpak` — her birinde seçili/toplam rozeti
- Dördüncü sekme `geçmiş`: uygulanan güncellemelerin zaman damgalı kaydı
  (`~/.local/share/updateo/history.jsonl`)
- Paket paket seçim (`space`), sekmede tümünü seç/kaldır (`a`)
- `u` ile uygulama: TUI geçici olarak askıya alınır; `sudo pacman -Syu`,
  `yay -S` ve `flatpak update` gerçek TTY'de, tüm etkileşimli onaylarıyla
  (parola sorusu, pacman çatışma onayları, yay derleme akışı) çalışır
- Kontroller paralel thread'lerde; ağ hatası tek kaynağı düşürür, arayüzü kilitlemez
- Tek statik binary, runtime bağımlılığı yok

## Tuşlar

```
j / k / ↑ / ↓   hareket          PageUp/PageDown  10 atla
space           seçim            a                sekmede tümünü seç
1 / 2 / 3 / 4   sekmeler (4 = geçmiş)   Tab / Ok tuşları  sekme değiştir
r               yeniden kontrol  R                db tazele (sudo pacman -Sy)
u / Enter       seçili paketleri güncelle        q / Esc  çık
```

## Derleme

```sh
cargo build --release
./target/release/updateo
```

Kurulum: `sudo install -Dm755 target/release/updateo /usr/local/bin/updateo`

## Nasıl çalışıyor

| Kaynak  | Kontrol                          | Uygulama                 |
|---------|----------------------------------|--------------------------|
| pacman  | `checkupdates` (pacman-contrib), yoksa `pacman -Qu` | `sudo pacman -Syu <pkgler>` |
| AUR     | `yay -Qua`                       | `yay -S <pkgler>`        |
| Flatpak | `flatpak remote-ls --updates`    | `flatpak update <refler>` |

Notlar:
- Repo listesi önbellekteki veritabanlarına bakar; en taze sonuç için `R` ile
  `sudo pacman -Sy` yapıp yeniden kontrol edebilirsiniz (dağıtımın mirror
  güncelleyicisi de kullanılabilir).
- Kök yetkisi gereken komutlar varsayılan olarak `sudo` ile koşar;
  `UPDATEO_SUDO=doas updateo` ile değiştirilebilir (`UPDATEO_SUDO="doas -n"`
  gibi önek argümanlar da desteklenir).
- AUR sorgusu ağ ister (yay, AUR RPC API'sine bakar).
- Flatpak uygulamaları sistem kuruluma göre polkit/parola sorabilir.

## Yol haritası

- [x] `r` sırasında db tazeleme seçeneği (`sudo pacman -Sy`) — `R` tuşu
- [x] Güncelleme geçmişi (log) sekmesi — `4` tuşu, JSONL kaydı
- [ ] flatpak çalışma saati/durum ayrımı (watch)
- [ ] systemd timer ile arka plan bildirimi

## Lisans

MIT
