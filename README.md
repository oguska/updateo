# updateo

Arch tabanlı sistemler (Arch, CachyOS, Manjaro…) için tek arayüzden güncelleme
yöneticisi: **pacman (repo) + AUR (yay) + Flatpak** güncellemelerini tek TUI'de
listeler, paket paket seçmene olanak tanır ve güncellemeleri doğrudan uygular.

## Özellikler

- Üç sekme: `pacman`, `AUR`, `Flatpak` — her birinde seçili/toplam rozeti
- Flatpak'ta uygulamalar ile runtime/uzantılar ayrılır: runtime'lar sonra
  gelir, `(runtime)` etiketi ve kurulu sürümle (`eski -> yeni`) gösterilir
- Dördüncü sekme `geçmiş`: uygulanan güncellemelerin zaman damgalı kaydı
  (`~/.local/share/updateo/history.db` — SQLite; eski `history.jsonl` ilk
  açılışta bir kez içe aktarılır)
- Paket paket seçim (`space`), sekmede tümünü seç/kaldır (`a`)
- `u` ile uygulama: TUI geçici olarak askıya alınır; `sudo pacman -Syu`,
  `yay -S` ve `flatpak update` gerçek TTY'de, tüm etkileşimli onaylarıyla
  (parola sorusu, pacman çatışma onayları, yay derleme akışı) çalışır
- Tek paket kurma: `i` alt barda isim girişi açar; Enter mevcut sekmenin
  kurucusunu (sudo pacman -S / yay -S / flatpak install) gerçek TTY'de
  çalıştırır, sonuç geçmişe yazılır
- Kontroller paralel thread'lerde; ağ hatası tek kaynağı düşürür, arayüzü kilitlemez
- Tek statik binary, runtime bağımlılığı yok (SQLite gömülü)
- Arayüz dili sistem dilinden seçilir: `LC_ALL` → `LC_MESSAGES` → `LANG`;
  `tr` öneki Türkçe, diğer her şey İngilizce

## Komutlar

```
updateo                        TUI
updateo check [--notify]       TUI'siz kontrol; --notify varsa masaüstü bildirimi
updateo install-timer          systemd-user günlük zamanlayıcısı (09:30, Persistent)
updateo uninstall-timer        zamanlayıcıyı kaldır
```

Zamanlayıcı kurulunca günlük `check --notify` çalışır; güncelleme varsa
`notify-send` ile bildirim gider (yoksa systemd journal'a düşer).

## Tuşlar

```
j / k / ↑ / ↓   hareket          PageUp/PageDown  10 atla
space           seçim            a                sekmede tümünü seç
1 / 2 / 3 / 4   sekmeler (4 = geçmiş)   Tab / Ok tuşları  sekme değiştir
r               yeniden kontrol  R                db tazele (sudo pacman -Sy)
u / Enter       seçili paketleri güncelle        q / Esc  çık
i               tek paket kur (sudo pacman -S / yay -S / flatpak install;
                Enter başlatır, Esc vazgeçer)
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
- [x] flatpak runtime/uzantı ayrımı — `(runtime)` etiketi, sıralamada sonra,
  kurulu sürüm gösterimi (`flatpak list` eşleşmesi ref'in ilk bölümüyle)
- [x] systemd timer ile arka plan bildirimi — `updateo install-timer` + `check --notify`
- [x] Geçmişin JSONL yerine SQLite'a taşınması (`history.db`, otomatik içe aktarma)
- [x] İngilizce arayüz — sistem diline göre otomatik (TR/EN)
- [x] Paket kurma: `i` → isim girişi (tui-input) → `pacman -S` / `yay -S` /
      `flatpak install` mevcut askıya-alma akışıyla; sonuç geçmişe yazılır
- [ ] Paket kaldırma: `X` → onay dialoğu (tui-popup) + ters-bağımlılık özeti +
      `base`/`base-devel` koruması → `pacman -Rns` / `flatpak uninstall`
- [ ] Paket arama ve `/` canlı filtre: `pacman -Ss` / `yay -Ss` çıktıları,
      detay paneli (`pacman -Qi/-Si`, `flatpak info`)
- [ ] Görsel yenileme: tui-scrollbar ile akıcı kaydırma, kontrol sırasında
      animasyonlu spinner + eta, popup'lı onay akışı, big-text başlık
      (hepsi ratatui 0.30.2 ile uyumu test edildi)

## Lisans

MIT
