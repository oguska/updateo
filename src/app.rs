//! Uygulama durumu, tuş atamaları ve çizim.

use crate::history::{self, Entry};
use crate::i18n::tr;
use crate::runner;
use crate::sources::{self, CheckResult, Source, Update};
use crossterm::event::{self, Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
use ratatui::Frame;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Paragraph};
use std::process::Command;
use std::sync::mpsc::{self, Receiver, Sender};
use std::thread;
use std::time::{Duration, Instant};
use tui_input::{Input, backend::crossterm::EventHandler};
use tui_popup::{KnownSizeWrapper, Popup};

/// Sekme sayısı: üç kaynak + geçmiş (sonuncusu).
const TAB_COUNT: usize = Source::ALL.len() + 1;

pub struct CheckMsg {
    pub source: Source,
    pub result: CheckResult,
}

/// Alt bardaki girişin ne ürettiği: `i` kurma hedefi, `x` kaldırma hedefi.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum InputMode {
    Install,
    Remove,
}

/// Onay bekleyen kaldırma: hedeflerin güvenlik durumu ve gezinme imleci.
pub struct RemovalSession {
    pub src: Source,
    pub plan: Vec<sources::Removal>,
    pub cursor: usize,
    pub scroll: usize,
}

impl RemovalSession {
    /// Kaldırılması seçili (ve engelli olmayan) hedef sayısı.
    pub fn included(&self) -> usize {
        self.plan.iter().filter(|r| r.include).count()
    }

    /// Engellenmiş hedef sayısı (korunan ya da kurulu olmayan).
    pub fn blocked(&self) -> usize {
        self.plan.iter().filter(|r| r.blocked.is_some()).count()
    }

    /// Onaylanan hedef adları (komut satırına gidecek sırayla).
    pub fn targets(&self) -> Vec<String> {
        self.plan
            .iter()
            .filter(|r| r.include)
            .map(|r| r.name.clone())
            .collect()
    }

    /// İmleçteki hedef engelli mi? Engelli hedef seçilemez.
    pub fn cursor_blocked(&self) -> bool {
        self.plan
            .get(self.cursor)
            .is_some_and(|r| r.blocked.is_some())
    }

    /// İmleci hedefler arasında hareket ettir.
    pub fn move_cursor(&mut self, delta: isize) {
        let n = self.plan.len();
        if n == 0 {
            return;
        }
        self.cursor = if delta > 0 {
            (self.cursor + delta as usize).min(n - 1)
        } else {
            self.cursor.saturating_sub((-delta) as usize)
        };
    }

    /// İmleçteki hedefi seç/çöz; engelli hedefe dokunulmaz.
    pub fn toggle_cursor(&mut self) {
        if let Some(r) = self.plan.get_mut(self.cursor)
            && r.blocked.is_none()
        {
            r.include = !r.include;
        }
    }

    /// Engelli olmayan tüm hedefleri seç ya da hepsinin seçimini kaldır.
    pub fn toggle_all(&mut self) {
        let open: Vec<usize> = self
            .plan
            .iter()
            .enumerate()
            .filter(|(_, r)| r.blocked.is_none())
            .map(|(i, _)| i)
            .collect();
        if open.is_empty() {
            return;
        }
        let all_on = open.iter().all(|&i| self.plan[i].include);
        for i in open {
            self.plan[i].include = !all_on;
        }
    }
}

pub struct App {
    updates: Vec<Update>, // kaynak + ada göre sıralı
    history: Vec<Entry>,  // güncelleme günlüğü, en yeni üstte
    tab: usize,           // Source::ALL indeksi
    sel: usize,           // aktif sekmedeki imleç (sekme-içi indeks)
    offset: usize,        // kaydırma penceresi başlangıcı
    checking: usize,      // bekleyen kontrol sayısı
    status: String,
    status_err: bool,
    /// Alt bardaki aktif giriş (`i` kurma / `x` kaldırma); None = gezinme modu.
    pkg_input: Option<Input>,
    input_mode: InputMode,
    /// Açık kaldırma onayı; None = onay penceresi yok.
    removal: Option<RemovalSession>,
    pub quit: bool,
    rx: Receiver<CheckMsg>,
    tx: Sender<CheckMsg>,
    last_check: Option<Instant>,
}

impl App {
    pub fn new() -> Self {
        let (tx, rx) = mpsc::channel();
        Self {
            updates: Vec::new(),
            history: Vec::new(),
            tab: 0,
            sel: 0,
            offset: 0,
            checking: 0,
            status: tr("Checking for updates…", "Güncellemeler kontrol ediliyor…"),
            status_err: false,
            pkg_input: None,
            input_mode: InputMode::Install,
            removal: None,
            quit: false,
            rx,
            tx,
            last_check: None,
        }
    }

    /// Üç kaynağı da paralel thread'lerde kontrol et; sonuçlar kanaldan gelir.
    pub fn start_refresh(&mut self) {
        self.status = tr("Checking for updates…", "Güncellemeler kontrol ediliyor…");
        self.status_err = false;
        self.spawn_checks();
    }

    /// Thread'leri başlat; durumu (mesajı) çağıranın yönetmesine bırak —
    /// uygulanan/senkron edilen işlemin mesajı kontrol bitene kadar görünür kalır.
    fn spawn_checks(&mut self) {
        self.checking = Source::ALL.len();
        for src in Source::ALL {
            let tx = self.tx.clone();
            thread::spawn(move || {
                let result = match src {
                    Source::Pacman => sources::check_pacman(),
                    Source::Aur => sources::check_aur(),
                    Source::Flatpak => sources::check_flatpak(),
                };
                let _ = tx.send(CheckMsg {
                    source: src,
                    result,
                });
            });
        }
    }

    fn drain(&mut self) {
        let mut got = false;
        while let Ok(msg) = self.rx.try_recv() {
            got = true;
            self.checking = self.checking.saturating_sub(1);
            match msg.result {
                Ok(list) => {
                    self.updates.retain(|u| u.source != msg.source);
                    self.updates.extend(list);
                    sources::sort_updates(&mut self.updates);
                }
                Err(e) => {
                    self.status = tr(
                        &format!("{} check failed: {e}", msg.source.label()),
                        &format!("{} kontrolü başarısız: {e}", msg.source.label()),
                    );
                    self.status_err = true;
                }
            }
        }
        if got && self.checking == 0 {
            self.last_check = Some(Instant::now());
            if !self.status_err {
                let total = self.updates.len();
                self.status = match total {
                    0 => tr(
                        "No updates — system is up to date.",
                        "Güncelleme yok — sistem güncel.",
                    ),
                    n => tr(
                        &format!("{n} updateable package(s)."),
                        &format!("{n} güncellenebilir paket."),
                    ),
                };
            }
            self.clamp();
        }
    }

    /// Aktif sekmedeki güncellemelerin global indeksleri (geçmişte boş).
    fn tab_items(&self) -> Vec<usize> {
        if self.is_hist_tab() {
            return Vec::new();
        }
        let src = Source::ALL[self.tab];
        self.updates
            .iter()
            .enumerate()
            .filter(|(_, u)| u.source == src)
            .map(|(i, _)| i)
            .collect()
    }

    fn is_hist_tab(&self) -> bool {
        self.tab == Source::ALL.len()
    }

    /// Aktif sekmedeki satır sayısı (geçmiş: kayıt sayısı).
    fn tab_len(&self) -> usize {
        if self.is_hist_tab() {
            self.history.len()
        } else {
            self.tab_items().len()
        }
    }

    fn count(&self, tab: usize) -> usize {
        if tab >= Source::ALL.len() {
            return 0;
        }
        let src = Source::ALL[tab];
        self.updates.iter().filter(|u| u.source == src).count()
    }

    fn selected_count(&self, tab: usize) -> usize {
        if tab >= Source::ALL.len() {
            return 0;
        }
        let src = Source::ALL[tab];
        self.updates
            .iter()
            .filter(|u| u.source == src && u.selected)
            .count()
    }

    fn clamp(&mut self) {
        let n = self.tab_len();
        if n == 0 {
            self.sel = 0;
            self.offset = 0;
            return;
        }
        self.sel = self.sel.min(n - 1);
    }

    fn move_sel(&mut self, delta: isize) {
        let n = self.tab_len();
        if n == 0 {
            return;
        }
        self.sel = if delta > 0 {
            (self.sel + delta as usize).min(n - 1)
        } else {
            self.sel.saturating_sub((-delta) as usize)
        };
    }

    fn toggle_current(&mut self) {
        if self.is_hist_tab() {
            return;
        }
        let items = self.tab_items();
        if let Some(&gi) = items.get(self.sel) {
            self.updates[gi].selected = !self.updates[gi].selected;
        }
    }

    /// Sekmedeki tümünü seç / tümünün seçimini kaldır.
    fn toggle_all_tab(&mut self) {
        if self.is_hist_tab() {
            return;
        }
        let src = Source::ALL[self.tab];
        let all_selected = self
            .updates
            .iter()
            .filter(|u| u.source == src)
            .all(|u| u.selected)
            && self.count(self.tab) > 0;
        for u in self.updates.iter_mut().filter(|u| u.source == src) {
            u.selected = !all_selected;
        }
    }

    fn switch_tab(&mut self, tab: usize) {
        self.tab = tab % TAB_COUNT;
        self.sel = 0;
        self.offset = 0;
        if self.is_hist_tab() {
            self.history = history::load();
        }
    }

    /// Yerel paket veritabanını `sudo pacman -Sy` ile tazele, sonra yeniden
    /// kontrol et. Repo listesi önbellekteki db'ye baktığı için taze db olmadan
    /// gözükmeyen güncellemeler böylece görünür olur.
    fn sync_db(&mut self) {
        let wrapper = std::env::var("UPDATEO_SUDO").unwrap_or_else(|_| "sudo".into());
        self.status = tr(
            &format!("Refreshing database ({wrapper} pacman -Sy)…"),
            &format!("Veritabanı tazeleniyor ({wrapper} pacman -Sy)…"),
        );
        self.status_err = false;
        let mut cmd = sources::root_command("pacman", &["-Sy"]);
        match runner::run_suspended(&mut cmd) {
            Ok(st) if st.success() => {
                self.status = tr(
                    "Database refreshed, rechecking…",
                    "Veritabanı tazelendi, yeniden kontrol ediliyor…",
                );
            }
            Ok(st) => {
                self.status = tr(
                    &format!("Database refresh failed ({st})"),
                    &format!("Veritabanı tazelenemedi ({st})"),
                );
                self.status_err = true;
                return;
            }
            Err(e) => {
                self.status = tr(
                    &format!("Cannot start database refresh: {e}"),
                    &format!("Veritabanı tazeleme başlatılamadı: {e}"),
                );
                self.status_err = true;
                return;
            }
        }
        self.spawn_checks();
    }

    /// Seçili paketleri kaynak bazında uygula. TUI askıya alınır, çocuk süreç
    /// gerçek TTY'yi kullanır (sudo parolası, pacman/yay onayları dahil).
    fn apply(&mut self) {
        let mut any = false;
        for src in Source::ALL {
            if self.updates.iter().any(|u| u.source == src && u.selected) {
                any = true;
            }
        }
        if !any {
            self.status = tr(
                "No package selected — press space to select, u to apply.",
                "Seçili paket yok — space ile seç, u ile uygula.",
            );
            self.status_err = false;
            return;
        }
        for src in Source::ALL {
            let names: Vec<String> = self
                .updates
                .iter()
                .filter(|u| u.source == src && u.selected)
                .map(|u| u.name.clone())
                .collect();
            if names.is_empty() {
                continue;
            }
            let mut cmd = match src {
                Source::Pacman => sources::root_command("pacman", &["-Syu"]),
                Source::Aur => {
                    let mut c = Command::new("yay");
                    c.arg("-S");
                    c
                }
                Source::Flatpak => {
                    let mut c = Command::new("flatpak");
                    c.arg("update");
                    c
                }
            };
            cmd.args(&names);
            match runner::run_suspended(&mut cmd) {
                Ok(st) if st.success() => history::record(src.label(), "ok", &names),
                Ok(st) => {
                    history::record(src.label(), &format!("failed ({st})"), &names);
                    self.status = tr(
                        &format!("{} update did not finish ({st})", src.label()),
                        &format!("{} güncellemesi tamamlanmadı ({st})", src.label()),
                    );
                    self.status_err = true;
                    break;
                }
                Err(e) => {
                    history::record(src.label(), &format!("could not start: {e}"), &names);
                    self.status = tr(
                        &format!("Cannot start {} update: {e}", src.label()),
                        &format!("{} güncellemesi başlatılamadı: {e}", src.label()),
                    );
                    self.status_err = true;
                    break;
                }
            }
        }
        if !self.status_err {
            self.status = tr(
                "Updates applied, rechecking…",
                "Güncellemeler uygulandı, yeniden kontrol ediliyor…",
            );
        }
        self.history = history::load();
        self.spawn_checks();
    }

    /// Paket kurma: TUI askıya alınır, kurucu gerçek TTY'yi kullanır
    /// (parola/onaylar dahil). Sonuç geçmişe yazılır, sonra yeniden kontrol.
    fn install(&mut self, names: Vec<String>) {
        if self.is_hist_tab() {
            return;
        }
        let src = Source::ALL[self.tab];
        let list = names.join(" ");
        let mut cmd = sources::install_command(src, &names);
        self.status = tr(
            &format!("Installing {list} via {}…", src.label()),
            &format!("{list}, {} ile kuruluyor…", src.label()),
        );
        self.status_err = false;
        let outcome = match runner::run_suspended(&mut cmd) {
            Ok(st) if st.success() => {
                self.status = tr(&format!("{list} installed."), &format!("{list} kuruldu."));
                "ok".to_string()
            }
            Ok(st) => {
                self.status = tr(
                    &format!("Install of {list} did not finish ({st})"),
                    &format!("{list} kurulumu tamamlanmadı ({st})"),
                );
                self.status_err = true;
                format!("failed ({st})")
            }
            Err(e) => {
                self.status = tr(
                    &format!("Cannot start install of {list}: {e}"),
                    &format!("{list} kurulumu başlatılamadı: {e}"),
                );
                self.status_err = true;
                format!("could not start: {e}")
            }
        };
        history::record(src.label(), &outcome, &names);
        self.history = history::load();
        self.spawn_checks();
    }

    /// `x` ile girilen adları / `X` ile seçili satırları kaldırma onayına taşı:
    /// her hedef sistemde sorgulanır (kurulu sürüm, ters bağımlılar, koruma).
    fn open_removal(&mut self, src: Source, names: Vec<String>) {
        let plan = sources::removal_plan(src, &names);
        let blocked = plan.iter().filter(|r| r.blocked.is_some()).count();
        self.removal = Some(RemovalSession {
            src,
            plan,
            cursor: 0,
            scroll: 0,
        });
        self.status_err = blocked > 0;
        self.status = if blocked > 0 {
            tr(
                &format!("{blocked} target(s) blocked from removal — see the dialog."),
                &format!("{blocked} hedef kaldırmada engelli — onay penceresine bak."),
            )
        } else {
            tr(
                "Review the removal plan — space toggles, y removes.",
                "Kaldırma planını incele — space seçer, y kaldırır.",
            )
        };
    }

    /// `X`: bulunulan sekmenin seçili satırlarını (seçili yoksa imleçtekini)
    /// kaldırma hedefi yap.
    fn remove_selected(&mut self) {
        if self.is_hist_tab() {
            return;
        }
        let src = Source::ALL[self.tab];
        let mut names: Vec<String> = self
            .updates
            .iter()
            .filter(|u| u.source == src && u.selected)
            .map(|u| u.name.clone())
            .collect();
        if names.is_empty() {
            let items = self.tab_items();
            if let Some(&gi) = items.get(self.sel) {
                names.push(self.updates[gi].name.clone());
            }
        }
        if names.is_empty() {
            self.status = tr(
                "Nothing to remove here — move the cursor onto a package.",
                "Kaldırılacak bir şey yok — imleci bir paketin üzerine getir.",
            );
            self.status_err = true;
            return;
        }
        self.open_removal(src, names);
    }

    /// Onaylanan hedefleri kaldır: TUI askıya alınır, `pacman -Rns` /
    /// `flatpak uninstall` gerçek TTY'de çalışır (parola/onaylar dahil).
    fn confirm_removal(&mut self) {
        let (src, names) = match self.removal.as_ref() {
            Some(sess) if sess.included() > 0 => (sess.src, sess.targets()),
            Some(_) => {
                self.status = tr(
                    "No target selected — press space on a package, then y.",
                    "Seçili hedef yok — paketin üzerinde space'e bas, sonra y.",
                );
                self.status_err = true;
                return;
            }
            None => return,
        };
        self.removal = None;
        let list = names.join(" ");
        let mut cmd = sources::remove_command(src, &names);
        self.status = tr(
            &format!("Removing {list} via {}…", src.label()),
            &format!("{list}, {} ile kaldırılıyor…", src.label()),
        );
        self.status_err = false;
        let outcome = match runner::run_suspended(&mut cmd) {
            Ok(st) if st.success() => {
                self.status = tr(&format!("{list} removed."), &format!("{list} kaldırıldı."));
                "removed".to_string()
            }
            Ok(st) => {
                self.status = tr(
                    &format!("Removal of {list} did not finish ({st})"),
                    &format!("{list} kaldırması tamamlanmadı ({st})"),
                );
                self.status_err = true;
                format!("failed ({st})")
            }
            Err(e) => {
                self.status = tr(
                    &format!("Cannot start removal of {list}: {e}"),
                    &format!("{list} kaldırması başlatılamadı: {e}"),
                );
                self.status_err = true;
                format!("could not start: {e}")
            }
        };
        history::record(src.label(), &outcome, &names);
        self.history = history::load();
        self.spawn_checks();
    }

    /// Onay penceresi açıkken tuşlar: gezinme, seçim, onay, iptal.
    fn on_removal_key(&mut self, k: KeyEvent) {
        if let Some(sess) = self.removal.as_mut() {
            match k.code {
                KeyCode::Esc | KeyCode::Char('q') => {
                    self.removal = None;
                    self.status = tr("Removal cancelled.", "Kaldırma iptal edildi.");
                    self.status_err = false;
                }
                KeyCode::Char('j') | KeyCode::Down => sess.move_cursor(1),
                KeyCode::Char('k') | KeyCode::Up => sess.move_cursor(-1),
                KeyCode::PageDown => sess.move_cursor(10),
                KeyCode::PageUp => sess.move_cursor(-10),
                KeyCode::Home => sess.cursor = 0,
                KeyCode::End => sess.move_cursor(isize::MAX),
                KeyCode::Char(' ') | KeyCode::Char('t') => {
                    if sess.cursor_blocked() {
                        let name = sess
                            .plan
                            .get(sess.cursor)
                            .map(|r| r.name.clone())
                            .unwrap_or_default();
                        self.status = tr(
                            &format!("'{name}' cannot be removed — see the reason."),
                            &format!("'{name}' kaldırılamaz — gerekçeye bak."),
                        );
                        self.status_err = true;
                    } else {
                        sess.toggle_cursor();
                    }
                }
                KeyCode::Char('a') => sess.toggle_all(),
                _ => {}
            }
        }
        if matches!(
            k.code,
            KeyCode::Enter | KeyCode::Char('y') | KeyCode::Char('Y')
        ) {
            self.confirm_removal();
        }
    }

    fn on_key(&mut self, k: KeyEvent) -> std::io::Result<()> {
        if k.modifiers == KeyModifiers::CONTROL && k.code == KeyCode::Char('c') {
            self.quit = true;
            return Ok(());
        }
        // Kaldırma onayı açıksa tuşlar yalnızca oraya gider (girdi/pencere üstü).
        if self.removal.is_some() {
            self.on_removal_key(k);
            return Ok(());
        }
        // Kurma girdisi açıksa tuşlar önce ona gider; Enter başlatır, Esc kapatır.
        if let Some(mut inp) = self.pkg_input.take() {
            let mode = self.input_mode;
            match k.code {
                KeyCode::Enter => {
                    let raw = inp.value().trim().to_string();
                    let names = match sources::split_names(&raw) {
                        Ok(v) => v,
                        Err(e) => {
                            self.status = tr(
                                &format!("Cannot parse name(s): {e}"),
                                &format!("Ad/lar çözülemedi: {e}"),
                            );
                            self.status_err = true;
                            self.pkg_input = Some(inp);
                            return Ok(());
                        }
                    };
                    if raw.is_empty() {
                        self.status = tr(
                            "Empty name — press Esc to cancel.",
                            "İsim boş — Esc ile vazgeç.",
                        );
                        self.status_err = false;
                        self.pkg_input = Some(inp);
                    } else {
                        match mode {
                            InputMode::Install => self.install(names),
                            InputMode::Remove => {
                                let src = Source::ALL[self.tab];
                                self.open_removal(src, names);
                            }
                        }
                    }
                }
                KeyCode::Esc => {
                    self.status = match mode {
                        InputMode::Install => tr("Install cancelled.", "Kurulum iptal edildi."),
                        InputMode::Remove => tr("Removal cancelled.", "Kaldırma iptal edildi."),
                    };
                    self.status_err = false;
                }
                _ => {
                    inp.handle_event(&Event::Key(k));
                    self.pkg_input = Some(inp);
                }
            }
            return Ok(());
        }
        match k.code {
            KeyCode::Char('q') | KeyCode::Esc => self.quit = true,
            KeyCode::Char('j') | KeyCode::Down => self.move_sel(1),
            KeyCode::Char('k') | KeyCode::Up => self.move_sel(-1),
            KeyCode::PageDown => self.move_sel(10),
            KeyCode::PageUp => self.move_sel(-10),
            KeyCode::Home => self.sel = 0,
            KeyCode::End => self.move_sel(isize::MAX),
            KeyCode::Char(' ') => self.toggle_current(),
            KeyCode::Char('a') => self.toggle_all_tab(),
            KeyCode::Char('r') => self.start_refresh(),
            KeyCode::Char('R') => self.sync_db(),
            KeyCode::Char('u') | KeyCode::Enter => self.apply(),
            KeyCode::Char('i') if !self.is_hist_tab() => {
                self.pkg_input = Some(Input::new(String::new()));
                self.input_mode = InputMode::Install;
                self.status = tr(
                    "Type package name — Enter installs via current tab's manager, Esc cancels.",
                    "Paket adı yaz — Enter içinde bulunulan sekmenin kurucusunu çalıştırır, Esc vazgeçer.",
                );
                self.status_err = false;
            }
            // `x` elle ad yazdırır, `X` bulunulan sekmenin seçili satırlarını hedefler.
            KeyCode::Char('x') if !self.is_hist_tab() => {
                self.pkg_input = Some(Input::new(String::new()));
                self.input_mode = InputMode::Remove;
                self.status = tr(
                    &format!(
                        "Type package name(s) to remove — Enter previews '{} …', Esc cancels.",
                        sources::remove_preview(Source::ALL[self.tab])
                    ),
                    &format!(
                        "Kaldırılacak paket adı yaz — Enter '{} …' komutunu önizler, Esc vazgeçer.",
                        sources::remove_preview(Source::ALL[self.tab])
                    ),
                );
                self.status_err = false;
            }
            KeyCode::Char('X') => self.remove_selected(),
            KeyCode::Char('1') => self.switch_tab(0),
            KeyCode::Char('2') => self.switch_tab(1),
            KeyCode::Char('3') => self.switch_tab(2),
            KeyCode::Char('4') => self.switch_tab(3),
            KeyCode::Tab => self.switch_tab(self.tab + 1),
            KeyCode::BackTab => self.switch_tab(self.tab + TAB_COUNT - 1),
            KeyCode::Left => self.switch_tab(self.tab + TAB_COUNT - 1),
            KeyCode::Right => self.switch_tab(self.tab + 1),
            _ => {}
        }
        Ok(())
    }

    pub fn handle_events(&mut self) -> std::io::Result<()> {
        if event::poll(Duration::from_millis(120))?
            && let Event::Key(k) = event::read()?
            && k.kind == KeyEventKind::Press
        {
            self.on_key(k)?;
        }
        self.drain();
        Ok(())
    }

    // ------------------------------- çizim -------------------------------

    /// Geçmiş sekmesi: uygulanan güncellemeler, en yeni üstte.
    fn draw_history(&mut self, f: &mut Frame, area: Rect, block: Block) {
        if self.history.is_empty() {
            f.render_widget(
                Paragraph::new(tr(
                    "no updates applied yet — entries appear here after 'u'",
                    "henüz güncelleme uygulanmadı — 'u' sonrası kayıtlar burada",
                ))
                .block(block)
                .style(Style::new().fg(Color::DarkGray)),
                area,
            );
            return;
        }
        let n = self.history.len();
        let inner_h = area.height.saturating_sub(2) as usize;
        if self.sel < self.offset {
            self.offset = self.sel;
        }
        if inner_h > 0 && self.sel >= self.offset + inner_h {
            self.offset = self.sel - inner_h + 1;
        }
        let pkgs_w = (area.width as usize).saturating_sub(52).max(12);
        let mut lines: Vec<Line> = Vec::new();
        for (idx, e) in self.history.iter().enumerate() {
            if idx < self.offset || idx >= self.offset + inner_h {
                continue;
            }
            let cursor = if idx == self.sel { ">" } else { " " };
            let row = format!(
                "{cursor} {}  {:<7} {:<24} {}",
                e.time,
                e.source,
                truncate(&e.result, 22),
                truncate(&e.pkgs.join(" "), pkgs_w),
            );
            let mut style = Style::new().fg(if history::is_success(&e.result) {
                Color::Green
            } else {
                Color::Red
            });
            if idx == self.sel {
                style = style.bg(Color::DarkGray).add_modifier(Modifier::BOLD);
            }
            lines.push(Line::from(Span::styled(row, style)));
        }
        if self.offset > 0
            && let Some(first) = lines.first_mut()
        {
            first.spans.push(Span::styled(
                format!("  ↑{}", self.offset),
                Style::new().fg(Color::DarkGray),
            ));
        }
        let hidden = n.saturating_sub(self.offset + inner_h);
        if hidden > 0 && lines.len() < inner_h {
            lines.push(Line::from(Span::styled(
                format!("  ↓{hidden}", hidden = hidden),
                Style::new().fg(Color::DarkGray),
            )));
        }
        f.render_widget(Paragraph::new(lines).block(block), area);
    }

    pub fn draw(&mut self, f: &mut Frame) {
        let area = f.area();
        let [tabs_area, list_area, status_area] = Layout::vertical([
            Constraint::Length(1),
            Constraint::Min(3),
            Constraint::Length(1),
        ])
        .areas(area);

        // Sekme çubuğu: "pacman (seçili/toplam)" rozetleri + kısayol ipucu
        let mut spans = Vec::new();
        for i in 0..TAB_COUNT {
            let text = if i < Source::ALL.len() {
                format!(
                    " {} ({}/{}) ",
                    Source::ALL[i].label(),
                    self.selected_count(i),
                    self.count(i)
                )
            } else {
                tr(" history ", " geçmiş ")
            };
            let style = if i == self.tab {
                Style::new()
                    .fg(Color::Black)
                    .bg(Color::Cyan)
                    .add_modifier(Modifier::BOLD)
            } else {
                Style::new().fg(Color::DarkGray)
            };
            spans.push(Span::styled(text, style));
        }
        spans.push(Span::styled(
            tr(
                "   j/k move · space select · a all · r refresh · R refresh db · i install · x/X remove · u apply · q quit",
                "   j/k hareket · space seçim · a tümünü · r yenile · R db tazele · i kur · x/X kaldır · u güncelle · q çık",
            ),
            Style::new().fg(Color::DarkGray),
        ));
        f.render_widget(Paragraph::new(Line::from(spans)), tabs_area);

        // Liste
        let block = Block::bordered().title(tr(
            " updateo · Arch updater ",
            " updateo · Arch güncelleyici ",
        ));
        let items = self.tab_items();
        if self.is_hist_tab() {
            self.draw_history(f, list_area, block);
        } else if items.is_empty() {
            let msg = if self.checking > 0 {
                tr("checking…", "kontrol ediliyor…")
            } else {
                tr(
                    "no packages to update — press 'r' to refresh",
                    "güncellenecek paket yok — 'r' ile yenile",
                )
            };
            f.render_widget(
                Paragraph::new(msg).style(Style::new().fg(Color::DarkGray)),
                list_area.inner(ratatui::layout::Margin::new(2, 0)),
            );
        } else {
            let inner_h = list_area.height.saturating_sub(2) as usize;
            // pencere: imleci görünür tut
            if self.sel < self.offset {
                self.offset = self.sel;
            }
            if inner_h > 0 && self.sel >= self.offset + inner_h {
                self.offset = self.sel - inner_h + 1;
            }
            let name_w = items
                .iter()
                .map(|&i| self.updates[i].display.chars().count())
                .max()
                .unwrap_or(8)
                .min((list_area.width as usize).saturating_sub(40))
                .max(10);

            let mut lines = Vec::new();
            for (idx, &gi) in items.iter().enumerate() {
                if idx < self.offset || idx >= self.offset + inner_h {
                    continue;
                }
                let u = &self.updates[gi];
                let cursor = if idx == self.sel { ">" } else { " " };
                let check = if u.selected { "[x]" } else { "[ ]" };
                let ver = if u.from.is_empty() {
                    u.to.clone()
                } else {
                    format!("{} -> {}", u.from, u.to)
                };
                let ver = if u.runtime {
                    format!("{ver}  (runtime)")
                } else {
                    ver
                };
                let row = format!(
                    "{cursor} {check} {:<name_w$}  {ver}",
                    truncate(&u.display, name_w),
                    name_w = name_w
                );
                let mut style = Style::new().fg(Color::White);
                if idx == self.sel {
                    style = style.bg(Color::DarkGray).add_modifier(Modifier::BOLD);
                }
                if !u.selected {
                    style = style.fg(Color::DarkGray);
                }
                lines.push(Line::from(Span::styled(row, style)));
            }
            // kaydırma göstergeleri
            if self.offset > 0
                && let Some(first) = lines.first_mut()
            {
                first.spans.push(Span::styled(
                    format!("  ↑{}", self.offset),
                    Style::new().fg(Color::DarkGray),
                ));
            }
            let hidden = items.len().saturating_sub(self.offset + inner_h);
            if hidden > 0 && lines.len() < inner_h {
                lines.push(Line::from(Span::styled(
                    format!("  ↓{hidden}", hidden = hidden),
                    Style::new().fg(Color::DarkGray),
                )));
            }
            f.render_widget(Paragraph::new(lines).block(block), list_area);
        }

        // Durum satırı — kurma girdisi aktifse o satır giriş satırına dönüşür
        if let Some(inp) = &self.pkg_input {
            let src = Source::ALL[self.tab];
            let removing = self.input_mode == InputMode::Remove;
            let prefix = format!(
                " {} ",
                if removing {
                    sources::remove_preview(src)
                } else {
                    sources::install_preview(src)
                }
            );
            let (hint, label, label_color) = if removing {
                (
                    tr("Enter review · Esc cancel", "Enter incele · Esc vazgeç"),
                    tr(" remove ", " kaldırma "),
                    Color::Red,
                )
            } else {
                (
                    tr("Enter install · Esc cancel", "Enter kur · Esc vazgeç"),
                    tr(" install ", " kurma "),
                    Color::Cyan,
                )
            };
            let avail = (status_area.width as usize)
                .saturating_sub(
                    prefix.chars().count() + hint.chars().count() + label.chars().count() + 4,
                )
                .max(1);
            let chars: Vec<char> = inp.value().chars().collect();
            let start = inp.visual_scroll(avail);
            let cur = inp.cursor();
            let mut spans = vec![
                Span::styled(
                    label,
                    Style::new()
                        .fg(Color::Black)
                        .bg(label_color)
                        .add_modifier(Modifier::BOLD),
                ),
                Span::styled(
                    prefix,
                    Style::new().fg(label_color).add_modifier(Modifier::BOLD),
                ),
            ];
            for (i, c) in chars.iter().enumerate().skip(start).take(avail) {
                let st = if i == cur {
                    Style::new().bg(Color::White).fg(Color::Black)
                } else {
                    Style::new().fg(Color::White)
                };
                spans.push(Span::styled(c.to_string(), st));
            }
            if cur >= start && cur < start + avail && cur >= chars.len() {
                spans.push(Span::styled(
                    " ",
                    Style::new().bg(Color::White).fg(Color::Black),
                ));
            }
            spans.push(Span::styled(
                format!("   {hint}"),
                Style::new().fg(Color::DarkGray),
            ));
            f.render_widget(Paragraph::new(Line::from(spans)), status_area);
            return;
        }
        let style = if self.status_err {
            Style::new().fg(Color::Red)
        } else if self.checking > 0 {
            Style::new().fg(Color::Yellow)
        } else {
            Style::new().fg(Color::Green)
        };
        let mut status = self.status.clone();
        if let Some(t) = self.last_check {
            status.push_str(&tr(
                &format!("  ·  last check {}s ago", t.elapsed().as_secs()),
                &format!("  ·  son kontrol {}sn önce", t.elapsed().as_secs()),
            ));
        }
        f.render_widget(
            Paragraph::new(Line::from(Span::styled(status, style))),
            status_area,
        );

        // Kaldırma onayı en üstte: hedefler, uyarılar ve çalıştırılacak komut.
        if self.removal.is_some() {
            self.draw_removal(f, area);
        }
    }

    /// Kaldırma onay penceresi: her hedef için kurulu sürüm, engelleme
    /// gerekçesi, ters bağımlılar / runtime kullanıcıları ve komut önizlemesi.
    fn draw_removal(&mut self, f: &mut Frame, area: Rect) {
        let Some(sess) = &mut self.removal else {
            return;
        };
        let src = sess.src;
        // Küçük uçbirimde pencere taşmasın: kenarlıklar + pay için 6 hane düş.
        let body_w = ((area.width as usize).saturating_sub(6)).clamp(18, 96);
        // Gövde yüksekliği en fazla uçbirime sığan kadar; içerik azsa küçülür.
        let max_h = (area.height as usize).saturating_sub(6).clamp(4, 24);

        // Sayaç satırı sabit: liste kaydırılırken de hep görünür kalır.
        let counts = tr(
            &format!(
                " · {} target(s) · {} selected · {} blocked ",
                sess.plan.len(),
                sess.included(),
                sess.blocked()
            ),
            &format!(
                " · {} hedef · {} seçili · {} engelli ",
                sess.plan.len(),
                sess.included(),
                sess.blocked()
            ),
        );

        // Kaydırılabilir liste: ipucu satırı + hedefler ve gerekçe/uyarı satırları.
        let mut lines: Vec<Line> = Vec::new();
        lines.push(Line::from(Span::styled(
            tr(
                "j/k move · space select · a all · y remove · Esc cancel",
                "j/k gez · space seç · a tümünü · y kaldır · Esc vazgeç",
            ),
            Style::new().fg(Color::DarkGray),
        )));

        // İmlecin `lines` içindeki satır konumu: 0. satır ipucu satırı.
        let mut cursor_line = 1usize;
        for (i, r) in sess.plan.iter().enumerate() {
            if i == sess.cursor {
                break;
            }
            cursor_line += 1 + usize::from(r.blocked.is_some()) + usize::from(r.note.is_some());
            if !r.dependents.is_empty() {
                cursor_line += 1;
            }
        }

        for (i, r) in sess.plan.iter().enumerate() {
            let cursor = if i == sess.cursor { ">" } else { " " };
            let check = if r.blocked.is_some() {
                "[-]"
            } else if r.include {
                "[x]"
            } else {
                "[ ]"
            };
            let ver = r.installed.clone().unwrap_or_else(|| "?".into());
            let row = format!("{cursor} {check} {}  {ver}", r.name);
            let color = if r.blocked.is_some() {
                Color::Red
            } else if r.include {
                Color::White
            } else {
                Color::DarkGray
            };
            let mut style = Style::new().fg(color);
            if i == sess.cursor {
                style = style.bg(Color::DarkGray).add_modifier(Modifier::BOLD);
            }
            lines.push(Line::from(Span::styled(row, style)));

            if let Some(why) = &r.blocked {
                lines.push(Line::from(Span::styled(
                    format!("      x {why}"),
                    Style::new().fg(Color::Red),
                )));
            }
            if let Some(note) = &r.note {
                lines.push(Line::from(Span::styled(
                    format!("      ? {note}"),
                    Style::new().fg(Color::DarkGray),
                )));
            }
            if !r.dependents.is_empty() {
                let label = if src == Source::Flatpak {
                    tr("used by", "kullananlar")
                } else {
                    tr("required by", "bunu isteyenler")
                };
                let shown: Vec<&str> = r.dependents.iter().take(6).map(String::as_str).collect();
                let more = r.dependents.len().saturating_sub(shown.len());
                let tail = if more > 0 {
                    format!(" (+{more})")
                } else {
                    String::new()
                };
                lines.push(Line::from(Span::styled(
                    format!("      ! {label}: {}{tail}", shown.join(", ")),
                    Style::new().fg(Color::Yellow),
                )));
            }
        }

        let footer = {
            let targets = sess.targets();
            let command = if targets.is_empty() {
                tr("(no target selected)", "(seçili hedef yok)").to_string()
            } else {
                format!("{} {}", sources::remove_preview(src), targets.join(" "))
            };
            // Komut satırı pencereye sığmalı; uzun listelerde sonu kısaltılır.
            let label = format!("{} ", tr("command:", "komut:"));
            let room = body_w.saturating_sub(label.chars().count() + 1);
            let command = truncate(&command, room);
            Line::from(vec![
                Span::styled(label, Style::new().fg(Color::DarkGray)),
                Span::styled(
                    command,
                    Style::new()
                        .fg(if targets.is_empty() {
                            Color::Red
                        } else {
                            Color::Cyan
                        })
                        .add_modifier(Modifier::BOLD),
                ),
            ])
        };

        // Gövde üç parça: sabit sayaç, kayan liste, sabit komut önizlemesi.
        let body_h = (lines.len() + 2).min(max_h).max(4);
        let window = body_h.saturating_sub(2).max(1);
        if cursor_line >= sess.scroll + window {
            sess.scroll = cursor_line + 1 - window;
        }
        if cursor_line < sess.scroll {
            sess.scroll = cursor_line;
        }
        sess.scroll = sess.scroll.min(lines.len().saturating_sub(1));
        let mut shown = vec![Line::from(Span::styled(
            truncate(counts.trim(), body_w),
            Style::new().fg(Color::Yellow).add_modifier(Modifier::BOLD),
        ))];
        shown.extend(lines[sess.scroll..(sess.scroll + window).min(lines.len())].to_vec());
        shown.push(footer);

        let body = KnownSizeWrapper {
            inner: Paragraph::new(shown),
            width: body_w,
            height: body_h,
        };
        let popup = Popup::new(body)
            .title(tr(
                &format!(" Remove via {} ", src.label()),
                &format!(" {} ile kaldır ", src.label()),
            ))
            .border_style(Style::new().fg(Color::Red))
            .style(Style::new().bg(Color::Reset));
        f.render_widget(popup, area);
    }
}

fn truncate(s: &str, w: usize) -> String {
    // karakter sayısıyla kısalt: Türkçe metinde bayt sayısı hanesi şişirir
    match s.char_indices().nth(w) {
        Some((i, _)) => format!("{}…", &s[..i]),
        None => s.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn target(name: &str, blocked: Option<&str>, deps: &[&str]) -> sources::Removal {
        sources::Removal {
            name: name.into(),
            installed: if blocked.is_some() {
                None
            } else {
                Some("1.0-1".into())
            },
            dependents: deps.iter().map(|d| (*d).to_string()).collect(),
            note: None,
            blocked: blocked.map(Into::into),
            include: blocked.is_none(),
        }
    }

    fn session(plan: Vec<sources::Removal>) -> RemovalSession {
        RemovalSession {
            src: Source::Pacman,
            plan,
            cursor: 0,
            scroll: 0,
        }
    }

    #[test]
    fn kaldirma_secimi_engelleri() {
        let mut s = session(vec![
            target("sed", None, &["nano"]),
            target("systemd", Some("protected base/system package"), &[]),
            target("nope", Some("'nope' is not installed"), &[]),
        ]);
        assert_eq!((s.included(), s.blocked()), (1, 2));
        assert_eq!(s.targets(), ["sed"]);

        // engelli hedef seçilemez; imleç gerekçesini bildirir
        s.move_cursor(1);
        assert!(s.cursor_blocked());
        s.toggle_cursor();
        assert_eq!(s.targets(), ["sed"]);

        // tümünü seç engelleri atlar: hepsi seçiliyse çözülür, değilse seçilir
        s.toggle_all();
        assert_eq!(s.included(), 0);
        assert!(s.targets().is_empty());
        s.toggle_all();
        assert_eq!((s.included(), s.targets().len()), (1, 1));
        assert_eq!(s.targets(), ["sed"]); // engelli hedefler hiç listeye girmez

        // imleç sınırlarda kalır
        s.move_cursor(-5);
        assert_eq!(s.cursor, 0);
        s.move_cursor(isize::MAX);
        assert_eq!(s.cursor, 2);
    }

    #[test]
    fn kisaltma_sonu_verir() {
        assert_eq!(truncate("linux", 8), "linux");
        assert_eq!(truncate("pacman 6.1.0", 8), "pacman 6…");
        // çok baytlı karakter tek gösterim hanesi sayılır
        assert_eq!(truncate("güncellendi", 6), "güncel…");
    }

    /// Onay penceresi küçük uçbirimde de çöker mi: kaydırma, sığdırma, panik yok.
    fn render_at(
        width: u16,
        height: u16,
        cursor: usize,
        plan: Vec<sources::Removal>,
    ) -> Vec<String> {
        use ratatui::Terminal;
        use ratatui::backend::TestBackend;

        let mut app = App::new();
        app.removal = Some(RemovalSession {
            src: Source::Pacman,
            plan,
            cursor,
            scroll: 0,
        });
        let mut term = Terminal::new(TestBackend::new(width, height)).unwrap();
        let drawn = term.draw(|f| app.draw(f)).unwrap();
        let buf = drawn.buffer;
        (0..buf.area.height)
            .map(|y| {
                (0..buf.area.width)
                    .map(|x| buf[(x, y)].symbol())
                    .collect::<String>()
            })
            .collect()
    }

    /// Pencere gövdesi satırları (iç kenarlık satırları): ilki sayaç, sonu komut.
    fn body(rows: &[String]) -> Vec<String> {
        rows.iter()
            .filter(|r| r.starts_with('│') || r.contains("│ "))
            .cloned()
            .collect()
    }

    #[test]
    fn kaldirma_penceresi_kucuk_ekranda_cokmez() {
        // 12 hedef, her birinin ters bağımlısı + engelli bir tanesi: uzun liste
        let plan: Vec<sources::Removal> = (0..12)
            .map(|i| {
                if i == 3 {
                    target("systemd", Some("korunan taban/sistem paketi"), &[])
                } else {
                    target(&format!("paket{i}"), None, &["liba", "libb", "libc"])
                }
            })
            .collect();

        for cursor in [0, 11] {
            let rows = render_at(64, 14, cursor, plan.clone());
            let text = rows.join("\n");
            let body = body(&rows);
            // kaydırma imleci görünür satıra taşır
            assert!(
                text.contains(&format!("[x] paket{cursor}")),
                "imleç görünür olmalı:\n{text}"
            );
            // sayaç üstte, komut önizlemesi altta sabit: kaydırmadan etkilenmez
            assert!(body[0].contains("12"), "sayaç sabit kalmalı:\n{text}");
            assert!(
                body[body.len() - 1].contains("pacman -Rns paket0"),
                "komut önizlemesi sabit kalmalı:\n{text}"
            );
            for row in &rows {
                assert!(row.chars().count() <= 64, "satır taşmış: {row:?}");
            }
        }

        // aşırı küçük uçbirim: panik yok, komut yine son satırda
        let tiny_plan: Vec<sources::Removal> = (0..12)
            .map(|i| target(&format!("p{i}"), None, &[]))
            .collect();
        let rows = render_at(30, 7, 11, tiny_plan);
        let body = body(&rows);
        assert!(
            body.last().unwrap().contains("pacman"),
            "komut görünmeli: {body:?}"
        );
        for row in &rows {
            assert!(row.chars().count() <= 30, "satır taşmış: {row:?}");
        }
    }
}
