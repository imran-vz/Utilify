//! The terminal: colour, size, and the live view's screen handling. Plain
//! ANSI escapes and a few libc calls; no terminal library.
//!
//! The live view draws each frame as one string and writes it with one
//! call: cursor home, every line followed by clear-to-end-of-line, then
//! clear-below. Nothing is erased first, so frames don't flicker, and an
//! unchanged frame costs one small write.

use std::io::Write;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

/// The few colours the views use, from the basic 16 so every terminal
/// theme maps them to something readable.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Color {
    Down,
    Up,
    Ok,
    Warn,
    Danger,
    Accent,
    Dim,
    Bold,
}

impl Color {
    fn sgr(self) -> &'static str {
        match self {
            Color::Down => "36",
            Color::Up => "35",
            Color::Ok => "32",
            Color::Warn => "33",
            Color::Danger => "31",
            Color::Accent => "34",
            Color::Dim => "2",
            Color::Bold => "1",
        }
    }
}

/// Paints text when colour is on.
#[derive(Clone, Copy, Debug)]
pub struct Paint {
    pub on: bool,
}

impl Paint {
    /// Colour for a terminal, unless `NO_COLOR` is set or it was turned off.
    pub fn detect(disabled: bool) -> Self {
        let on = !disabled
            && std::env::var_os("NO_COLOR").is_none_or(|v| v.is_empty())
            && is_tty(libc::STDOUT_FILENO)
            && std::env::var("TERM").map_or(true, |t| t != "dumb");
        Paint { on }
    }

    pub fn paint(&self, c: Color, s: impl AsRef<str>) -> String {
        let s = s.as_ref();
        if self.on && !s.is_empty() {
            format!("\x1b[{}m{s}\x1b[0m", c.sgr())
        } else {
            s.to_string()
        }
    }

    pub fn dim(&self, s: impl AsRef<str>) -> String {
        self.paint(Color::Dim, s)
    }

    pub fn bold(&self, s: impl AsRef<str>) -> String {
        self.paint(Color::Bold, s)
    }
}

pub fn is_tty(fd: i32) -> bool {
    // SAFETY: a plain query on a file descriptor.
    unsafe { libc::isatty(fd) == 1 }
}

/// Columns and rows of the terminal on stdout; 80×24 when it isn't one.
pub fn size() -> (usize, usize) {
    // SAFETY: TIOCGWINSZ fills a winsize; zeroed is a valid start.
    let mut ws: libc::winsize = unsafe { std::mem::zeroed() };
    let ok = unsafe { libc::ioctl(libc::STDOUT_FILENO, libc::TIOCGWINSZ, &mut ws) } == 0;
    if ok && ws.ws_col > 0 {
        return (ws.ws_col as usize, ws.ws_row.max(1) as usize);
    }
    let cols = std::env::var("COLUMNS")
        .ok()
        .and_then(|c| c.parse().ok())
        .unwrap_or(80);
    (cols, 24)
}

/// Columns a character takes: two for wide ones (CJK, most emoji), none
/// for combining marks.
fn columns(c: char) -> usize {
    unicode_width::UnicodeWidthChar::width(c).unwrap_or(0)
}

/// Columns a string takes on screen, skipping the views' own ANSI escapes.
pub fn visible_len(s: &str) -> usize {
    let mut n = 0;
    let mut chars = s.chars();
    while let Some(c) = chars.next() {
        if c == '\x1b' {
            for c in chars.by_ref() {
                if c.is_ascii_alphabetic() {
                    break;
                }
            }
        } else {
            n += columns(c);
        }
    }
    n
}

/// Text from elsewhere (app names, preset names, error messages) made safe
/// to print: control characters, escapes among them, become spaces, so a
/// name can't move the cursor or change colours.
pub fn clean(s: &str) -> String {
    if !s.chars().any(char::is_control) {
        return s.to_string();
    }
    s.chars()
        .map(|c| if c.is_control() { ' ' } else { c })
        .collect()
}

/// `s` cut to `width` columns, keeping its escapes (and resetting colour if
/// it was cut mid-span).
pub fn fit(s: &str, width: usize) -> String {
    if visible_len(s) <= width {
        return s.to_string();
    }
    let mut out = String::with_capacity(s.len());
    let mut n = 0;
    let mut chars = s.chars();
    while let Some(c) = chars.next() {
        if c == '\x1b' {
            out.push(c);
            for c in chars.by_ref() {
                out.push(c);
                if c.is_ascii_alphabetic() {
                    break;
                }
            }
        } else if n + columns(c) <= width {
            out.push(c);
            n += columns(c);
        } else {
            // Full: skip the rest of the text, but keep escapes.
            n = width;
        }
    }
    out.push_str("\x1b[0m");
    out
}

/// Pad `s` with spaces to `width` columns.
pub fn pad(s: &str, width: usize) -> String {
    let n = visible_len(s);
    if n >= width {
        s.to_string()
    } else {
        format!("{s}{}", " ".repeat(width - n))
    }
}

/// Right-align `s` in `width` columns.
pub fn pad_left(s: &str, width: usize) -> String {
    let n = visible_len(s);
    if n >= width {
        s.to_string()
    } else {
        format!("{}{s}", " ".repeat(width - n))
    }
}

/// Set by SIGINT, SIGTERM and SIGHUP: leave the live view.
static STOP: AtomicBool = AtomicBool::new(false);
/// Set by SIGWINCH: redraw at the new size now.
static RESIZED: AtomicBool = AtomicBool::new(false);
/// Set by SIGTSTP (Ctrl-Z): hand the terminal back, then stop.
static SUSPEND: AtomicBool = AtomicBool::new(false);
/// Set by SIGCONT: take the terminal again (after `fg`, or a SIGSTOP the
/// view couldn't see coming).
static CONTINUED: AtomicBool = AtomicBool::new(false);

extern "C" fn on_suspend(_: libc::c_int) {
    SUSPEND.store(true, Ordering::Relaxed);
}

extern "C" fn on_continue(_: libc::c_int) {
    CONTINUED.store(true, Ordering::Relaxed);
}

extern "C" fn on_stop(_: libc::c_int) {
    STOP.store(true, Ordering::Relaxed);
}

extern "C" fn on_resize(_: libc::c_int) {
    RESIZED.store(true, Ordering::Relaxed);
}

pub fn stopped() -> bool {
    STOP.load(Ordering::Relaxed)
}

pub fn take_resized() -> bool {
    RESIZED.swap(false, Ordering::Relaxed)
}

fn install(signal: libc::c_int, handler: extern "C" fn(libc::c_int)) {
    // SAFETY: the handlers only store to atomics, which is signal-safe.
    // No SA_RESTART, so a blocking poll returns early with EINTR.
    unsafe {
        let mut sa: libc::sigaction = std::mem::zeroed();
        sa.sa_sigaction = handler as usize;
        libc::sigemptyset(&mut sa.sa_mask);
        libc::sigaction(signal, &sa, std::ptr::null_mut());
    }
}

/// Let Ctrl-C set [`stopped`] instead of killing the process, so a command
/// can clean up (cancel a speed test) before it exits.
pub fn catch_stop() {
    install(libc::SIGINT, on_stop);
    install(libc::SIGTERM, on_stop);
}

/// The live view's screen: the alternate screen with the cursor hidden,
/// and the keyboard read key by key without echo. Everything is put back
/// on drop, including after Ctrl-C (which only sets a flag) and panics.
pub struct Screen {
    /// The terminal as it was, and as the view sets it.
    saved: Option<libc::termios>,
    raw: Option<libc::termios>,
    /// Keys read but not handed out yet (`5q` typed fast arrives as one
    /// read: two keys).
    pending: std::cell::RefCell<Vec<u8>>,
}

impl Screen {
    pub fn enter() -> Self {
        install(libc::SIGINT, on_stop);
        install(libc::SIGTERM, on_stop);
        install(libc::SIGHUP, on_stop);
        install(libc::SIGWINCH, on_resize);
        install(libc::SIGTSTP, on_suspend);
        install(libc::SIGCONT, on_continue);
        let (saved, raw) = if is_tty(libc::STDIN_FILENO) {
            // SAFETY: tcgetattr fills a termios; stdin is a terminal.
            let mut t: libc::termios = unsafe { std::mem::zeroed() };
            unsafe { libc::tcgetattr(libc::STDIN_FILENO, &mut t) };
            let saved = t;
            // Keys arrive one at a time, unechoed; Ctrl-C and Ctrl-Z still
            // signal.
            t.c_lflag &= !(libc::ICANON | libc::ECHO);
            t.c_cc[libc::VMIN] = 0;
            t.c_cc[libc::VTIME] = 0;
            (Some(saved), Some(t))
        } else {
            (None, None)
        };
        let screen = Screen {
            saved,
            raw,
            pending: Default::default(),
        };
        screen.take();
        screen
    }

    /// Raw keys, the alternate screen, no cursor.
    fn take(&self) {
        if let Some(t) = &self.raw {
            // SAFETY: setting attributes read from this terminal.
            unsafe { libc::tcsetattr(libc::STDIN_FILENO, libc::TCSANOW, t) };
        }
        write_raw("\x1b[?1049h\x1b[?25l\x1b[H\x1b[2J");
    }

    /// The terminal as it was before.
    fn give_back(&self) {
        write_raw("\x1b[0m\x1b[?25h\x1b[?1049l");
        if let Some(t) = &self.saved {
            // SAFETY: restoring the attributes read in `enter`.
            unsafe { libc::tcsetattr(libc::STDIN_FILENO, libc::TCSANOW, t) };
        }
    }

    /// Ctrl-Z and `fg`: give the terminal back and stop; on continuing, take
    /// it again. True when the screen needs drawing from scratch.
    pub fn job_control(&self) -> bool {
        let mut redraw = false;
        if SUSPEND.swap(false, Ordering::Relaxed) {
            self.give_back();
            // SAFETY: stop with the default action, then catch it again.
            unsafe {
                libc::signal(libc::SIGTSTP, libc::SIG_DFL);
                libc::raise(libc::SIGTSTP);
            }
            // Running again after `fg`.
            install(libc::SIGTSTP, on_suspend);
            CONTINUED.store(true, Ordering::Relaxed);
        }
        if CONTINUED.swap(false, Ordering::Relaxed) {
            self.take();
            redraw = true;
        }
        redraw
    }

    /// Draw a frame of `lines`, cut to the screen.
    pub fn draw(&self, lines: &[String]) {
        let (cols, rows) = size();
        let mut frame = String::with_capacity(lines.len() * (cols + 8) + 16);
        frame.push_str("\x1b[H");
        for (i, line) in lines.iter().take(rows).enumerate() {
            if i > 0 {
                frame.push_str("\r\n");
            }
            frame.push_str(&fit(line, cols));
            frame.push_str("\x1b[K");
        }
        frame.push_str("\x1b[J");
        write_raw(&frame);
    }

    /// Wait up to `timeout` for a key. Returns early (with `None`) on a
    /// signal, so Ctrl-C and resizes act at once.
    pub fn key(&self, timeout: Duration) -> Option<u8> {
        if self.saved.is_none() {
            std::thread::sleep(timeout.min(Duration::from_millis(200)));
            return None;
        }
        let mut pending = self.pending.borrow_mut();
        let wait = |ms: i32| {
            let mut fds = libc::pollfd {
                fd: libc::STDIN_FILENO,
                events: libc::POLLIN,
                revents: 0,
            };
            // SAFETY: one valid pollfd.
            unsafe { libc::poll(&mut fds, 1, ms) > 0 }
        };
        let fill = |pending: &mut Vec<u8>| {
            let mut buf = [0u8; 64];
            let n = read_stdin(&mut buf);
            pending.extend_from_slice(&buf[..n]);
            n > 0
        };
        if pending.is_empty()
            && (!wait(timeout.as_millis().min(i32::MAX as u128) as i32) || !fill(&mut pending))
        {
            return None;
        }
        // An escape sequence can arrive in pieces: give the rest a moment
        // (up to about 120 ms in all) before taking Esc on its own or
        // dropping a cut-short sequence, so no piece of one is read as keys.
        let deadline = std::time::Instant::now() + Duration::from_millis(120);
        while incomplete(&pending) {
            let left = deadline.saturating_duration_since(std::time::Instant::now());
            if left.is_zero() {
                break;
            }
            // A quiet poll isn't the end: keep waiting out the deadline.
            if wait(left.as_millis().max(1) as i32) && !fill(&mut pending) {
                break;
            }
        }
        let (key, used) = parse_key(&pending);
        let used = used.min(pending.len());
        pending.drain(..used);
        key
    }
}

/// Whether `bytes` starts with an escape sequence that isn't finished yet.
fn incomplete(bytes: &[u8]) -> bool {
    match bytes {
        [0x1b] | [0x1b, b'O'] => true,
        [0x1b, b'[', rest @ ..] => !rest.iter().any(|c| (0x40..=0x7e).contains(c)),
        _ => false,
    }
}

/// The first key in `bytes`, and how many bytes it took. A lone Esc quits;
/// Right/Left and Shift-Tab switch tabs; other escape sequences (Up, Down,
/// function keys, Alt with a key) do nothing.
fn parse_key(bytes: &[u8]) -> (Option<u8>, usize) {
    match bytes {
        [] => (None, 0),
        [0x1b, b'[', rest @ ..] => {
            // CSI: parameters, then a final byte in @..~.
            match rest.iter().position(|c| (0x40..=0x7e).contains(c)) {
                Some(i) => {
                    let key = match rest[i] {
                        b'C' => Some(b'\t'),
                        b'D' | b'Z' => Some(KEY_BACK),
                        _ => None,
                    };
                    (key, 2 + i + 1)
                }
                // Cut short: drop it rather than read it as keys.
                None => (None, bytes.len()),
            }
        }
        // SS3 (arrows in application mode, F1–F4).
        [0x1b, b'O', fin, ..] => (
            match fin {
                b'C' => Some(b'\t'),
                b'D' => Some(KEY_BACK),
                _ => None,
            },
            3,
        ),
        [0x1b] | [0x1b, 0x1b, ..] => (Some(0x1b), 1),
        [0x1b, _, ..] => (None, 2),
        [k, ..] => (Some(*k), 1),
    }
}

/// Bytes read from stdin into `buf`, 0 on nothing or an error.
fn read_stdin(buf: &mut [u8]) -> usize {
    // SAFETY: reading into a caller's buffer from stdin.
    let n = unsafe { libc::read(libc::STDIN_FILENO, buf.as_mut_ptr().cast(), buf.len()) };
    n.max(0) as usize
}

/// Shift-Tab or the left arrow.
pub const KEY_BACK: u8 = 0x02;

impl Drop for Screen {
    fn drop(&mut self) {
        self.give_back();
    }
}

fn write_raw(s: &str) {
    let mut out = std::io::stdout().lock();
    let _ = out.write_all(s.as_bytes());
    let _ = out.flush();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn widths_skip_escapes() {
        let p = Paint { on: true };
        let s = format!("{} {}", p.paint(Color::Ok, "ok"), p.dim("·"));
        assert_eq!(visible_len(&s), 4);
        assert_eq!(visible_len("━━╸──"), 5);
        let cut = fit(&s, 2);
        assert_eq!(visible_len(&cut), 2);
        assert!(cut.ends_with("\x1b[0m"));
        assert_eq!(fit("abc", 5), "abc");
        // Wide characters count twice and aren't split.
        assert_eq!(visible_len("日本"), 4);
        assert_eq!(visible_len(&fit("日本語", 5)), 4);
        assert_eq!(clean("a\x1b[2Jb\nc"), "a [2Jb c");
    }

    #[test]
    fn keys_come_out_one_at_a_time() {
        assert_eq!(parse_key(b"5q"), (Some(b'5'), 1));
        assert_eq!(parse_key(b"q"), (Some(b'q'), 1));
        assert_eq!(parse_key(b"\x1b[C5"), (Some(b'\t'), 3));
        assert_eq!(parse_key(b"\x1b[D"), (Some(KEY_BACK), 3));
        assert_eq!(parse_key(b"\x1b[Z"), (Some(KEY_BACK), 3));
        assert_eq!(parse_key(b"\x1b[A"), (None, 3), "Up does nothing");
        assert_eq!(parse_key(b"\x1b[15~q"), (None, 5), "F5 does nothing");
        assert_eq!(parse_key(b"\x1bOC"), (Some(b'\t'), 3));
        assert_eq!(parse_key(b"\x1b"), (Some(0x1b), 1));
        assert_eq!(parse_key(b"\x1bx"), (None, 2), "Alt-x does nothing");
        assert_eq!(parse_key(b"\x1b["), (None, 2));
        assert!(incomplete(b"\x1b[1"), "F5 cut after ESC [ 1");
        assert!(incomplete(b"\x1b"));
        assert!(!incomplete(b"\x1b[15~"));
        assert!(!incomplete(b"\x1bx"));
        assert!(!incomplete(b"q"));
        assert_eq!(pad("ab", 4), "ab  ");
        assert_eq!(pad_left("ab", 4), "  ab");
    }

    #[test]
    fn colour_off_leaves_text_alone() {
        let p = Paint { on: false };
        assert_eq!(p.paint(Color::Danger, "x"), "x");
    }
}
