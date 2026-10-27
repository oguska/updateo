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
use ratatui::widgets::{Block, Paragraph, Wrap};
use std::process::{Command, ExitStatus};
use std::sync::mpsc::{self, Receiver, Sender};
use std::thread;
use std::time::{Duration, Instant};
use tui_big_text::{BigText, PixelSize};
use tui_input::{Input, backend::crossterm::EventHandler};
use tui_popup::{KnownSizeWrapper, Popup};
use tui_scrollbar::{GlyphSet, ScrollBar, ScrollLengths};

/// Sekme sayısı: üç kaynak + geçmiş (sonuncusu).
const TAB_COUNT: usize = Source::ALL.len() + 1;

pub struct CheckMsg {
    pub source: Source,
    pub result: CheckResult,
}

/// Arama thread'inden dönen sonuç; eski sorguların yanıtları `id` ile elenir.
struct SearchMsg {
    id: u64,
    result: Result<Vec<sources::SearchHit>, String>,
}

/// Açık paket arama penceresi.
struct SearchState {
    src: Source,
    input: Input,
    query: String,
    hits: Vec<sources::SearchHit>,
    cursor: usize,
    scroll: usize,
    loading: bool,
    started: Option<Instant>,
    error: Option<String>,
    id: u64,
}

impl SearchState {
    fn selected(&self) -> Option<&sources::SearchHit> {
        self.hits.get(self.cursor)
    }

    fn move_cursor(&mut self, delta: isize) {
        if self.hits.is_empty() {
            return;
        }
        self.cursor = if delta > 0 {
            (self.cursor + delta as usize).min(self.hits.len() - 1)
        } else {
            self.cursor.saturating_sub((-delta) as usize)
        };
    }
}

/// Sağdaki/üstteki paket ayrıntı paneli.
struct DetailsState {
    title: String,
    lines: Vec<String>,
    scroll: usize,
}

/// Onay bekleyen uygulama: kaynak başına seçili adlar.
struct ApplySession {
    plan: Vec<ApplyPlan>,
}

struct ApplyPlan {
    src: Source,
    names: Vec<String>,
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
    /// Giriş satırında gösterilecek reddetme gerekçesi (ör. `-` ile başlayan
    /// ad). Girilen her tuşla, giriş kapanınca ve kabul edilince temizlenir.
    input_error: Option<String>,
    /// Açık kaldırma onayı; None = onay penceresi yok.
    removal: Option<RemovalSession>,
    /// Açık uygulama onayı; None = onay penceresi yok.
    pending_apply: Option<ApplySession>,
    /// Açık paket arama penceresi.
    search: Option<SearchState>,
    /// Seçili paketin ayrıntı paneli.
    details: Option<DetailsState>,
    /// `/` canlı filtresi (güncelleme sekmeleri).
    filter: String,
    filter_input: Option<Input>,
    pub quit: bool,
    rx: Receiver<CheckMsg>,
    tx: Sender<CheckMsg>,
    search_rx: Receiver<SearchMsg>,
    search_tx: Sender<SearchMsg>,
    search_next_id: u64,
    last_check: Option<Instant>,
    check_started: Option<Instant>,
    /// Askıya alınmış TUI döndükten sonra ratatui'nin ön belleği terminalle
    /// uyuşmaz; ana döngü bu bayrakla ekranı/önbelleği sıfırlayıp tam çizer.
    force_redraw: bool,
}

impl App {
    pub fn new() -> Self {
        let (tx, rx) = mpsc::channel();
        let (search_tx, search_rx) = mpsc::channel();
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
            input_error: None,
            removal: None,
            pending_apply: None,
            search: None,
            details: None,
            filter: String::new(),
            filter_input: None,
            quit: false,
            rx,
            tx,
            search_rx,
            search_tx,
            search_next_id: 0,
            last_check: None,
            check_started: None,
            force_redraw: false,
        }
    }

    /// Üç kaynağı da paralel thread'lerde kontrol et; sonuçlar kanaldan gelir.
    pub fn start_refresh(&mut self) {
        self.status = tr("Checking for updates…", "Güncellemeler kontrol ediliyor…");
        self.status_err = false;
        self.spawn_checks();
    }

    /// Askıya alınmış TUI'yi çalıştır; dönüşte tam yeniden çizim iste.
    /// `runner` alternatif ekranı temizleyip geri açar, fakat ratatui'nin
    /// önceki-kare önbelleği eski içeriği hatırlar; önbellek sıfırlanmadan
    /// değişmeyen sekmeler/kenarlıklar yeniden çizilmez.
    fn run_suspended(&mut self, cmd: &mut Command) -> std::io::Result<ExitStatus> {
        let result = runner::run_suspended(cmd);
        self.force_redraw = true;
        result
    }

    /// Ana döngü: tam yeniden çizim gerekiyorsa bayrağı tüket.
    pub fn take_redraw(&mut self) -> bool {
        std::mem::take(&mut self.force_redraw)
    }

    /// Thread'leri başlat; durumu (mesajı) çağıranın yönetmesine bırak —
    /// uygulanan/senkron edilen işlemin mesajı kontrol bitene kadar görünür kalır.
    fn spawn_checks(&mut self) {
        self.checking = Source::ALL.len();
        self.check_started = Some(Instant::now());
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
            self.check_started = None;
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
    /// `/` filtresi ad, görünen ad ve sürüm alanlarında canlı uygulanır.
    fn tab_items(&self) -> Vec<usize> {
        if self.is_hist_tab() {
            return Vec::new();
        }
        let src = Source::ALL[self.tab];
        let needle = self.filter.to_lowercase();
        self.updates
            .iter()
            .enumerate()
            .filter(|(_, u)| {
                u.source == src
                    && (needle.is_empty()
                        || u.name.to_lowercase().contains(&needle)
                        || u.display.to_lowercase().contains(&needle)
                        || u.from.to_lowercase().contains(&needle)
                        || u.to.to_lowercase().contains(&needle))
            })
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

    /// Sekmedeki görünen tümünü seç / tümünün seçimini kaldır.
    fn toggle_all_tab(&mut self) {
        if self.is_hist_tab() {
            return;
        }
        let items = self.tab_items();
        if items.is_empty() {
            return;
        }
        let all_selected = items.iter().all(|&i| self.updates[i].selected);
        for i in items {
            self.updates[i].selected = !all_selected;
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

    /// Kontrol sürerken dönen animasyon karesi.
    fn spinner(&self) -> &'static str {
        const FRAMES: [&str; 10] = ["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"];
        let elapsed = self.check_started.map(|s| s.elapsed()).unwrap_or_default();
        FRAMES[(elapsed.as_millis() / 120) as usize % FRAMES.len()]
    }

    /// Tamamlanan kontrollere göre kalan süre tahmini.
    fn check_eta(&self) -> Option<u64> {
        let start = self.check_started?;
        let done = Source::ALL.len().saturating_sub(self.checking);
        if done == 0 || self.checking == 0 {
            return None;
        }
        let per = start.elapsed().as_secs_f64() / done as f64;
        Some((per * self.checking as f64).ceil() as u64)
    }

    fn open_filter(&mut self) {
        if self.is_hist_tab() {
            self.status = tr(
                "Filter is available on package tabs.",
                "Filtre paket sekmelerinde kullanılabilir.",
            );
            self.status_err = true;
            return;
        }
        self.filter_input = Some(Input::new(self.filter.clone()));
        self.status = tr(
            "Type to filter this tab — Enter keeps it, Esc clears.",
            "Bu sekmeyi filtrelemek için yaz — Enter korur, Esc temizler.",
        );
        self.status_err = false;
    }

    fn on_filter_key(&mut self, k: KeyEvent) {
        let Some(mut inp) = self.filter_input.take() else {
            return;
        };
        match k.code {
            KeyCode::Enter => {
                self.filter = inp.value().to_string();
                self.sel = 0;
                self.offset = 0;
                self.clamp();
                self.status = if self.filter.is_empty() {
                    tr("Filter cleared.", "Filtre temizlendi.")
                } else {
                    tr(
                        &format!("Filter: {}", self.filter),
                        &format!("Filtre: {}", self.filter),
                    )
                };
                self.status_err = false;
            }
            KeyCode::Esc => {
                self.filter.clear();
                self.sel = 0;
                self.offset = 0;
                self.status = tr("Filter cleared.", "Filtre temizlendi.");
                self.status_err = false;
            }
            _ => {
                inp.handle_event(&Event::Key(k));
                self.filter = inp.value().to_string();
                self.sel = 0;
                self.offset = 0;
                self.clamp();
                self.filter_input = Some(inp);
            }
        }
    }

    fn open_search(&mut self) {
        if self.is_hist_tab() {
            self.status = tr(
                "Search is available on package tabs.",
                "Arama paket sekmelerinde kullanılabilir.",
            );
            self.status_err = true;
            return;
        }
        self.details = None;
        self.search = Some(SearchState {
            src: Source::ALL[self.tab],
            input: Input::new(String::new()),
            query: String::new(),
            hits: Vec::new(),
            cursor: 0,
            scroll: 0,
            loading: false,
            started: None,
            error: None,
            id: 0,
        });
        self.status = tr(
            "Type a query; Enter searches, then Enter installs the selected result.",
            "Sorgu yaz; Enter arar, sonra Enter seçili sonucu kurar.",
        );
        self.status_err = false;
    }

    fn run_search(&mut self) {
        let tx = self.search_tx.clone();
        let id = self.search_next_id + 1;
        let Some(sess) = self.search.as_mut() else {
            return;
        };
        let query = sess.input.value().trim().to_string();
        if query.is_empty() {
            sess.error = Some(tr("empty search query", "boş arama sorgusu"));
            return;
        }
        if query.starts_with('-') {
            sess.error = Some(tr(
                "search query may not start with '-'",
                "arama sorgusu '-' ile başlayamaz",
            ));
            return;
        }
        let src = sess.src;
        sess.id = id;
        sess.query = query.clone();
        sess.loading = true;
        sess.started = Some(Instant::now());
        sess.error = None;
        sess.hits.clear();
        sess.cursor = 0;
        sess.scroll = 0;
        self.search_next_id = id;
        thread::spawn(move || {
            let result = sources::search(src, &query);
            let _ = tx.send(SearchMsg { id, result });
        });
    }

    fn drain_search(&mut self) {
        while let Ok(msg) = self.search_rx.try_recv() {
            let Some(sess) = self.search.as_mut() else {
                continue;
            };
            if sess.id != msg.id {
                continue;
            }
            sess.loading = false;
            sess.started = None;
            match msg.result {
                Ok(hits) => {
                    sess.hits = hits;
                    sess.cursor = 0;
                    sess.scroll = 0;
                    sess.error = None;
                }
                Err(e) => {
                    sess.hits.clear();
                    sess.error = Some(e);
                }
            }
        }
    }

    fn open_details(&mut self, src: Source, repo: &str, name: &str) {
        match sources::package_details(src, repo, name) {
            Ok(lines) if !lines.is_empty() => {
                self.details = Some(DetailsState {
                    title: if repo.is_empty() {
                        name.to_string()
                    } else {
                        format!("{repo}/{name}")
                    },
                    lines,
                    scroll: 0,
                });
                self.status = tr(
                    "Details open — j/k scrolls, d/Esc closes.",
                    "Ayrıntılar açık — j/k kaydırır, d/Esc kapatır.",
                );
                self.status_err = false;
            }
            Ok(_) => {
                self.status = tr(
                    &format!("No details for {name}."),
                    &format!("{name} için ayrıntı yok."),
                );
                self.status_err = true;
            }
            Err(e) => {
                self.status = tr(
                    &format!("Details for {name} failed: {e}"),
                    &format!("{name} ayrıntıları alınamadı: {e}"),
                );
                self.status_err = true;
            }
        }
    }

    fn details_current(&mut self) {
        if self.is_hist_tab() {
            return;
        }
        let items = self.tab_items();
        let Some(&gi) = items.get(self.sel) else {
            return;
        };
        let u = &self.updates[gi];
        let (src, name) = (u.source, u.name.clone());
        self.open_details(src, "", &name);
    }

    fn on_details_key(&mut self, k: KeyEvent) {
        let Some(details) = self.details.as_mut() else {
            return;
        };
        match k.code {
            KeyCode::Esc | KeyCode::Char('d') | KeyCode::Char('q') => {
                self.details = None;
                self.status = tr("Details closed.", "Ayrıntılar kapatıldı.");
                self.status_err = false;
            }
            KeyCode::Char('j') | KeyCode::Down => {
                details.scroll = details.scroll.saturating_add(1);
            }
            KeyCode::Char('k') | KeyCode::Up => {
                details.scroll = details.scroll.saturating_sub(1);
            }
            KeyCode::PageDown => details.scroll = details.scroll.saturating_add(10),
            KeyCode::PageUp => details.scroll = details.scroll.saturating_sub(10),
            KeyCode::Home => details.scroll = 0,
            KeyCode::End => details.scroll = details.lines.len().saturating_sub(1),
            _ => {}
        }
    }

    fn on_search_key(&mut self, k: KeyEvent) {
        let Some(mut sess) = self.search.take() else {
            return;
        };
        match k.code {
            KeyCode::Esc => {
                self.status = tr("Search closed.", "Arama kapatıldı.");
                self.status_err = false;
            }
            KeyCode::Enter => {
                let hit = sess.selected().cloned();
                let changed = sess.input.value().trim() != sess.query;
                if changed || hit.is_none() {
                    self.search = Some(sess);
                    self.run_search();
                } else if let Some(hit) = hit {
                    self.search = None;
                    self.install_via(hit.source, vec![hit.name]);
                }
            }
            KeyCode::Down => {
                sess.move_cursor(1);
                self.search = Some(sess);
            }
            KeyCode::Up => {
                sess.move_cursor(-1);
                self.search = Some(sess);
            }
            KeyCode::PageDown => {
                sess.move_cursor(10);
                self.search = Some(sess);
            }
            KeyCode::PageUp => {
                sess.move_cursor(-10);
                self.search = Some(sess);
            }
            KeyCode::Home => {
                sess.cursor = 0;
                self.search = Some(sess);
            }
            KeyCode::End => {
                sess.cursor = sess.hits.len().saturating_sub(1);
                self.search = Some(sess);
            }
            KeyCode::Char('d') if k.modifiers.contains(KeyModifiers::CONTROL) => {
                let hit = sess.selected().cloned();
                self.search = Some(sess);
                if let Some(hit) = hit {
                    self.open_details(hit.source, &hit.repo, &hit.name);
                }
            }
            KeyCode::Char('i') if k.modifiers.contains(KeyModifiers::CONTROL) => {
                let hit = sess.selected().cloned();
                if let Some(hit) = hit {
                    self.search = None;
                    self.install_via(hit.source, vec![hit.name]);
                } else {
                    self.search = Some(sess);
                }
            }
            _ => {
                sess.input.handle_event(&Event::Key(k));
                self.search = Some(sess);
            }
        }
    }

    fn request_apply(&mut self) {
        let mut plan = Vec::new();
        for src in Source::ALL {
            let names: Vec<String> = self
                .updates
                .iter()
                .filter(|u| u.source == src && u.selected)
                .map(|u| u.name.clone())
                .collect();
            if !names.is_empty() {
                plan.push(ApplyPlan { src, names });
            }
        }
        if plan.is_empty() {
            self.status = tr(
                "No package selected — press space to select, u to apply.",
                "Seçili paket yok — space ile seç, u ile uygula.",
            );
            self.status_err = false;
            return;
        }
        self.pending_apply = Some(ApplySession { plan });
        self.status = tr(
            "Review the apply plan — y applies, Esc cancels.",
            "Uygulama planını incele — y uygular, Esc vazgeçer.",
        );
        self.status_err = false;
    }

    fn confirm_apply(&mut self) {
        if let Some(session) = self.pending_apply.take() {
            self.run_apply(session.plan);
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
        match self.run_suspended(&mut cmd) {
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

    /// Onaylanan planı kaynak bazında uygula. TUI askıya alınır, çocuk süreç
    /// gerçek TTY'yi kullanır (sudo parolası, pacman/yay onayları dahil).
    fn run_apply(&mut self, plan: Vec<ApplyPlan>) {
        for ApplyPlan { src, names } in plan {
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
            match self.run_suspended(&mut cmd) {
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

    /// Paket kurma: TUI askıya alınır, kurucu gerçek TTY'yi kullanır. Onay
    /// soruları bayraklarla kapatılır; yalnız sudo parolası sorulabilir.
    /// Sonuç geçmişe yazılır, sonra yeniden kontrol.
    fn install(&mut self, names: Vec<String>) {
        if self.is_hist_tab() {
            return;
        }
        let src = Source::ALL[self.tab];
        self.install_via(src, names);
    }

    /// Belirtilen kaynakla kur: arama sonucu farklı kaynaktan gelebilir.
    fn install_via(&mut self, src: Source, names: Vec<String>) {
        // Arama çıktısı da aynı ad doğrulamasından geçsin.
        let names = match sources::split_names(&names.join(" ")) {
            Ok(names) => names,
            Err(e) => {
                self.status = e;
                self.status_err = true;
                return;
            }
        };
        let list = names.join(" ");
        let mut cmd = sources::install_command(src, &names);
        self.status = tr(
            &format!("Installing {list} via {}…", src.label()),
            &format!("{list}, {} ile kuruluyor…", src.label()),
        );
        self.status_err = false;
        let outcome = match self.run_suspended(&mut cmd) {
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
        let outcome = match self.run_suspended(&mut cmd) {
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

    fn on_apply_key(&mut self, k: KeyEvent) {
        match k.code {
            KeyCode::Esc | KeyCode::Char('q') => {
                self.pending_apply = None;
                self.status = tr("Apply cancelled.", "Uygulama iptal edildi.");
                self.status_err = false;
            }
            KeyCode::Enter | KeyCode::Char('y') | KeyCode::Char('Y') => {
                self.confirm_apply();
            }
            _ => {}
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
        if self.pending_apply.is_some() {
            self.on_apply_key(k);
            return Ok(());
        }
        // Ayrıntı paneli açıkken j/k kaydırır, d/Esc kapatır.
        if self.details.is_some() {
            self.on_details_key(k);
            return Ok(());
        }
        if self.search.is_some() {
            self.on_search_key(k);
            return Ok(());
        }
        if self.filter_input.is_some() {
            self.on_filter_key(k);
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
                            // Girdi satırı açıkken durum satırı da o satırdır:
                            // gerekçeyi orada, yazılan adın yanında göster.
                            self.input_error = Some(e);
                            self.pkg_input = Some(inp);
                            return Ok(());
                        }
                    };
                    if raw.is_empty() {
                        self.input_error = Some(tr(
                            "empty name — press Esc to cancel",
                            "isim boş — Esc ile vazgeç",
                        ));
                        self.status_err = false;
                        self.pkg_input = Some(inp);
                    } else {
                        self.input_error = None;
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
                    self.input_error = None;
                }
                _ => {
                    inp.handle_event(&Event::Key(k));
                    // Yeni bir tuş: önceki reddetme gerekçesi geçersizleşir.
                    self.input_error = None;
                    self.pkg_input = Some(inp);
                }
            }
            return Ok(());
        }
        match k.code {
            KeyCode::Char('q') => self.quit = true,
            KeyCode::Esc if !self.filter.is_empty() => {
                self.filter.clear();
                self.sel = 0;
                self.offset = 0;
                self.clamp();
                self.status = tr("Filter cleared.", "Filtre temizlendi.");
                self.status_err = false;
            }
            KeyCode::Esc => self.quit = true,
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
            KeyCode::Char('/') => self.open_filter(),
            KeyCode::Char('s') => self.open_search(),
            KeyCode::Char('d') if !self.is_hist_tab() => self.details_current(),
            KeyCode::Char('u') | KeyCode::Enter => self.request_apply(),
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
        self.drain_search();
        Ok(())
    }

    // ------------------------------- çizim -------------------------------

    /// Geçmiş sekmesi: uygulanan güncellemeler, en yeni üstte.
    fn draw_history(&mut self, f: &mut Frame, area: Rect) {
        if self.history.is_empty() {
            f.render_widget(
                Paragraph::new(tr(
                    "no updates applied yet — entries appear here after 'u'",
                    "henüz güncelleme uygulanmadı — 'u' sonrası kayıtlar burada",
                ))
                .style(Style::new().fg(Color::DarkGray)),
                area,
            );
            return;
        }
        let n = self.history.len();
        let inner_h = area.height as usize;
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
        f.render_widget(Paragraph::new(lines), area);
    }

    pub fn draw(&mut self, f: &mut Frame) {
        let area = f.area();
        let hero = area.width >= 64 && area.height >= 18;
        let [hero_area, tabs_area, hint_area, list_area, status_area] = Layout::vertical([
            Constraint::Length(if hero { 2 } else { 0 }),
            Constraint::Length(1),
            Constraint::Length(1),
            Constraint::Min(3),
            Constraint::Length(1),
        ])
        .areas(area);

        if hero {
            let title = BigText::builder()
                .pixel_size(PixelSize::QuarterHeight)
                .style(Style::new().fg(Color::Cyan).add_modifier(Modifier::BOLD))
                .lines(vec![Line::from("updateo")])
                .build();
            f.render_widget(title, hero_area);
        }

        // Sekme çubuğu: "pacman (seçili/toplam)" rozetleri; kısayollar alt satırda.
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
        f.render_widget(Paragraph::new(Line::from(spans)), tabs_area);
        // Kısayol ipucu sekme rozetlerinin altında ayrı satırda.
        f.render_widget(
            Paragraph::new(Line::from(Span::styled(
                tr(
                    " / filter · s search · d details · space select · a all · r refresh · R refresh db · i install · x/X remove · u apply · q quit",
                    " / filtre · s ara · d ayrıntı · space seçim · a tümünü · r yenile · R db tazele · i kur · x/X kaldır · u güncelle · q çık",
                ),
                Style::new().fg(Color::DarkGray),
            ))),
            hint_area,
        );

        // Liste: ayrıntı paneli açıksa sağdan yer ayır.
        let (list_area, detail_area) =
            if self.details.is_some() && self.search.is_none() && area.width >= 84 {
                let [list, detail] =
                    Layout::horizontal([Constraint::Percentage(58), Constraint::Percentage(42)])
                        .areas(list_area);
                (list, Some(detail))
            } else {
                (list_area, None)
            };
        let title = if self.filter.is_empty() || self.is_hist_tab() {
            tr(
                &format!(" updateo v{} · Arch updater ", crate::VERSION),
                &format!(" updateo v{} · Arch güncelleyici ", crate::VERSION),
            )
        } else {
            tr(
                &format!(" updateo · filter: {} ", self.filter),
                &format!(" updateo · filtre: {} ", self.filter),
            )
        };
        let block = Block::bordered().title(truncate(&title, list_area.width as usize));
        let content = list_area.inner(ratatui::layout::Margin::new(1, 1));
        f.render_widget(block, list_area);
        let [content_area, bar_area] =
            Layout::horizontal([Constraint::Fill(1), Constraint::Length(1)]).areas(content);
        let items = self.tab_items();
        if self.is_hist_tab() {
            self.draw_history(f, content_area);
            self.draw_scrollbar(
                f,
                bar_area,
                self.history.len(),
                content_area.height as usize,
            );
        } else if items.is_empty() {
            let msg = if self.checking > 0 {
                format!(
                    "{} {}",
                    self.spinner(),
                    tr("checking…", "kontrol ediliyor…")
                )
            } else if !self.filter.is_empty() {
                tr(
                    &format!(
                        "no packages match '{}' — Esc clears the filter",
                        self.filter
                    ),
                    &format!(
                        "'{}' ile eşleşen paket yok — Esc filtreyi temizler",
                        self.filter
                    ),
                )
                .to_string()
            } else {
                tr(
                    "no packages to update — press 'r' to refresh",
                    "güncellenecek paket yok — 'r' ile yenile",
                )
                .to_string()
            };
            f.render_widget(
                Paragraph::new(msg)
                    .style(Style::new().fg(Color::DarkGray))
                    .wrap(Wrap { trim: false }),
                content_area,
            );
        } else {
            let inner_h = content_area.height as usize;
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
                .min((content_area.width as usize).saturating_sub(40))
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
            f.render_widget(Paragraph::new(lines), content_area);
            self.draw_scrollbar(f, bar_area, items.len(), inner_h);
        }

        if let Some(detail_area) = detail_area {
            self.draw_details_panel(f, detail_area);
        }

        // Durum satırı — filtre girdisi aktifse o satır giriş satırına dönüşür
        if let Some(inp) = &self.filter_input {
            let label = tr(" filter ", " filtre ");
            let hint = tr("Enter keep · Esc clear", "Enter koru · Esc temizle");
            let (_, avail, hint) = input_layout(status_area.width, &label, "", &hint);
            let chars: Vec<char> = inp.value().chars().collect();
            let cur = inp.cursor();
            let mut spans = vec![Span::styled(
                label.clone(),
                Style::new()
                    .fg(Color::Black)
                    .bg(Color::Cyan)
                    .add_modifier(Modifier::BOLD),
            )];
            let start = inp.visual_scroll(avail);
            let mut used = 0usize;
            for (i, c) in chars.iter().enumerate().skip(start).take(avail) {
                let st = if i == cur {
                    Style::new().bg(Color::White).fg(Color::Black)
                } else {
                    Style::new().fg(Color::White)
                };
                spans.push(Span::styled(c.to_string(), st));
                used += 1;
            }
            if cur >= start && cur < start + avail && cur >= chars.len() {
                spans.push(Span::styled(
                    " ",
                    Style::new().bg(Color::White).fg(Color::Black),
                ));
                used += 1;
            }
            if avail > used {
                spans.push(Span::raw(" ".repeat(avail - used)));
            }
            spans.push(Span::styled(
                format!("   {hint}"),
                Style::new().fg(Color::DarkGray),
            ));
            f.render_widget(Paragraph::new(Line::from(spans)), status_area);
            return;
        }

        // Kurma girdisi aktifse o satır giriş satırına dönüşür
        if let Some(inp) = &self.pkg_input {
            let src = Source::ALL[self.tab];
            let removing = self.input_mode == InputMode::Remove;
            let chars: Vec<char> = inp.value().chars().collect();
            let cur = inp.cursor();
            let label = if removing {
                tr(" remove ", " kaldırma ")
            } else {
                tr(" install ", " kurma ")
            };
            let label_color = if removing { Color::Red } else { Color::Cyan };
            let mut spans = vec![Span::styled(
                label.clone(),
                Style::new()
                    .fg(Color::Black)
                    .bg(label_color)
                    .add_modifier(Modifier::BOLD),
            )];
            // Reddedilen bir ad varsa gerekçe yazılanın hemen yanında durur:
            // durum satırı girdi açıkken yok, bu yüzden mesaj burada gösterilir.
            if let Some(err) = &self.input_error {
                let (avail, why) = input_error_layout(status_area.width, &label, err);
                let start = inp.visual_scroll(avail);
                spans.push(Span::raw(" "));
                let mut used = 0usize;
                for (i, c) in chars.iter().enumerate().skip(start).take(avail) {
                    let st = if i == cur {
                        Style::new().bg(Color::White).fg(Color::Black)
                    } else {
                        Style::new().fg(Color::White)
                    };
                    spans.push(Span::styled(c.to_string(), st));
                    used += 1;
                }
                if avail > used {
                    spans.push(Span::raw(" ".repeat(avail - used)));
                }
                spans.push(Span::styled(
                    format!("  {why}"),
                    Style::new().fg(Color::Red).add_modifier(Modifier::BOLD),
                ));
                f.render_widget(Paragraph::new(Line::from(spans)), status_area);
                return;
            }
            let preview = if removing {
                sources::remove_preview(src)
            } else {
                sources::install_preview(src)
            };
            let hint = if removing {
                tr("Enter review · Esc cancel", "Enter incele · Esc vazgeç")
            } else {
                tr("Enter install · Esc cancel", "Enter kur · Esc vazgeç")
            };
            // Hane paylaşımı: yazılan paket adı öncelikli. Uzun bir sarmalayıcı
            // (ör. tam yol `UPDATEO_SUDO`) ya da dar uçbirim girişi ezmesin.
            let (preview, avail, hint) = input_layout(status_area.width, &label, &preview, &hint);
            let prefix = format!(" {preview} ");
            let start = inp.visual_scroll(avail);
            let mut used = 0usize;
            spans.push(Span::styled(
                prefix,
                Style::new().fg(label_color).add_modifier(Modifier::BOLD),
            ));
            for (i, c) in chars.iter().enumerate().skip(start).take(avail) {
                let st = if i == cur {
                    Style::new().bg(Color::White).fg(Color::Black)
                } else {
                    Style::new().fg(Color::White)
                };
                spans.push(Span::styled(c.to_string(), st));
                used += 1;
            }
            if cur >= start && cur < start + avail && cur >= chars.len() {
                spans.push(Span::styled(
                    " ",
                    Style::new().bg(Color::White).fg(Color::Black),
                ));
                used += 1;
            }
            // Girdi hanesi tam `avail` haneye doldurulur: yazarken sağdaki
            // ipucu sola kaymasın, aynı hizada dursun.
            if avail > used {
                spans.push(Span::raw(" ".repeat(avail - used)));
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
        if self.checking > 0 {
            let total = Source::ALL.len();
            let done = total.saturating_sub(self.checking);
            status = format!("{} {status}  ·  {done}/{total}", self.spinner());
            if let Some(eta) = self.check_eta() {
                status.push_str(&tr(
                    &format!("  ·  ~{eta}s left"),
                    &format!("  ·  ~{eta}sn kaldı"),
                ));
            }
        } else if let Some(t) = self.last_check {
            status.push_str(&tr(
                &format!("  ·  last check {}s ago", t.elapsed().as_secs()),
                &format!("  ·  son kontrol {}sn önce", t.elapsed().as_secs()),
            ));
        }
        if !self.filter.is_empty() && !self.is_hist_tab() {
            status.push_str(&tr(
                &format!("  ·  filter '{}' ({} match)", self.filter, self.tab_len()),
                &format!("  ·  filtre '{}' ({} eşleşme)", self.filter, self.tab_len()),
            ));
        }
        f.render_widget(
            Paragraph::new(Line::from(Span::styled(status, style))),
            status_area,
        );

        if self.search.is_some() {
            self.draw_search(f, area);
        }
        // Ayrıntı paneli dar uçbirimde ya da arama üstünde popup olur.
        if self.details.is_some() && (detail_area.is_none() || self.search.is_some()) {
            self.draw_details_popup(f, area);
        }
        if self.pending_apply.is_some() {
            self.draw_apply(f, area);
        }
        // Kaldırma onayı en üstte: hedefler, uyarılar ve çalıştırılacak komut.
        if self.removal.is_some() {
            self.draw_removal(f, area);
        }
    }

    /// Dikey kaydırma çubuğu: içerik pencereye sığmıyorsa sağ kenarda.
    fn draw_scrollbar(&self, f: &mut Frame, area: Rect, content: usize, viewport: usize) {
        if area.width == 0 || area.height == 0 || content <= viewport {
            return;
        }
        let lengths = ScrollLengths {
            content_len: content,
            viewport_len: viewport.max(1),
        };
        let bar = ScrollBar::vertical(lengths)
            .offset(self.offset)
            .glyph_set(GlyphSet::unicode())
            .track_style(Style::new().fg(Color::DarkGray))
            .thumb_style(Style::new().fg(Color::Cyan));
        f.render_widget(&bar, area);
    }

    /// Sağdaki ayrıntı paneli (geniş uçbirimde).
    fn draw_details_panel(&self, f: &mut Frame, area: Rect) {
        let Some(details) = &self.details else {
            return;
        };
        let title = tr(
            &format!(" details · {} ", details.title),
            &format!(" ayrıntı · {} ", details.title),
        );
        let block = Block::bordered().title(truncate(&title, area.width as usize));
        let inner = block.inner(area);
        f.render_widget(block, area);
        let lines: Vec<Line> = details
            .lines
            .iter()
            .map(|line| Line::from(line.clone()))
            .collect();
        f.render_widget(
            Paragraph::new(lines)
                .wrap(Wrap { trim: false })
                .scroll((details.scroll as u16, 0)),
            inner,
        );
    }

    /// Dar uçbirimde / arama üstünde ayrıntı popup'ı.
    fn draw_details_popup(&self, f: &mut Frame, area: Rect) {
        let Some(details) = &self.details else {
            return;
        };
        let body_w = (area.width as usize)
            .saturating_sub(4)
            .clamp(8, 88)
            .min((area.width as usize).saturating_sub(2).max(1));
        let max_h = (area.height as usize).saturating_sub(2).clamp(3, 22);
        let lines: Vec<Line> = details
            .lines
            .iter()
            .map(|line| Line::from(line.clone()))
            .collect();
        let paragraph = Paragraph::new(lines)
            .wrap(Wrap { trim: false })
            .scroll((details.scroll as u16, 0));
        // Sarmalanan satırları yaklaşık say: kısa ayrıntıda boş popup büyümesin.
        let cols = body_w.max(1);
        let wrapped = details
            .lines
            .iter()
            .map(|line| line.chars().count().div_ceil(cols))
            .sum::<usize>();
        let body_h = wrapped.clamp(1, max_h);
        let body = KnownSizeWrapper {
            inner: paragraph,
            width: body_w,
            height: body_h,
        };
        let popup = Popup::new(body)
            .title(tr(
                &format!(" details · {} ", details.title),
                &format!(" ayrıntı · {} ", details.title),
            ))
            .border_style(Style::new().fg(Color::Cyan))
            .style(Style::new().bg(Color::Reset));
        f.render_widget(popup, area);
    }

    /// Paket arama penceresi: sorgu, sonuçlar ve kaydırma.
    fn draw_search(&mut self, f: &mut Frame, area: Rect) {
        let Some(sess) = self.search.as_mut() else {
            return;
        };
        let body_w = (area.width as usize)
            .saturating_sub(4)
            .clamp(20, 96)
            .min((area.width as usize).saturating_sub(2).max(1));
        let max_h = (area.height as usize).saturating_sub(2).clamp(3, 24);
        let query = truncate_tail(sess.input.value(), body_w.saturating_sub(3));
        let mut lines: Vec<Line> = vec![Line::from(vec![
            Span::styled(
                "> ",
                Style::new().fg(Color::Cyan).add_modifier(Modifier::BOLD),
            ),
            Span::styled(query, Style::new().fg(Color::White)),
            Span::styled("▏", Style::new().fg(Color::Cyan)),
        ])];
        if sess.loading {
            let frame = sess
                .started
                .map(|s| {
                    const FRAMES: [&str; 10] = ["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"];
                    FRAMES[(s.elapsed().as_millis() / 120) as usize % FRAMES.len()]
                })
                .unwrap_or("⠋");
            lines.push(Line::from(Span::styled(
                format!(
                    "{frame} searching {} for '{}'…",
                    sess.src.label(),
                    sess.query
                ),
                Style::new().fg(Color::Yellow),
            )));
        } else if let Some(err) = &sess.error {
            lines.push(Line::from(Span::styled(
                truncate(err, body_w),
                Style::new().fg(Color::Red),
            )));
        } else if sess.hits.is_empty() {
            lines.push(Line::from(Span::styled(
                tr(
                    "no results — edit the query and press Enter",
                    "sonuç yok — sorguyu değiştirip Enter'a bas",
                ),
                Style::new().fg(Color::DarkGray),
            )));
        }
        // Sonuç penceresi: imleci görünür tut, liste sonunu aşma.
        let window = max_h.saturating_sub(lines.len() + 1).max(1);
        if sess.cursor < sess.scroll {
            sess.scroll = sess.cursor;
        }
        if sess.cursor >= sess.scroll + window {
            sess.scroll = sess.cursor + 1 - window;
        }
        sess.scroll = sess.scroll.min(sess.hits.len().saturating_sub(window));
        for (idx, hit) in sess.hits.iter().enumerate().skip(sess.scroll).take(window) {
            let cursor = if idx == sess.cursor { ">" } else { " " };
            let line = format!(
                "{cursor} {}  {}  {}",
                hit.display(),
                hit.version,
                hit.description
            );
            let mut style = Style::new().fg(Color::White);
            if idx == sess.cursor {
                style = style.bg(Color::DarkGray).add_modifier(Modifier::BOLD);
            }
            lines.push(Line::from(Span::styled(truncate(&line, body_w), style)));
        }
        lines.push(Line::from(Span::styled(
            tr(
                "↑/↓ choose · Enter install · Ctrl+D details · Esc close",
                "↑/↓ seç · Enter kur · Ctrl+D ayrıntı · Esc kapat",
            ),
            Style::new().fg(Color::DarkGray),
        )));
        let body_h = lines.len().clamp(1, max_h);
        let body = KnownSizeWrapper {
            inner: Paragraph::new(lines),
            width: body_w,
            height: body_h,
        };
        let popup = Popup::new(body)
            .title(tr(
                &format!(" Search · {} ", sess.src.label()),
                &format!(" Ara · {} ", sess.src.label()),
            ))
            .border_style(Style::new().fg(Color::Cyan))
            .style(Style::new().bg(Color::Reset));
        f.render_widget(popup, area);
    }

    /// Uygulama onay penceresi: kaynak başına komut ve hedefler.
    fn draw_apply(&mut self, f: &mut Frame, area: Rect) {
        let Some(session) = &self.pending_apply else {
            return;
        };
        let body_w = (area.width as usize)
            .saturating_sub(4)
            .clamp(20, 96)
            .min((area.width as usize).saturating_sub(2).max(1));
        let total: usize = session.plan.iter().map(|p| p.names.len()).sum();
        let mut lines = vec![Line::from(Span::styled(
            tr(
                &format!("{} package(s) from {} source(s)", total, session.plan.len()),
                &format!("{} kaynaktan {} paket", session.plan.len(), total),
            ),
            Style::new().fg(Color::Yellow).add_modifier(Modifier::BOLD),
        ))];
        for plan in &session.plan {
            let label = format!("{} ", plan.src.label());
            let room = body_w.saturating_sub(label.chars().count());
            let command = command_preview(&sources::apply_preview(plan.src), &plan.names, room);
            lines.push(Line::from(vec![
                Span::styled(label, Style::new().fg(Color::DarkGray)),
                Span::styled(command, Style::new().fg(Color::Cyan)),
            ]));
        }
        lines.push(Line::from(Span::styled(
            tr("y/Enter apply · Esc cancel", "y/Enter uygula · Esc vazgeç"),
            Style::new().fg(Color::DarkGray),
        )));
        let max_h = (area.height as usize).saturating_sub(2).clamp(3, 24);
        let body_h = lines.len().clamp(1, max_h);
        let body = KnownSizeWrapper {
            inner: Paragraph::new(lines),
            width: body_w,
            height: body_h,
        };
        let popup = Popup::new(body)
            .title(tr(" Apply updates ", " Güncellemeleri uygula "))
            .border_style(Style::new().fg(Color::Green))
            .style(Style::new().bg(Color::Reset));
        f.render_widget(popup, area);
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
            // Komut satırı pencereye sığmalı; uzun listelerde sonu kısaltılır.
            let label = format!("{} ", tr("command:", "komut:"));
            let room = body_w.saturating_sub(label.chars().count() + 1);
            let command = command_preview(&sources::remove_preview(src), &targets, room);
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

/// Karakter sayısıyla kısalt: Türkçe metinde bayt sayısı hanesi şişirir.
fn truncate(s: &str, w: usize) -> String {
    // karakter sayısıyla kısalt: Türkçe metinde bayt sayısı hanesi şişirir
    match s.char_indices().nth(w) {
        Some((i, _)) => format!("{}…", &s[..i]),
        None => s.to_string(),
    }
}

/// Kuyruğu korunarak kısaltma: komut önizlemesinde kritik kısım (bayraklar ve
/// paket adları) sonda durur; sarmalayıcının tam yolu feda edilebilir.
fn truncate_tail(s: &str, w: usize) -> String {
    let n = s.chars().count();
    if n <= w {
        return s.to_string();
    }
    match w {
        0 => String::new(),
        1 => "…".into(),
        keep => {
            let at = s
                .char_indices()
                .nth(n - (keep - 1))
                .map(|(i, _)| i)
                .unwrap_or(s.len());
            format!("…{}", &s[at..])
        }
    }
}

/// Önizlemedeki yetki sarmalayıcısı tam bir yolysa dizin kısmı düşürülür:
/// `…/fake_sudo pacman -Rns` gibi kalır; bayraklar ve hedefler korunur.
fn compact_wrapper(preview: &str) -> String {
    let mut words = preview.splitn(2, ' ');
    let head = words.next().unwrap_or_default();
    let rest = words.next().unwrap_or_default();
    let head = match head.rsplit('/').next() {
        Some(base) if head.contains('/') && !base.is_empty() => format!("…/{base}"),
        _ => head.to_string(),
    };
    if rest.is_empty() {
        head
    } else {
        format!("{head} {rest}")
    }
}

/// Komut önizlemesini `room` hanesine sığdır. Önce ekseni dene; sığmazsa
/// sarmalayıcının dizin yolunu düşür, sonra hedefleri kuyruğundan kısalt
/// (`… +N`). Böylece çalışacak komutun kendisi — sarmalayıcı ve bayraklar —
/// sonuna kadar okunur; en kötü durumda iki uçtan kısaltmaya düşülür.
fn command_preview(preview: &str, targets: &[String], room: usize) -> String {
    if targets.is_empty() {
        return truncate(&tr("(no target selected)", "(seçili hedef yok)"), room);
    }
    let full = format!("{} {}", preview, targets.join(" "));
    if full.chars().count() <= room {
        return full;
    }
    let preview = compact_wrapper(preview);
    let mut n = targets.len();
    loop {
        let chosen = targets[..n].join(" ");
        let dropped = targets.len() - n;
        let line = match (dropped, n) {
            (0, _) => format!("{preview} {chosen}"),
            (m, 0) => format!("{preview} … +{m}"),
            (m, _) => format!("{preview} {chosen} … +{m}"),
        };
        if line.chars().count() <= room {
            return line;
        }
        match n.checked_sub(1) {
            Some(next) => n = next,
            None => break,
        }
    }
    // Hedef listesi bile sığmıyor: hiç olmazsa çalışacak komut görünsün.
    truncate_tail(&preview, room)
}

/// Giriş satırının hane paylaşımı: `(önizleme, girdi hanesi, ipucu)`.
/// Öncelik: sabit etiket → yazılan ad (`MIN_INPUT` hane) → ipucu → önizleme.
/// Yer daralınca önce önizleme kuyruğundan kısalır, sonra ipucu kısalmış
/// biçimine düşer; uzun bir `UPDATEO_SUDO` yolu girişi ezemez.
fn input_layout(width: u16, label: &str, preview: &str, hint: &str) -> (String, usize, String) {
    const MIN_INPUT: usize = 12;
    const SHORT_HINT: &str = "Enter/Esc";
    // etiket + " önizleme " + girdi ile ipucu arasındaki ayraç boşlukları
    let fixed = label.chars().count() + 5;
    let room = (width as usize).saturating_sub(fixed);
    let full = preview.chars().count() + hint.chars().count();
    if room >= full + MIN_INPUT {
        return (preview.to_string(), room - full, hint.to_string());
    }
    let hint_txt = if room >= MIN_INPUT + SHORT_HINT.chars().count() + 2 {
        SHORT_HINT.to_string()
    } else {
        String::new()
    };
    let avail = room
        .saturating_sub(hint_txt.chars().count() + 2)
        .min(MIN_INPUT)
        .max(1.min(room));
    let preview_w = room.saturating_sub(avail + hint_txt.chars().count() + 2);
    // Onay penceresiyle aynı mantık: önce sarmalayıcının dizin yolu düşer,
    // sonra kuyruktan kısalır — kalan parça her hâlde çalışacak komuttur.
    let compact = compact_wrapper(preview);
    let shown = if compact.chars().count() <= preview_w {
        compact
    } else {
        truncate_tail(&compact, preview_w)
    };
    (shown, avail, hint_txt)
}

/// Reddetme gerekçeli giriş satırının hane paylaşımı: `(girdi hanesi, gerekçe)`.
/// Öncelik yazılan ad (`MIN_INPUT` hane); gerekçe kalan yere sığmazsa
/// kuyruğundan kısalır, ama gerekçenin kendisi de en az 8 hane alır.
fn input_error_layout(width: u16, label: &str, err: &str) -> (usize, String) {
    const MIN_INPUT: usize = 12;
    const MIN_WHY: usize = 8;
    let fixed = label.chars().count() + 3; // etiket + ayraç boşlukları
    let room = (width as usize).saturating_sub(fixed);
    let why_full = err.chars().count();
    if room >= MIN_INPUT + why_full + 2 {
        return (room.saturating_sub(why_full + 2), err.to_string());
    }
    let why_w = room.saturating_sub(MIN_INPUT + 2).max(MIN_WHY.min(room));
    (
        room.saturating_sub(why_w + 2),
        truncate(err, why_w.max(1.min(room))),
    )
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
    fn askidan_sonra_tam_cizim_istenir() {
        let mut app = App::new();
        assert!(!app.take_redraw());
        app.force_redraw = true;
        assert!(app.take_redraw());
        assert!(!app.take_redraw());
    }

    #[test]
    fn kisaltma_sonu_verir() {
        assert_eq!(truncate("linux", 8), "linux");
        assert_eq!(truncate("pacman 6.1.0", 8), "pacman 6…");
        // çok baytlı karakter tek gösterim hanesi sayılır
        assert_eq!(truncate("güncellendi", 6), "güncel…");
    }

    #[test]
    fn kuyruk_kisaltmasi_bayragi_korur() {
        assert_eq!(truncate_tail("linux", 8), "linux");
        assert_eq!(truncate_tail("sudo pacman -Rns", 8), "…an -Rns");
        assert_eq!(truncate_tail("sudo pacman -Rns", 1), "…");
        assert_eq!(truncate_tail("sudo pacman -Rns", 0), "");
        // çok baytlı karakterler haneyi şişirmez
        assert_eq!(truncate_tail("güncellendi", 6).chars().count(), 6);
    }
    #[test]
    fn girdi_hanesi_onizlemeyi_ezmez() {
        let label = tr(" remove ", " kaldırma ");
        let hint = tr("Enter review · Esc cancel", "Enter incele · Esc vazgeç");
        let long = "/long/path/to/updateo/bin/fake-root-wrapper pacman -Rns";
        // geniş uçbirim: her şey eksik, girdi kalan yeri alır
        let (preview, avail, hint_txt) = input_layout(84, &label, "sudo pacman -Rns", &hint);
        assert_eq!(preview, "sudo pacman -Rns");
        assert_eq!(hint_txt, hint);
        assert!(avail >= 20, "geniş hane: {avail}");
        // uzun sarmalayıcı yolu (ör. tam yollu UPDATEO_SUDO) girişi ezmez:
        // önizleme kuyruğundan kısalır, girdi hanesi tavan yapar
        let (preview, avail, hint_txt) = input_layout(84, &label, long, &hint);
        assert!(
            preview.ends_with("pacman -Rns") && preview.chars().count() < long.chars().count(),
            "kuyruk korunmalı: {preview}"
        );
        assert_eq!(avail, 12, "girdi hanesi korunmalı: {avail}");
        assert_eq!(hint_txt, "Enter/Esc");
        // orta/dar uçbirim: satır taşmaz, girdi hanesi hiç yok olmaz
        for width in [34u16, 48, 60, 84] {
            let (preview, avail, hint_txt) = input_layout(width, &label, long, &hint);
            let used = label.chars().count()
                + preview.chars().count()
                + avail
                + hint_txt.chars().count()
                + 5;
            assert!(used <= width as usize, "{width} hanede taşma: {used}");
            assert!(avail >= 1, "{width} hanede girdi görünmeli");
        }
        // etiketin kendisinin sığmadığı uçbirim: panik yok, çökme yok
        for width in [4u16, 10, 18] {
            let (_, _, _) = input_layout(width, &label, long, &hint);
        }
    }

    #[test]
    fn reddedilen_ad_gerekcesi_girdiyi_ezmez() {
        let label = tr(" remove ", " kaldırma ");
        let err = "invalid name '-Rns': package names may not start with '-'";
        // geniş uçbirim: gerekçe eksik, girdi hanesi kalan yeri alır
        let (avail, why) = input_error_layout(100, &label, err);
        assert_eq!(why, err);
        assert!(avail >= 12, "girdi hanesi: {avail}");
        // dar uçbirim: satır taşmaz, iki parça da görünür kalır
        for width in [24u16, 34, 48, 60, 84] {
            let (avail, why) = input_error_layout(width, &label, err);
            let used = label.chars().count() + avail + why.chars().count() + 3;
            assert!(used <= width as usize, "{width} hanede taşma: {used}");
            assert!(avail >= 1, "{width} hanede yazılan ad görünmeli");
            assert!(why.chars().count() >= 8, "{width} hanede gerekçe: {why}");
        }
        // etiketin sığmadığı uçbirim: panik yok
        for width in [2u16, 6, 12] {
            input_error_layout(width, &label, err);
        }
    }

    #[test]
    fn komut_onizlemesi_pencereye_sigar() {
        let long = "/long/path/to/updateo/bin/fake-root-wrapper pacman -Rns";
        let targets: Vec<String> = (0..12).map(|i| format!("paket{i}")).collect();
        // geniş yer: ekseni komut, kısaltma yok
        let wide = command_preview(long, &targets, 200);
        assert_eq!(wide, format!("{long} {}", targets.join(" ")));
        // dar yer: sarmalayıcının yolu düşer, bayraklar ve hedefler kalır
        let tight = command_preview(long, &targets, 60);
        assert!(tight.chars().count() <= 60, "taşma: {tight}");
        assert!(
            tight.contains("fake-root-wrapper pacman -Rns"),
            "komut okunmalı: {tight}"
        );
        assert!(tight.contains('…'), "kısaltma işareti: {tight}");
        // aşırı dar: yine de sığar, panik yok; son kalan parça komutun kendisi
        for room in [0usize, 1, 8, 17, 24, 40] {
            let line = command_preview(long, &targets, room);
            assert!(line.chars().count() <= room, "{room} haneye sığmadı: {line}");
        }
        assert!(
            command_preview(long, &targets, 17).contains("-Rns"),
            "bayraklar korunmalı"
        );
        // seçili hedef yoksa boşluk değil, açıklama görünür
        let none = command_preview(long, &[], 40);
        assert!(
            none.contains("target") || none.contains("hedef"),
            "açıklama görünmeli: {none}"
        );
    }

    /// Alt giriş satırı: yazılan paket adı uzun önizlemeden önce gelir ve
    /// çizim satıra sığar (karakter kaybı görünen hata değil, yerleşim).
    fn input_row(width: u16, value: &str, cursor: usize) -> String {
        use ratatui::Terminal;
        use ratatui::backend::TestBackend;

        let mut app = App::new();
        app.input_mode = InputMode::Remove;
        app.pkg_input = Some(Input::new(value.into()).with_cursor(cursor));
        let mut term = Terminal::new(TestBackend::new(width, 12)).unwrap();
        let drawn = term.draw(|f| app.draw(f)).unwrap();
        let buf = drawn.buffer;
        let y = buf.area.height - 1;
        (0..buf.area.width)
            .map(|x| buf[(x, y)].symbol())
            .collect::<String>()
            .trim_end()
            .to_string()
    }

    #[test]
    fn girdi_satiri_yazilan_adi_gosterir() {
        // dar uçbirimde bile yazılan ad tam görünür; uzun önizleme kısalır
        let row = input_row(48, "nano sed", 8);
        assert!(row.contains("nano sed"), "yazılan ad kaybolmuş: {row:?}");
        assert!(row.contains("kaldırma") || row.contains("remove"), "etiket: {row:?}");
        assert!(row.chars().count() <= 48, "satır taşmış: {row:?}");
        // uzun girdi kaydırılır: imleçteki son kısım görünür
        let long = input_row(48, "nano sed grep diffutils", 23);
        assert!(long.contains("diffutils"), "imleç görünmeli: {long:?}");
        assert!(long.chars().count() <= 48, "satır taşmış: {long:?}");
        // aşırı dar uçbirim: panik yok, etiket yine satırın başında
        let tiny = input_row(18, "nano", 4);
        assert!(tiny.chars().count() <= 18, "satır taşmış: {tiny:?}");
        assert!(tiny.starts_with(' ') || tiny.contains("nano"), "girdi: {tiny:?}");
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

    /// Pencere gövdesi satırları: popup sol kenarı (x=2) olan iç satırlar.
    fn body(rows: &[String]) -> Vec<String> {
        rows.iter()
            .filter(|r| r.chars().nth(2) == Some('│'))
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
            body.last().unwrap().contains("pacman -Rns"),
            "komut görünmeli: {body:?}"
        );
        for row in &rows {
            assert!(row.chars().count() <= 30, "satır taşmış: {row:?}");
        }
    }

    fn update(src: Source, name: &str, selected: bool) -> Update {
        Update {
            source: src,
            name: name.into(),
            display: name.into(),
            from: "1.0-1".into(),
            to: "1.1-1".into(),
            runtime: false,
            selected,
        }
    }

    #[test]
    fn filtre_ve_toplu_secim_gorunene_uyar() {
        let mut app = App::new();
        app.updates = vec![
            update(Source::Pacman, "sed", true),
            update(Source::Pacman, "nano", false),
            update(Source::Aur, "tribler", false),
        ];
        app.filter = "se".into();
        let items = app.tab_items();
        assert_eq!(items.len(), 1, "filtre yalnız sed'i bırakmalı");
        assert_eq!(app.updates[items[0]].name, "sed");
        // görünen tek satır seçiliyse a tümünü kaldırır, gizli nano'ya dokunmaz
        app.toggle_all_tab();
        assert!(!app.updates[0].selected);
        assert!(!app.updates[1].selected);
        // filtre temizlenince iki satır görünür ve a ikisini seçer
        app.filter.clear();
        app.toggle_all_tab();
        assert!(app.updates[0].selected);
        assert!(app.updates[1].selected);
    }

    #[test]
    fn uygulama_plani_kaynaklara_bolunur() {
        let mut app = App::new();
        app.updates = vec![
            update(Source::Pacman, "sed", true),
            update(Source::Pacman, "nano", true),
            update(Source::Aur, "tribler", true),
            update(Source::Flatpak, "org.example.App", false),
        ];
        app.request_apply();
        let session = app.pending_apply.as_ref().expect("plan açılmalı");
        assert_eq!(session.plan.len(), 2);
        assert_eq!(session.plan[0].src, Source::Pacman);
        assert_eq!(session.plan[0].names, ["sed", "nano"]);
        assert_eq!(session.plan[1].src, Source::Aur);
        assert_eq!(session.plan[1].names, ["tribler"]);
    }

    #[test]
    fn arama_gezinme_tuslari_pencereyi_kapatmaz() {
        let hits = ["sed", "nano", "tribler"]
            .into_iter()
            .map(|name| sources::SearchHit {
                source: Source::Pacman,
                repo: "extra".into(),
                name: name.into(),
                version: "1.0-1".into(),
                description: String::new(),
                installed: false,
            })
            .collect();
        let mut app = App::new();
        app.search = Some(SearchState {
            src: Source::Pacman,
            input: Input::new("nano".into()),
            query: "nano".into(),
            hits,
            cursor: 0,
            scroll: 0,
            loading: false,
            started: None,
            error: None,
            id: 1,
        });
        let steps = [
            (KeyCode::Down, 1),
            (KeyCode::Down, 2),
            (KeyCode::Up, 1),
            (KeyCode::Home, 0),
            (KeyCode::End, 2),
        ];
        for (code, expected) in steps {
            app.on_search_key(KeyEvent::new(code, KeyModifiers::NONE));
            let sess = app.search.as_ref().expect("arama penceresi kapanmamalı");
            assert_eq!(sess.cursor, expected, "{code:?} sonrası imleç");
        }
    }
}
