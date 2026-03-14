use std::fmt;
#[cfg(unix)]
use std::fs::OpenOptions;
use std::future::Future;
use std::io::IsTerminal;
#[cfg(unix)]
use std::io::Read;
use std::io::Result;
use std::io::Stdout;
use std::io::Write;
use std::io::stdin;
use std::io::stdout;
#[cfg(unix)]
use std::os::fd::AsRawFd;
#[cfg(unix)]
use std::os::unix::fs::OpenOptionsExt;
use std::panic;
use std::pin::Pin;
use std::sync::Arc;
use std::sync::Mutex;
use std::sync::atomic::AtomicBool;
use std::sync::atomic::Ordering;
use std::time::Duration;
#[cfg(unix)]
use std::time::Instant;

use crossterm::Command;
use crossterm::SynchronizedUpdate;
use crossterm::event::DisableBracketedPaste;
use crossterm::event::DisableFocusChange;
use crossterm::event::EnableBracketedPaste;
use crossterm::event::EnableFocusChange;
use crossterm::event::KeyEvent;
use crossterm::event::KeyboardEnhancementFlags;
use crossterm::event::PopKeyboardEnhancementFlags;
use crossterm::event::PushKeyboardEnhancementFlags;
use crossterm::terminal::EnterAlternateScreen;
use crossterm::terminal::LeaveAlternateScreen;
use crossterm::terminal::supports_keyboard_enhancement;
use ratatui::backend::Backend;
use ratatui::backend::CrosstermBackend;
use ratatui::crossterm::execute;
use ratatui::crossterm::terminal::disable_raw_mode;
use ratatui::crossterm::terminal::enable_raw_mode;
use ratatui::layout::Offset;
use ratatui::layout::Rect;
use ratatui::text::Line;
use tokio::sync::broadcast;
use tokio_stream::Stream;

pub use self::frame_requester::FrameRequester;
use crate::custom_terminal;
use crate::custom_terminal::Terminal as CustomTerminal;
use crate::notifications::DesktopNotificationBackend;
use crate::notifications::detect_backend;
use crate::tui::event_stream::EventBroker;
use crate::tui::event_stream::TuiEventStream;
#[cfg(unix)]
use crate::tui::job_control::SuspendContext;
use codex_core::config::types::NotificationMethod;

mod event_stream;
mod frame_rate_limiter;
mod frame_requester;
#[cfg(unix)]
mod job_control;

/// Target frame interval for UI redraw scheduling.
pub(crate) const TARGET_FRAME_INTERVAL: Duration = frame_rate_limiter::MIN_FRAME_INTERVAL;

/// A type alias for the terminal type used in this application
pub type Terminal = CustomTerminal<CrosstermBackend<Stdout>>;
const DEFAULT_TERMINAL_TITLE: &str = "Codex";
const TERMINAL_TITLE_DISABLE_ENV: &str = "CODEX_DISABLE_TERMINAL_TITLE";
#[cfg(unix)]
const TERMINAL_TITLE_QUERY_TIMEOUT: Duration = Duration::from_millis(50);
static TERMINAL_TITLE_SAVED: AtomicBool = AtomicBool::new(false);
static TERMINAL_TITLE_CURRENT: Mutex<Option<String>> = Mutex::new(None);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum TerminalTitleTransport {
    Direct,
    TmuxPassthrough,
}

fn set_process_terminal_title(title: Option<String>) {
    let mut current = TERMINAL_TITLE_CURRENT
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    *current = title;
}

fn write_process_terminal_title(
    writer: &mut impl std::io::Write,
    transport: TerminalTitleTransport,
    preexisting_title: Option<&str>,
    context: Option<&str>,
) -> Result<()> {
    let mut current = TERMINAL_TITLE_CURRENT
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    write_terminal_title(writer, &mut current, transport, preexisting_title, context)
}

fn sanitize_terminal_title_component(text: &str) -> Option<String> {
    let text = text
        .chars()
        .filter(|c| !c.is_control())
        .collect::<String>()
        .trim()
        .to_string();

    if text.is_empty() { None } else { Some(text) }
}

fn format_terminal_title(context: Option<&str>, preexisting_title: Option<&str>) -> String {
    let codex_title = context
        .and_then(sanitize_terminal_title_component)
        .unwrap_or_else(|| DEFAULT_TERMINAL_TITLE.to_string());
    let preexisting_title = preexisting_title
        .and_then(sanitize_terminal_title_component)
        .filter(|title| title != &codex_title);

    match preexisting_title {
        Some(preexisting_title) => format!("{preexisting_title} | {codex_title}"),
        None => codex_title,
    }
}

fn write_terminal_title(
    writer: &mut impl std::io::Write,
    current_title: &mut Option<String>,
    transport: TerminalTitleTransport,
    preexisting_title: Option<&str>,
    context: Option<&str>,
) -> Result<()> {
    let title = format_terminal_title(context, preexisting_title);
    if current_title.as_ref() == Some(&title) {
        return Ok(());
    }

    match transport {
        TerminalTitleTransport::Direct => {
            write!(writer, "\x1b]0;{title}\x07")?;
        }
        TerminalTitleTransport::TmuxPassthrough => {
            write!(writer, "\x1bPtmux;\x1b\x1b]0;{title}\x07\x1b\\")?;
        }
    }
    writer.flush()?;
    *current_title = Some(title);
    Ok(())
}

fn save_terminal_title(writer: &mut impl std::io::Write) -> Result<()> {
    write!(writer, "\x1b[22;0t")?;
    writer.flush()?;
    Ok(())
}

fn restore_terminal_title(writer: &mut impl std::io::Write) -> Result<()> {
    write!(writer, "\x1b[23;0t")?;
    writer.flush()?;
    Ok(())
}

fn restore_saved_terminal_title(writer: &mut impl std::io::Write) -> Result<bool> {
    if !TERMINAL_TITLE_SAVED.swap(false, Ordering::Relaxed) {
        return Ok(false);
    }

    restore_terminal_title(writer)?;
    set_process_terminal_title(None);
    Ok(true)
}

fn restore_saved_terminal_title_best_effort() {
    let mut output = stdout();
    if let Err(err) = restore_saved_terminal_title(&mut output) {
        tracing::warn!("failed to restore terminal title during cleanup: {err}");
    }
}

fn clear_process_terminal_title() {
    set_process_terminal_title(None);
}

#[cfg(unix)]
fn parse_terminal_title_response(response: &[u8]) -> Option<String> {
    let start = response.windows(3).position(|window| window == b"\x1b]l")? + 3;
    let payload = &response[start..];
    let bel_end = payload.iter().position(|&byte| byte == b'\x07');
    let st_end = payload.windows(2).position(|window| window == b"\x1b\\");
    let end = match (bel_end, st_end) {
        (Some(bel_end), Some(st_end)) => bel_end.min(st_end),
        (Some(bel_end), None) => bel_end,
        (None, Some(st_end)) => st_end,
        (None, None) => return None,
    };

    Some(String::from_utf8_lossy(&payload[..end]).into_owned())
}

#[cfg(unix)]
fn query_preexisting_terminal_title(transport: Option<TerminalTitleTransport>) -> Option<String> {
    if !matches!(transport, Some(TerminalTitleTransport::Direct))
        || !stdin().is_terminal()
        || !stdout().is_terminal()
    {
        return None;
    }

    let mut tty = OpenOptions::new()
        .read(true)
        .write(true)
        .custom_flags(libc::O_NONBLOCK)
        .open("/dev/tty")
        .ok()?;

    tty.write_all(b"\x1b[21t").ok()?;
    tty.flush().ok()?;

    let deadline = Instant::now() + TERMINAL_TITLE_QUERY_TIMEOUT;
    let mut response = Vec::new();

    loop {
        let remaining = deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            break;
        }

        let timeout_ms = remaining.as_millis().min(i32::MAX as u128) as i32;
        let mut pollfd = libc::pollfd {
            fd: tty.as_raw_fd(),
            events: libc::POLLIN,
            revents: 0,
        };
        let poll_result = unsafe { libc::poll(&mut pollfd, 1, timeout_ms) };
        if poll_result == 0 {
            break;
        }
        if poll_result < 0 {
            let err = std::io::Error::last_os_error();
            if err.kind() == std::io::ErrorKind::Interrupted {
                continue;
            }
            return None;
        }

        let mut chunk = [0u8; 256];
        loop {
            match tty.read(&mut chunk) {
                Ok(0) => break,
                Ok(bytes_read) => {
                    response.extend_from_slice(&chunk[..bytes_read]);
                    if let Some(title) = parse_terminal_title_response(&response) {
                        return sanitize_terminal_title_component(&title);
                    }
                }
                Err(err) if err.kind() == std::io::ErrorKind::WouldBlock => break,
                Err(err) if err.kind() == std::io::ErrorKind::Interrupted => continue,
                Err(_) => return None,
            }
        }
    }

    parse_terminal_title_response(&response)
        .and_then(|title| sanitize_terminal_title_component(&title))
}

#[cfg(not(unix))]
fn query_preexisting_terminal_title(_transport: Option<TerminalTitleTransport>) -> Option<String> {
    None
}

fn terminal_title_transport_for_env(
    term: &str,
    has_tmux: bool,
    has_sty: bool,
) -> Option<TerminalTitleTransport> {
    if has_tmux {
        return Some(TerminalTitleTransport::TmuxPassthrough);
    }
    if has_sty || term.starts_with("screen") || term.starts_with("tmux") {
        return None;
    }
    Some(TerminalTitleTransport::Direct)
}

fn terminal_title_transport() -> Option<TerminalTitleTransport> {
    let term = std::env::var("TERM").unwrap_or_default();
    terminal_title_transport_for_env(
        &term,
        std::env::var_os("TMUX").is_some(),
        std::env::var_os("STY").is_some(),
    )
}

fn terminal_title_restore_supported(transport: Option<TerminalTitleTransport>) -> bool {
    matches!(transport, Some(TerminalTitleTransport::Direct))
}

fn terminal_title_disabled(value: Option<&str>) -> bool {
    matches!(value, Some("1") | Some("true") | Some("TRUE"))
}

fn terminal_title_enabled() -> bool {
    !terminal_title_disabled(std::env::var(TERMINAL_TITLE_DISABLE_ENV).ok().as_deref())
}

pub fn set_modes() -> Result<()> {
    execute!(stdout(), EnableBracketedPaste)?;

    enable_raw_mode()?;
    // Enable keyboard enhancement flags so modifiers for keys like Enter are disambiguated.
    // chat_composer.rs is using a keyboard event listener to enter for any modified keys
    // to create a new line that require this.
    // Some terminals (notably legacy Windows consoles) do not support
    // keyboard enhancement flags. Attempt to enable them, but continue
    // gracefully if unsupported.
    let _ = execute!(
        stdout(),
        PushKeyboardEnhancementFlags(
            KeyboardEnhancementFlags::DISAMBIGUATE_ESCAPE_CODES
                | KeyboardEnhancementFlags::REPORT_EVENT_TYPES
                | KeyboardEnhancementFlags::REPORT_ALTERNATE_KEYS
        )
    );

    let _ = execute!(stdout(), EnableFocusChange);
    Ok(())
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct EnableAlternateScroll;

impl Command for EnableAlternateScroll {
    fn write_ansi(&self, f: &mut impl fmt::Write) -> fmt::Result {
        write!(f, "\x1b[?1007h")
    }

    #[cfg(windows)]
    fn execute_winapi(&self) -> Result<()> {
        Err(std::io::Error::other(
            "tried to execute EnableAlternateScroll using WinAPI; use ANSI instead",
        ))
    }

    #[cfg(windows)]
    fn is_ansi_code_supported(&self) -> bool {
        true
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct DisableAlternateScroll;

impl Command for DisableAlternateScroll {
    fn write_ansi(&self, f: &mut impl fmt::Write) -> fmt::Result {
        write!(f, "\x1b[?1007l")
    }

    #[cfg(windows)]
    fn execute_winapi(&self) -> Result<()> {
        Err(std::io::Error::other(
            "tried to execute DisableAlternateScroll using WinAPI; use ANSI instead",
        ))
    }

    #[cfg(windows)]
    fn is_ansi_code_supported(&self) -> bool {
        true
    }
}

fn restore_common(should_disable_raw_mode: bool) -> Result<()> {
    // Pop may fail on platforms that didn't support the push; ignore errors.
    let _ = execute!(stdout(), PopKeyboardEnhancementFlags);
    execute!(stdout(), DisableBracketedPaste)?;
    let _ = execute!(stdout(), DisableFocusChange);
    if should_disable_raw_mode {
        disable_raw_mode()?;
    }
    let _ = execute!(stdout(), crossterm::cursor::Show);
    Ok(())
}

/// Restore the terminal to its original state.
/// Inverse of `set_modes`.
pub fn restore() -> Result<()> {
    let should_disable_raw_mode = true;
    restore_common(should_disable_raw_mode)
}

/// Restore the terminal to its original state, but keep raw mode enabled.
pub fn restore_keep_raw() -> Result<()> {
    let should_disable_raw_mode = false;
    restore_common(should_disable_raw_mode)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RestoreMode {
    #[allow(dead_code)]
    Full, // Fully restore the terminal (disables raw mode).
    KeepRaw, // Restore the terminal but keep raw mode enabled.
}

impl RestoreMode {
    fn restore(self) -> Result<()> {
        match self {
            RestoreMode::Full => restore(),
            RestoreMode::KeepRaw => restore_keep_raw(),
        }
    }
}

/// Flush the underlying stdin buffer to clear any input that may be buffered at the terminal level.
/// For example, clears any user input that occurred while the crossterm EventStream was dropped.
#[cfg(unix)]
fn flush_terminal_input_buffer() {
    // Safety: flushing the stdin queue is safe and does not move ownership.
    let result = unsafe { libc::tcflush(libc::STDIN_FILENO, libc::TCIFLUSH) };
    if result != 0 {
        let err = std::io::Error::last_os_error();
        tracing::warn!("failed to tcflush stdin: {err}");
    }
}

/// Flush the underlying stdin buffer to clear any input that may be buffered at the terminal level.
/// For example, clears any user input that occurred while the crossterm EventStream was dropped.
#[cfg(windows)]
fn flush_terminal_input_buffer() {
    use windows_sys::Win32::Foundation::GetLastError;
    use windows_sys::Win32::Foundation::INVALID_HANDLE_VALUE;
    use windows_sys::Win32::System::Console::FlushConsoleInputBuffer;
    use windows_sys::Win32::System::Console::GetStdHandle;
    use windows_sys::Win32::System::Console::STD_INPUT_HANDLE;

    let handle = unsafe { GetStdHandle(STD_INPUT_HANDLE) };
    if handle == INVALID_HANDLE_VALUE || handle == 0 {
        let err = unsafe { GetLastError() };
        tracing::warn!("failed to get stdin handle for flush: error {err}");
        return;
    }

    let result = unsafe { FlushConsoleInputBuffer(handle) };
    if result == 0 {
        let err = unsafe { GetLastError() };
        tracing::warn!("failed to flush stdin buffer: error {err}");
    }
}

#[cfg(not(any(unix, windows)))]
pub(crate) fn flush_terminal_input_buffer() {}

/// Initialize the terminal (inline viewport; history stays in normal scrollback)
pub fn init() -> Result<Terminal> {
    if !stdin().is_terminal() {
        return Err(std::io::Error::other("stdin is not a terminal"));
    }
    if !stdout().is_terminal() {
        return Err(std::io::Error::other("stdout is not a terminal"));
    }
    set_modes()?;

    flush_terminal_input_buffer();

    set_panic_hook();

    let backend = CrosstermBackend::new(stdout());
    let tui = CustomTerminal::with_options(backend)?;
    Ok(tui)
}

fn set_panic_hook() {
    let hook = panic::take_hook();
    panic::set_hook(Box::new(move |panic_info| {
        restore_saved_terminal_title_best_effort();
        let _ = restore(); // ignore any errors as we are already failing
        let _ = panic::catch_unwind(panic::AssertUnwindSafe(|| hook(panic_info)));
    }));
}

#[derive(Clone, Debug)]
pub enum TuiEvent {
    Key(KeyEvent),
    Paste(String),
    Draw,
}

pub struct Tui {
    frame_requester: FrameRequester,
    draw_tx: broadcast::Sender<()>,
    event_broker: Arc<EventBroker>,
    pub(crate) terminal: Terminal,
    pending_history_lines: Vec<Line<'static>>,
    alt_saved_viewport: Option<ratatui::layout::Rect>,
    #[cfg(unix)]
    suspend_context: SuspendContext,
    // True when overlay alt-screen UI is active
    alt_screen_active: Arc<AtomicBool>,
    // True when terminal/tab is focused; updated internally from crossterm events
    terminal_focused: Arc<AtomicBool>,
    enhanced_keys_supported: bool,
    notification_backend: Option<DesktopNotificationBackend>,
    // When false, enter_alt_screen() becomes a no-op (for Zellij scrollback support)
    alt_screen_enabled: bool,
    terminal_title_enabled: bool,
    terminal_title_transport: Option<TerminalTitleTransport>,
    terminal_title_restore_supported: bool,
    preexisting_terminal_title: Option<String>,
}

impl Tui {
    pub fn new(terminal: Terminal) -> Self {
        let (draw_tx, _) = broadcast::channel(1);
        let frame_requester = FrameRequester::new(draw_tx.clone());
        let terminal_title_enabled = terminal_title_enabled();
        let terminal_title_transport = terminal_title_transport();
        let preexisting_terminal_title = if terminal_title_enabled {
            query_preexisting_terminal_title(terminal_title_transport)
        } else {
            None
        };

        // Detect keyboard enhancement support before any EventStream is created so the
        // crossterm poller can acquire its lock without contention.
        let enhanced_keys_supported = supports_keyboard_enhancement().unwrap_or(false);
        // Cache this to avoid contention with the event reader.
        supports_color::on_cached(supports_color::Stream::Stdout);
        let _ = crate::terminal_palette::default_colors();

        Self {
            frame_requester,
            draw_tx,
            event_broker: Arc::new(EventBroker::new()),
            terminal,
            pending_history_lines: vec![],
            alt_saved_viewport: None,
            #[cfg(unix)]
            suspend_context: SuspendContext::new(),
            alt_screen_active: Arc::new(AtomicBool::new(false)),
            terminal_focused: Arc::new(AtomicBool::new(true)),
            enhanced_keys_supported,
            notification_backend: Some(detect_backend(NotificationMethod::default())),
            alt_screen_enabled: true,
            terminal_title_enabled,
            terminal_title_transport,
            terminal_title_restore_supported: terminal_title_restore_supported(
                terminal_title_transport,
            ),
            preexisting_terminal_title,
        }
    }

    /// Set whether alternate screen is enabled. When false, enter_alt_screen() becomes a no-op.
    pub fn set_alt_screen_enabled(&mut self, enabled: bool) {
        self.alt_screen_enabled = enabled;
    }

    pub fn set_notification_method(&mut self, method: NotificationMethod) {
        self.notification_backend = Some(detect_backend(method));
    }

    pub fn set_title_context(&mut self, context: Option<&str>) -> Result<()> {
        if !self.terminal_title_enabled {
            return Ok(());
        }

        let Some(transport) = self.terminal_title_transport else {
            return Ok(());
        };

        let backend = self.terminal.backend_mut();
        if self.terminal_title_restore_supported && !TERMINAL_TITLE_SAVED.load(Ordering::Relaxed) {
            save_terminal_title(backend)?;
            TERMINAL_TITLE_SAVED.store(true, Ordering::Relaxed);
        }

        write_process_terminal_title(
            backend,
            transport,
            self.preexisting_terminal_title.as_deref(),
            context,
        )
    }

    pub fn restore_title(&mut self) -> Result<()> {
        if !self.terminal_title_enabled {
            return Ok(());
        }

        if !self.terminal_title_restore_supported || !TERMINAL_TITLE_SAVED.load(Ordering::Relaxed) {
            clear_process_terminal_title();
            return Ok(());
        }

        let backend = self.terminal.backend_mut();
        restore_terminal_title(backend)?;
        TERMINAL_TITLE_SAVED.store(false, Ordering::Relaxed);
        set_process_terminal_title(None);
        Ok(())
    }

    pub fn frame_requester(&self) -> FrameRequester {
        self.frame_requester.clone()
    }

    pub fn enhanced_keys_supported(&self) -> bool {
        self.enhanced_keys_supported
    }

    pub fn is_alt_screen_active(&self) -> bool {
        self.alt_screen_active.load(Ordering::Relaxed)
    }

    // Drop crossterm EventStream to avoid stdin conflicts with other processes.
    pub fn pause_events(&mut self) {
        self.event_broker.pause_events();
    }

    // Resume crossterm EventStream to resume stdin polling.
    // Inverse of `pause_events`.
    pub fn resume_events(&mut self) {
        self.event_broker.resume_events();
    }

    /// Temporarily restore terminal state to run an external interactive program `f`.
    ///
    /// This pauses crossterm's stdin polling by dropping the underlying event stream, restores
    /// terminal modes (optionally keeping raw mode enabled), then re-applies Codex TUI modes and
    /// flushes pending stdin input before resuming events.
    pub async fn with_restored<R, F, Fut>(&mut self, mode: RestoreMode, f: F) -> R
    where
        F: FnOnce() -> Fut,
        Fut: Future<Output = R>,
    {
        // Pause crossterm events to avoid stdin conflicts with external program `f`.
        self.pause_events();

        // Leave alt screen if active to avoid conflicts with external program `f`.
        let was_alt_screen = self.is_alt_screen_active();
        if was_alt_screen {
            let _ = self.leave_alt_screen();
        }

        if let Err(err) = mode.restore() {
            tracing::warn!("failed to restore terminal modes before external program: {err}");
        }

        let output = f().await;

        if let Err(err) = set_modes() {
            tracing::warn!("failed to re-enable terminal modes after external program: {err}");
        }
        // After the external program `f` finishes, reset terminal state and flush any buffered keypresses.
        flush_terminal_input_buffer();
        clear_process_terminal_title();

        if was_alt_screen {
            let _ = self.enter_alt_screen();
        }

        self.resume_events();
        output
    }

    /// Emit a desktop notification now if the terminal is unfocused.
    /// Returns true if a notification was posted.
    pub fn notify(&mut self, message: impl AsRef<str>) -> bool {
        if self.terminal_focused.load(Ordering::Relaxed) {
            return false;
        }

        let Some(backend) = self.notification_backend.as_mut() else {
            return false;
        };

        let message = message.as_ref().to_string();
        match backend.notify(&message) {
            Ok(()) => true,
            Err(err) => {
                let method = backend.method();
                tracing::warn!(
                    error = %err,
                    method = %method,
                    "Failed to emit terminal notification; disabling future notifications"
                );
                self.notification_backend = None;
                false
            }
        }
    }

    pub fn event_stream(&self) -> Pin<Box<dyn Stream<Item = TuiEvent> + Send + 'static>> {
        #[cfg(unix)]
        let stream = TuiEventStream::new(
            self.event_broker.clone(),
            self.draw_tx.subscribe(),
            self.terminal_focused.clone(),
            self.suspend_context.clone(),
            self.alt_screen_active.clone(),
        );
        #[cfg(not(unix))]
        let stream = TuiEventStream::new(
            self.event_broker.clone(),
            self.draw_tx.subscribe(),
            self.terminal_focused.clone(),
        );
        Box::pin(stream)
    }

    /// Enter alternate screen and expand the viewport to full terminal size, saving the current
    /// inline viewport for restoration when leaving.
    pub fn enter_alt_screen(&mut self) -> Result<()> {
        if !self.alt_screen_enabled {
            return Ok(());
        }
        let _ = execute!(self.terminal.backend_mut(), EnterAlternateScreen);
        // Enable "alternate scroll" so terminals may translate wheel to arrows
        let _ = execute!(self.terminal.backend_mut(), EnableAlternateScroll);
        if let Ok(size) = self.terminal.size() {
            self.alt_saved_viewport = Some(self.terminal.viewport_area);
            self.terminal.set_viewport_area(ratatui::layout::Rect::new(
                0,
                0,
                size.width,
                size.height,
            ));
            let _ = self.terminal.clear();
        }
        self.alt_screen_active.store(true, Ordering::Relaxed);
        Ok(())
    }

    /// Leave alternate screen and restore the previously saved inline viewport, if any.
    pub fn leave_alt_screen(&mut self) -> Result<()> {
        if !self.alt_screen_enabled {
            return Ok(());
        }
        // Disable alternate scroll when leaving alt-screen
        let _ = execute!(self.terminal.backend_mut(), DisableAlternateScroll);
        let _ = execute!(self.terminal.backend_mut(), LeaveAlternateScreen);
        if let Some(saved) = self.alt_saved_viewport.take() {
            self.terminal.set_viewport_area(saved);
        }
        self.alt_screen_active.store(false, Ordering::Relaxed);
        Ok(())
    }

    pub fn insert_history_lines(&mut self, lines: Vec<Line<'static>>) {
        self.pending_history_lines.extend(lines);
        self.frame_requester().schedule_frame();
    }

    pub fn clear_pending_history_lines(&mut self) {
        self.pending_history_lines.clear();
    }

    pub fn draw(
        &mut self,
        height: u16,
        draw_fn: impl FnOnce(&mut custom_terminal::Frame),
    ) -> Result<()> {
        // If we are resuming from ^Z, we need to prepare the resume action now so we can apply it
        // in the synchronized update.
        #[cfg(unix)]
        let mut prepared_resume = self
            .suspend_context
            .prepare_resume_action(&mut self.terminal, &mut self.alt_saved_viewport);

        // Precompute any viewport updates that need a cursor-position query before entering
        // the synchronized update, to avoid racing with the event reader.
        let mut pending_viewport_area = self.pending_viewport_area()?;

        stdout().sync_update(|_| {
            #[cfg(unix)]
            if let Some(prepared) = prepared_resume.take() {
                prepared.apply(&mut self.terminal)?;
            }

            let terminal = &mut self.terminal;
            if let Some(new_area) = pending_viewport_area.take() {
                terminal.set_viewport_area(new_area);
                terminal.clear()?;
            }

            let size = terminal.size()?;

            let mut area = terminal.viewport_area;
            area.height = height.min(size.height);
            area.width = size.width;
            // If the viewport has expanded, scroll everything else up to make room.
            if area.bottom() > size.height {
                terminal
                    .backend_mut()
                    .scroll_region_up(0..area.top(), area.bottom() - size.height)?;
                area.y = size.height - area.height;
            }
            if area != terminal.viewport_area {
                // TODO(nornagon): probably this could be collapsed with the clear + set_viewport_area above.
                terminal.clear()?;
                terminal.set_viewport_area(area);
            }

            if !self.pending_history_lines.is_empty() {
                crate::insert_history::insert_history_lines(
                    terminal,
                    self.pending_history_lines.clone(),
                )?;
                self.pending_history_lines.clear();
            }

            // Update the y position for suspending so Ctrl-Z can place the cursor correctly.
            #[cfg(unix)]
            {
                let inline_area_bottom = if self.alt_screen_active.load(Ordering::Relaxed) {
                    self.alt_saved_viewport
                        .map(|r| r.bottom().saturating_sub(1))
                        .unwrap_or_else(|| area.bottom().saturating_sub(1))
                } else {
                    area.bottom().saturating_sub(1)
                };
                self.suspend_context.set_cursor_y(inline_area_bottom);
            }

            terminal.draw(|frame| {
                draw_fn(frame);
            })
        })?
    }

    fn pending_viewport_area(&mut self) -> Result<Option<Rect>> {
        let terminal = &mut self.terminal;
        let screen_size = terminal.size()?;
        let last_known_screen_size = terminal.last_known_screen_size;
        if screen_size != last_known_screen_size
            && let Ok(cursor_pos) = terminal.get_cursor_position()
        {
            let last_known_cursor_pos = terminal.last_known_cursor_pos;
            // If we resized AND the cursor moved, we adjust the viewport area to keep the
            // cursor in the same position. This is a heuristic that seems to work well
            // at least in iTerm2.
            if cursor_pos.y != last_known_cursor_pos.y {
                let offset = Offset {
                    x: 0,
                    y: cursor_pos.y as i32 - last_known_cursor_pos.y as i32,
                };
                return Ok(Some(terminal.viewport_area.offset(offset)));
            }
        }
        Ok(None)
    }
}

#[cfg(test)]
mod tests {
    use super::DEFAULT_TERMINAL_TITLE;
    use super::TERMINAL_TITLE_SAVED;
    use super::TerminalTitleTransport;
    use super::format_terminal_title;
    #[cfg(unix)]
    use super::parse_terminal_title_response;
    use super::restore_saved_terminal_title;
    use super::terminal_title_disabled;
    use super::terminal_title_restore_supported;
    use super::terminal_title_transport_for_env;
    use super::write_terminal_title;
    use pretty_assertions::assert_eq;
    use std::sync::atomic::Ordering;

    #[test]
    fn terminal_title_defaults_to_codex() {
        assert_eq!(format_terminal_title(None, None), DEFAULT_TERMINAL_TITLE);
        assert_eq!(
            format_terminal_title(Some("   "), None),
            DEFAULT_TERMINAL_TITLE
        );
    }

    #[test]
    fn terminal_title_includes_context() {
        assert_eq!(
            format_terminal_title(Some("fix title syncing"), None),
            "fix title syncing"
        );
    }

    #[test]
    fn terminal_title_preserves_spinner_prefixed_context() {
        assert_eq!(format_terminal_title(Some("⠋ Codex"), None), "⠋ Codex");
        assert_eq!(
            format_terminal_title(Some("⠋ named thread"), None),
            "⠋ named thread"
        );
    }

    #[test]
    fn terminal_title_strips_control_characters() {
        assert_eq!(
            format_terminal_title(Some("hello\x1b\t\n\r\u{7}world"), None),
            "helloworld"
        );
    }

    #[test]
    fn terminal_title_appends_preexisting_title() {
        assert_eq!(
            format_terminal_title(Some("⠋ Codex"), Some("Terminal Title")),
            "Terminal Title | ⠋ Codex"
        );
        assert_eq!(
            format_terminal_title(None, Some("Terminal Title")),
            "Terminal Title | Codex"
        );
    }

    #[test]
    fn terminal_title_ignores_empty_or_duplicate_preexisting_title() {
        assert_eq!(format_terminal_title(Some("plan"), Some("   ")), "plan");
        assert_eq!(format_terminal_title(Some("plan"), Some("plan")), "plan");
    }

    #[test]
    fn terminal_title_write_is_deduplicated() {
        let mut output = Vec::new();
        let mut current_title = None;

        write_terminal_title(
            &mut output,
            &mut current_title,
            TerminalTitleTransport::Direct,
            None,
            Some("plan"),
        )
        .expect("first title write should succeed");
        write_terminal_title(
            &mut output,
            &mut current_title,
            TerminalTitleTransport::Direct,
            None,
            Some("plan"),
        )
        .expect("duplicate title write should succeed");

        assert_eq!(output, b"\x1b]0;plan\x07");
        assert_eq!(current_title, Some("plan".to_string()));
    }

    #[test]
    fn terminal_title_write_uses_tmux_passthrough() {
        let mut output = Vec::new();
        let mut current_title = None;

        write_terminal_title(
            &mut output,
            &mut current_title,
            TerminalTitleTransport::TmuxPassthrough,
            Some("Terminal Title"),
            Some("plan"),
        )
        .expect("tmux title write should succeed");

        assert_eq!(
            output,
            b"\x1bPtmux;\x1b\x1b]0;Terminal Title | plan\x07\x1b\\"
        );
        assert_eq!(current_title, Some("Terminal Title | plan".to_string()));
    }

    #[cfg(unix)]
    #[test]
    fn terminal_title_query_parser_accepts_st_and_bel_terminators() {
        assert_eq!(
            parse_terminal_title_response(b"\x1b]lTerminal Title\x1b\\"),
            Some("Terminal Title".to_string())
        );
        assert_eq!(
            parse_terminal_title_response(b"\x1b]lTerminal Title\x07"),
            Some("Terminal Title".to_string())
        );
    }

    #[test]
    fn restore_saved_terminal_title_writes_once_and_clears_flag() {
        TERMINAL_TITLE_SAVED.store(true, Ordering::Relaxed);

        let mut output = Vec::new();
        assert_eq!(
            restore_saved_terminal_title(&mut output).expect("restore should succeed"),
            true
        );
        assert_eq!(
            restore_saved_terminal_title(&mut output).expect("second restore should be a no-op"),
            false
        );

        assert_eq!(output, b"\x1b[23;0t");
        assert!(!TERMINAL_TITLE_SAVED.load(Ordering::Relaxed));
    }

    #[test]
    fn terminal_title_transport_uses_tmux_passthrough_in_tmux() {
        assert_eq!(
            terminal_title_transport_for_env("xterm-256color", true, false),
            Some(TerminalTitleTransport::TmuxPassthrough)
        );
        assert_eq!(
            terminal_title_transport_for_env("tmux-256color", false, false),
            None
        );
        assert_eq!(
            terminal_title_transport_for_env("screen-256color", false, false),
            None
        );
        assert_eq!(
            terminal_title_transport_for_env("xterm-256color", false, true),
            None
        );
        assert_eq!(
            terminal_title_transport_for_env("xterm-256color", false, false),
            Some(TerminalTitleTransport::Direct)
        );
    }

    #[test]
    fn terminal_title_restore_support_is_only_available_for_direct_titles() {
        assert!(terminal_title_restore_supported(Some(
            TerminalTitleTransport::Direct
        )));
        assert!(!terminal_title_restore_supported(Some(
            TerminalTitleTransport::TmuxPassthrough
        )));
        assert!(!terminal_title_restore_supported(None));
    }

    #[test]
    fn direct_terminal_title_transport_remains_supported() {
        assert_eq!(
            terminal_title_transport_for_env("xterm-256color", false, false),
            Some(TerminalTitleTransport::Direct)
        );
    }

    #[test]
    fn terminal_title_disable_env_is_case_sensitive_to_supported_values() {
        assert!(terminal_title_disabled(Some("1")));
        assert!(terminal_title_disabled(Some("true")));
        assert!(terminal_title_disabled(Some("TRUE")));
        assert!(!terminal_title_disabled(Some("True")));
        assert!(!terminal_title_disabled(Some("0")));
        assert!(!terminal_title_disabled(None));
    }
}
