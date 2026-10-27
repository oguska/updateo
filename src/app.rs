//! Uygulama durumu, tuş atamaları ve çizim.

use crate::history::{self, Entry};
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

/// Sekme sayısı: üç kaynak + geçmiş (sonuncusu).
const TAB_COUNT: usize = Source::ALL.len() + 1;

pub struct CheckMsg {
    pub source: Source,
    pub result: CheckResult,
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
            status: "Güncellemeler kontrol ediliyor…".into(),
            status_err: false,
            quit: false,
            rx,
            tx,
            last_check: None,
        }
    }

    /// Üç kaynağı da paralel thread'lerde kontrol et; sonuçlar kanaldan gelir.
    pub fn start_refresh(&mut self) {
        self.status = "Güncellemeler kontrol ediliyor…".into();
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
                    self.status = format!("{} kontrolü başarısız: {e}", msg.source.label());
                    self.status_err = true;
                }
            }
        }
        if got && self.checking == 0 {
            self.last_check = Some(Instant::now());
            if !self.status_err {
                let total = self.updates.len();
                self.status = match total {
                    0 => "Güncelleme yok — sistem güncel.".into(),
                    n => format!("{n} güncellenebilir paket."),
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

    fn move_sel(&mut self, delta: isize, page: usize) {
        let n = self.tab_len();
        if n == 0 {
            return;
        }
        let new = if delta > 0 {
            (self.sel + delta as usize).min(n - 1)
        } else if delta < 0 {
            self.sel.saturating_sub((-delta) as usize)
        } else if delta == 0 && page > 0 {
            self.sel
        } else {
            self.sel
        };
        let _ = page;
        self.sel = new;
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
        self.status = format!("Veritabanı tazeleniyor ({wrapper} pacman -Sy)…");
        self.status_err = false;
        let mut cmd = sources::root_command("pacman", &["-Sy"]);
        match runner::run_suspended(&mut cmd) {
            Ok(st) if st.success() => {
                self.status = "Veritabanı tazelendi, yeniden kontrol ediliyor…".into();
            }
            Ok(st) => {
                self.status = format!("Veritabanı tazelenemedi ({st})");
                self.status_err = true;
                return;
            }
            Err(e) => {
                self.status = format!("Veritabanı tazeleme başlatılamadı: {e}");
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
            self.status = "Seçili paket yok — space ile seç, u ile uygula.".into();
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
                    history::record(src.label(), &format!("başarısız ({st})"), &names);
                    self.status = format!("{} güncellemesi tamamlanmadı ({st})", src.label());
                    self.status_err = true;
                    break;
                }
                Err(e) => {
                    history::record(src.label(), &format!("başlatılamadı: {e}"), &names);
                    self.status = format!("{} başlatılamadı: {e}", src.label());
                    self.status_err = true;
                    break;
                }
            }
        }
        if !self.status_err {
            self.status = "Güncellemeler uygulandı, yeniden kontrol ediliyor…".into();
        }
        self.history = history::load();
        self.spawn_checks();
    }

    fn on_key(&mut self, k: KeyEvent) -> std::io::Result<()> {
        if k.modifiers == KeyModifiers::CONTROL && k.code == KeyCode::Char('c') {
            self.quit = true;
            return Ok(());
        }
        match k.code {
            KeyCode::Char('q') | KeyCode::Esc => self.quit = true,
            KeyCode::Char('j') | KeyCode::Down => self.move_sel(1, 0),
            KeyCode::Char('k') | KeyCode::Up => self.move_sel(-1, 0),
            KeyCode::PageDown => self.move_sel(10, 0),
            KeyCode::PageUp => self.move_sel(-10, 0),
            KeyCode::Home => self.sel = 0,
            KeyCode::End => self.move_sel(isize::MAX, 0),
            KeyCode::Char(' ') => self.toggle_current(),
            KeyCode::Char('a') => self.toggle_all_tab(),
            KeyCode::Char('r') => self.start_refresh(),
            KeyCode::Char('R') => self.sync_db(),
            KeyCode::Char('u') | KeyCode::Enter => self.apply(),
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
        if event::poll(Duration::from_millis(120))? {
            if let Event::Key(k) = event::read()? {
                if k.kind == KeyEventKind::Press {
                    self.on_key(k)?;
                }
            }
        }
        self.drain();
        Ok(())
    }

    // ------------------------------- çizim -------------------------------

    /// Geçmiş sekmesi: uygulanan güncellemeler, en yeni üstte.
    fn draw_history(&mut self, f: &mut Frame, area: Rect, block: Block) {
        if self.history.is_empty() {
            f.render_widget(
                Paragraph::new("henüz güncelleme uygulanmadı — 'u' sonrası kayıtlar burada")
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
            let mut style = Style::new().fg(if e.result == "ok" {
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
                " geçmiş ".to_string()
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
            "   j/k hareket · space seçim · a tümünü · r yenile · R db tazele · u güncelle · q çık",
            Style::new().fg(Color::DarkGray),
        ));
        f.render_widget(Paragraph::new(Line::from(spans)), tabs_area);

        // Liste
        let block = Block::bordered().title(" updateo · Arch güncelleyici ");
        let items = self.tab_items();
        if self.is_hist_tab() {
            self.draw_history(f, list_area, block);
        } else if items.is_empty() {
            let msg = if self.checking > 0 {
                "kontrol ediliyor…"
            } else {
                "güncellenecek paket yok — 'r' ile yenile"
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

        // Durum satırı
        let style = if self.status_err {
            Style::new().fg(Color::Red)
        } else if self.checking > 0 {
            Style::new().fg(Color::Yellow)
        } else {
            Style::new().fg(Color::Green)
        };
        let mut status = self.status.clone();
        if let Some(t) = self.last_check {
            status.push_str(&format!(
                "  ·  son kontrol {}sn önce",
                t.elapsed().as_secs()
            ));
        }
        f.render_widget(
            Paragraph::new(Line::from(Span::styled(status, style))),
            status_area,
        );
    }
}

fn truncate(s: &str, w: usize) -> String {
    let mut n = 0;
    for (i, c) in s.char_indices() {
        if n + c.len_utf8() > w {
            return format!("{}…", &s[..i]);
        }
        n += c.len_utf8();
    }
    s.to_string()
}
