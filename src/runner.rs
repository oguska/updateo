//! Güncelleme uygulama: TUI'yi geçici olarak askıya al ve çocuğa gerçek TTY'yi
//! bırak. Böylece sudo parolası, pacman onayları ve yay derleme akışı birebir
//! yerel deneyimle çalışır; TUI bittiğinde kaldığı yerden devam eder.

use crossterm::{
    execute,
    terminal::{
        Clear, ClearType, EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode,
        enable_raw_mode,
    },
};
use std::io;
use std::process::{Command, ExitStatus};

pub fn run_suspended(cmd: &mut Command) -> io::Result<ExitStatus> {
    disable_raw_mode()?;
    let mut out = io::stdout();
    execute!(out, LeaveAlternateScreen)?;

    let status = cmd.status(); // stdin/stdout/stderr çocuğa devredilir

    enable_raw_mode()?;
    execute!(out, EnterAlternateScreen, Clear(ClearType::All))?;
    status
}
