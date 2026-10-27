#!/bin/sh
# Uçtan uca korniş (fake pty) kılmaları: her biri ayrı ayrı doğrular.
# Kullanım: sh .e2e/probes.sh   (repo kökünden)
# Gerekli: target/release/updateo, python3, .e2e/tools/*, .e2e/fake_db
set -u
cd "$(dirname "$0")/.." || exit 1

LONGWRAP="$PWD/.e2e/tools/very/long/path/with/segments/fake_sudo"
mkdir -p .e2e/tools/very/long/path/with/segments
mkdir -p .e2e/logs
cp .e2e/fake_sudo .e2e/tools/very/long/path/with/segments/fake_sudo

fail=0
run() { # run NAME CMD...
    name=$1
    shift
    printf '%-46s ' "$name"
    if "$@" >.e2e/logs/probe.out 2>&1; then
        echo OK
    else
        echo FAIL
        sed 's/^/    /' .e2e/logs/probe.out
        fail=$((fail + 1))
    fi
}

# 1) elle ad yazımı: girdi hanesi yazılan adları gösterir, önizleme okunur
run "input line keeps typed names" \
    python3 .e2e/screen_dump.py .e2e/xdgA "x nano sed" \
    --input-cell "nano sed" --expect "pacman -Rns" --expect "Enter/Esc"

# 2) uzun UPDATEO_SUDO yolu: girdi hanesi yine yazılan adları korur
run "long sudo wrapper keeps typed names" \
    python3 .e2e/screen_dump.py .e2e/xdgA "x nano sed" --width 60 \
    --sudo "$LONGWRAP" --input-cell "nano sed" \
    --expect "…/fake_sudo pacman -Rns"

# 3) onay penceresi: engeller, gerekçeler, ters bağımlılar, komut önizleme;
#    sayaçlar gerçek durumu saysın, engelli hedef space ile seçilemesin
run "removal dialog (en): blocks + reasons" \
    env LC_ALL=en_US.UTF-8 python3 .e2e/screen_dump.py .e2e/xdgA "x base nope nano<CR><SP>" \
    --width 100 --height 30 \
    --expect "protected base/system package" \
    --expect "'nope' is not installed" \
    --expect "3 target(s) · 1 selected · 2 blocked" \
    --expect "'base' cannot be removed" \
    --expect "required by: flex" \
    --expect "command: "

# 4) aynı pencere Türkçe: kullanıcı metni yerinde
run "removal dialog (tr): korunanan paketler" \
    env LC_ALL=tr_TR.UTF-8 python3 .e2e/screen_dump.py .e2e/xdgA "x base nope<CR>" \
    --width 100 --height 30 \
    --expect "korunan taban/sistem paketi" \
    --expect "'nope' kurulu değil" \
    --expect "0 seçili · 2 engelli"

# 5) seçenek enjeksiyonu reddi: '-' ile başlayan ad komuta dönüştürülmez ve
#    gerekçe giriş satırında, yazılan adın yanında görünür (durum satırı yok)
run "option-like name is refused" \
    env LC_ALL=en_US.UTF-8 python3 .e2e/screen_dump.py .e2e/xdgA "x -Rns nope<CR>" \
    --width 100 --height 30 \
    --expect "may not start with" \
    --expect "'-Rns'"
test -s .e2e/sudo.log && { echo "    FAIL: sudo.log dolu olmalıydı"; fail=$((fail + 1)); }

# 5b) aynı reddi Türkçe: gerekçe Türkçe ve giriş satırında
run "option-like name refused (tr)" \
    env LC_ALL=tr_TR.UTF-8 python3 .e2e/screen_dump.py .e2e/xdgA "x -Rns nope<CR>" \
    --width 100 --height 30 \
    --expect "geçersiz ad '-Rns'" --expect "kaldırma"
test -s .e2e/sudo.log && { echo "    FAIL: sudo.log dolu olmalıydı"; fail=$((fail + 1)); }

# 6) uçtan uca kaldırma: onay + y -> sarmalayıcıya tam komut gider; kalıcı iz
#    geçmiş sekmesinde ("removed" + hedef adlar) çünkü durum satırını taze
#    tarama birkaç tik içinde ezer
run "confirmed removal reaches the wrapper" \
    env LC_ALL=en_US.UTF-8 python3 .e2e/screen_dump.py .e2e/xdgA "x nano sed<CR>y<BACKTAB>" \
    --width 100 --height 30 --expect "removed" --expect "nano sed" \
    --log "FAKE-SUDO: pacman -Rns nano sed"

# 7) iptal: Esc ile pencere kapanır, hiçbir şey çalıştırılmaz
run "cancelled removal runs nothing" \
    env LC_ALL=en_US.UTF-8 python3 .e2e/screen_dump.py .e2e/xdgA "x nano sed<CR><ESC>" \
    --width 100 --height 30 --expect "Removal cancelled"
if test -s .e2e/sudo.log; then
    echo "    FAIL: iptal sonrası sudo.log dolu: $(cat .e2e/sudo.log)"
    fail=$((fail + 1))
fi

# 8) kurulum onay sormaz: pacman'e --noconfirm ile gidilir, Y beklenmez
run "install skips the Y confirmation" \
    env LC_ALL=en_US.UTF-8 python3 .e2e/screen_dump.py .e2e/xdgA "i nano<CR>" \
    --width 100 --height 30 \
    --log "FAKE-SUDO: pacman -S --noconfirm nano"

# 9) canlı filtre: yalnız eşleşen satır kalır, sekme rozeti toplamı korur
run "live filter narrows the list" \
    env LC_ALL=en_US.UTF-8 python3 .e2e/screen_dump.py .e2e/xdgA "/sed<CR>" \
    --width 100 --height 30 \
    --expect "filter: sed" --expect "4.10-2.1 -> 4.11-1" --reject "nano"

# 10) paket arama + ayrıntı: pacman -Ss sonucu ve pacman -Si detayı
run "package search opens details" \
    env LC_ALL=en_US.UTF-8 python3 .e2e/screen_dump.py .e2e/xdgA "s nano<CR><CR>" \
    --width 100 --height 30 \
    --expect "extra/nano" --expect "Small and friendly text editor" \
    --expect "details · extra/nano" --expect "Repository      : extra"

# 11) uygulama onayı: plan görünür, henüz komut çalışmaz
run "apply dialog previews the plan" \
    env LC_ALL=en_US.UTF-8 python3 .e2e/screen_dump.py .e2e/xdgA "u" \
    --width 100 --height 30 \
    --expect "Apply updates" --expect "pacman -Syu" --expect "y/Enter apply"
test -s .e2e/sudo.log && { echo "    FAIL: önizlemede sudo.log dolu"; fail=$((fail + 1)); }

# 12) uygulama iptali: Esc pencereyi kapatır, hiçbir şey çalıştırmaz
run "apply dialog cancels without running" \
    env LC_ALL=en_US.UTF-8 python3 .e2e/screen_dump.py .e2e/xdgA "u<ESC>" \
    --width 100 --height 30 --expect "Apply cancelled."
test -s .e2e/sudo.log && { echo "    FAIL: iptal sonrası sudo.log dolu"; fail=$((fail + 1)); }

# 13) onaylanan uygulama: seçili paketler sarmalayıcıya gider
run "confirmed apply reaches the wrapper" \
    env LC_ALL=en_US.UTF-8 python3 .e2e/screen_dump.py .e2e/xdgA "uy" \
    --width 100 --height 30 \
    --log "FAKE-SUDO: pacman -Syu nano sed"

# 14) geniş uçbirimde ayrıntı sağ panelde açılır (arama yokken)
run "details sidebar opens on wide terminal" \
    env LC_ALL=en_US.UTF-8 python3 .e2e/screen_dump.py .e2e/xdgA "d" \
    --width 100 --height 30 \
    --expect "details · nano" --expect "Name            : nano" \
    --expect "Repository      : extra"

# 15) arama sonuçlarında ok tuşları popup'ı kapatmaz; Down ikinci sonucu seçer
run "search arrows keep the popup open" \
    env LC_ALL=en_US.UTF-8 python3 .e2e/screen_dump.py .e2e/xdgA "s e<CR><DOWN><CR>" \
    --width 100 --height 30 \
    --expect "details · extra/nano" --expect "Name            : nano"

echo "----"
if [ "$fail" -eq 0 ]; then
    echo "ALL PROBES OK"
else
    echo "$fail PROBE FAILED"
fi
exit "$fail"
