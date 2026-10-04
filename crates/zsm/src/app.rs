use std::cell::{Cell, RefCell};
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::rc::{Rc, Weak};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

use crossbeam_channel::Receiver;
use native_windows_gui as nwg;
use zsm_core::config::{Config, Kind, ServiceConfig, ServiceEntry};
use zsm_core::manager::{Manager, ManagerOptions};
use zsm_core::supervisor::{CtrlBreakHelper, Event, Service, State, StopMode};
use zsm_core::update::{self, Release};
use zsm_core::win::{self, RichEdit, WindowId};
use zsm_core::{paths, VERSION};

use crate::console::ConsoleView;
use crate::dialog::edit_service;
use crate::format::{memory, state_label, uptime};
use crate::summary::{allowed, summarize, TrayColor};
use crate::updater::{self, UpdateMsg};

pub const APP_NAME: &str = "Z Service Manager";
const RUN_VALUE: &str = "ZServiceManager";
const UI_TICK: Duration = Duration::from_millis(100);
const QUIT_WAIT: Duration = Duration::from_secs(3600);
const SESSION_END_WAIT: Duration = Duration::from_secs(90);
const WM_QUERYENDSESSION: u32 = 0x0011;
const WM_ENDSESSION: u32 = 0x0016;
const WM_GETDLGCODE: u32 = 0x0087;
/// DLGC_WANTMESSAGE | DLGC_WANTARROWS | DLGC_HASSETSEL | DLGC_WANTCHARS
const DLGC_EDIT_WANT_MESSAGE: isize = 0x04 | 0x01 | 0x08 | 0x80;
const WM_KEYDOWN: u32 = 0x0100;
const WM_CHAR: u32 = 0x0102;
const VK_RETURN: usize = 0x0D;
const VK_UP: usize = 0x26;
const VK_DOWN: usize = 0x28;
const HISTORY_MAX: usize = 50;
const COLUMNS: [(&str, i32); 7] = [
    ("이름", 140),
    ("상태", 75),
    ("PID", 55),
    ("CPU", 55),
    ("메모리", 75),
    ("가동 시간", 80),
    ("재시작", 50),
];

pub struct AppOptions {
    pub tray_mode: bool,
    pub resume_ids: Vec<String>,
    pub activate_message: u32,
    pub instance: Option<win::SingleInstance>,
}

#[derive(Default)]
struct Ui {
    window: nwg::Window,
    icons: [nwg::Icon; 4],
    console_font: nwg::Font,
    list: nwg::ListView,
    console: nwg::RichTextBox,
    input: nwg::TextInput,
    btn_start: nwg::Button,
    btn_stop: nwg::Button,
    btn_restart: nwg::Button,
    btn_window: nwg::Button,
    menu_file: nwg::Menu,
    mi_add: nwg::MenuItem,
    mi_edit: nwg::MenuItem,
    mi_delete: nwg::MenuItem,
    mi_open_config: nwg::MenuItem,
    mi_reload: nwg::MenuItem,
    mi_autostart: nwg::MenuItem,
    mi_exit: nwg::MenuItem,
    menu_help: nwg::Menu,
    mi_check_update: nwg::MenuItem,
    mi_install_update: nwg::MenuItem,
    mi_logs: nwg::MenuItem,
    mi_about: nwg::MenuItem,
    separators: Vec<nwg::MenuSeparator>,
    tray: nwg::TrayNotification,
    tray_menu: nwg::Menu,
    ti_open: nwg::MenuItem,
    ti_start_all: nwg::MenuItem,
    ti_stop_all: nwg::MenuItem,
    ti_check_update: nwg::MenuItem,
    list_menu: nwg::Menu,
    li_start: nwg::MenuItem,
    li_stop: nwg::MenuItem,
    li_restart: nwg::MenuItem,
    li_window: nwg::MenuItem,
    li_edit: nwg::MenuItem,
    li_delete: nwg::MenuItem,
    li_logs: nwg::MenuItem,
    li_add: nwg::MenuItem,
    ti_exit: nwg::MenuItem,
    notice: nwg::Notice,
}

#[derive(Clone, Copy)]
enum Action {
    Start,
    Stop,
    Restart,
    ToggleWindow,
}

fn window_label(visible: bool) -> &'static str {
    if visible {
        "창 숨기기"
    } else {
        "창 보이기"
    }
}

#[derive(Default)]
struct UiState {
    selected: Option<String>,
    rows: Vec<[String; 7]>,
    history: Vec<String>,
    history_pos: Option<usize>,
    hide_hint_shown: bool,
    tray_color: Option<TrayColor>,
    tray_tip: String,
    quitting: bool,
    dialog_open: bool,
    release: Option<Release>,
    update_busy: bool,
}

pub struct App {
    ui: Ui,
    opts: AppOptions,
    manager: RefCell<Manager>,
    events: Receiver<Event>,
    config: RefCell<Config>,
    state: RefCell<UiState>,
    console: RefCell<ConsoleView>,
    rebuilding: Cell<bool>,
    quit_ready: Arc<AtomicBool>,
    updates_tx: crossbeam_channel::Sender<UpdateMsg>,
    updates_rx: Receiver<UpdateMsg>,
    instance: RefCell<Option<win::SingleInstance>>,
    ticker_stop: Arc<AtomicBool>,
    handlers: RefCell<Vec<nwg::RawEventHandler>>,
    event_handler: RefCell<Option<nwg::EventHandler>>,
}

fn icon(bytes: &[u8], out: &mut nwg::Icon) -> Result<(), nwg::NwgError> {
    nwg::Icon::builder().source_bin(Some(bytes)).build(out)
}

fn item(text: &str, parent: &nwg::Menu, out: &mut nwg::MenuItem) -> Result<(), nwg::NwgError> {
    nwg::MenuItem::builder()
        .text(text)
        .parent(parent)
        .build(out)
}

fn separator(parent: &nwg::Menu, list: &mut Vec<nwg::MenuSeparator>) -> Result<(), nwg::NwgError> {
    let mut sep = nwg::MenuSeparator::default();
    nwg::MenuSeparator::builder()
        .parent(parent)
        .build(&mut sep)?;
    list.push(sep);
    Ok(())
}

fn button(
    text: &str,
    parent: nwg::ControlHandle,
    out: &mut nwg::Button,
) -> Result<(), nwg::NwgError> {
    nwg::Button::builder()
        .text(text)
        .size((90, 28))
        .parent(parent)
        .build(out)
}

fn build_ui(ui: &mut Ui) -> Result<(), nwg::NwgError> {
    icon(include_bytes!("../res/icons/gray.ico"), &mut ui.icons[0])?;
    icon(include_bytes!("../res/icons/green.ico"), &mut ui.icons[1])?;
    icon(include_bytes!("../res/icons/yellow.ico"), &mut ui.icons[2])?;
    icon(include_bytes!("../res/icons/red.ico"), &mut ui.icons[3])?;

    nwg::Window::builder()
        .title(APP_NAME)
        .size((1180, 680))
        .center(true)
        .flags(nwg::WindowFlags::MAIN_WINDOW)
        .icon(Some(&ui.icons[1]))
        .build(&mut ui.window)?;
    let w = ui.window.handle;

    nwg::Font::builder()
        .family("Consolas")
        .size(16)
        .build(&mut ui.console_font)?;

    nwg::ListView::builder()
        .list_style(nwg::ListViewStyle::Detailed)
        .ex_flags(nwg::ListViewExFlags::GRID | nwg::ListViewExFlags::FULL_ROW_SELECT)
        .flags(
            nwg::ListViewFlags::VISIBLE
                | nwg::ListViewFlags::SINGLE_SELECTION
                | nwg::ListViewFlags::TAB_STOP,
        )
        .double_buffer(true)
        .parent(w)
        .build(&mut ui.list)?;
    for (i, (text, width)) in COLUMNS.iter().enumerate() {
        ui.list.insert_column(nwg::InsertListViewColumn {
            index: Some(i as i32),
            fmt: None,
            width: Some(*width),
            text: Some((*text).into()),
        });
    }
    ui.list.set_headers_enabled(true);

    nwg::RichTextBox::builder()
        .readonly(true)
        .font(Some(&ui.console_font))
        .flags(
            nwg::RichTextBoxFlags::VISIBLE
                | nwg::RichTextBoxFlags::VSCROLL
                | nwg::RichTextBoxFlags::AUTOVSCROLL
                | nwg::RichTextBoxFlags::SAVE_SELECTION
                | nwg::RichTextBoxFlags::TAB_STOP,
        )
        .parent(w)
        .build(&mut ui.console)?;
    ui.console.set_limit(0x7FFF_FFFE);

    nwg::TextInput::builder()
        .font(Some(&ui.console_font))
        .placeholder_text(Some("명령 입력 후 Enter (위/아래: 이전 입력)"))
        .parent(w)
        .build(&mut ui.input)?;

    button("시작", w, &mut ui.btn_start)?;
    button("중지", w, &mut ui.btn_stop)?;
    button("재시작", w, &mut ui.btn_restart)?;
    button("창 보이기", w, &mut ui.btn_window)?;

    nwg::Menu::builder()
        .text("파일")
        .parent(w)
        .build(&mut ui.menu_file)?;
    item("서비스 추가...", &ui.menu_file, &mut ui.mi_add)?;
    item("서비스 편집...", &ui.menu_file, &mut ui.mi_edit)?;
    item("서비스 삭제", &ui.menu_file, &mut ui.mi_delete)?;
    separator(&ui.menu_file, &mut ui.separators)?;
    item("설정 파일 열기", &ui.menu_file, &mut ui.mi_open_config)?;
    item("설정 다시 불러오기", &ui.menu_file, &mut ui.mi_reload)?;
    separator(&ui.menu_file, &mut ui.separators)?;
    item("Windows 시작 시 실행", &ui.menu_file, &mut ui.mi_autostart)?;
    separator(&ui.menu_file, &mut ui.separators)?;
    item("종료", &ui.menu_file, &mut ui.mi_exit)?;

    nwg::Menu::builder()
        .text("도움말")
        .parent(w)
        .build(&mut ui.menu_help)?;
    item("업데이트 확인", &ui.menu_help, &mut ui.mi_check_update)?;
    nwg::MenuItem::builder()
        .text("업데이트 설치")
        .disabled(true)
        .parent(&ui.menu_help)
        .build(&mut ui.mi_install_update)?;
    separator(&ui.menu_help, &mut ui.separators)?;
    item("로그 폴더 열기", &ui.menu_help, &mut ui.mi_logs)?;
    item("정보", &ui.menu_help, &mut ui.mi_about)?;

    nwg::TrayNotification::builder()
        .parent(w)
        .icon(Some(&ui.icons[0]))
        .tip(Some(APP_NAME))
        .build(&mut ui.tray)?;
    nwg::Menu::builder()
        .popup(true)
        .parent(w)
        .build(&mut ui.tray_menu)?;
    item("열기", &ui.tray_menu, &mut ui.ti_open)?;
    item("모두 시작", &ui.tray_menu, &mut ui.ti_start_all)?;
    item("모두 중지", &ui.tray_menu, &mut ui.ti_stop_all)?;
    separator(&ui.tray_menu, &mut ui.separators)?;
    item("업데이트 확인", &ui.tray_menu, &mut ui.ti_check_update)?;
    separator(&ui.tray_menu, &mut ui.separators)?;
    item("종료", &ui.tray_menu, &mut ui.ti_exit)?;

    nwg::Menu::builder()
        .popup(true)
        .parent(w)
        .build(&mut ui.list_menu)?;
    item("시작", &ui.list_menu, &mut ui.li_start)?;
    item("중지", &ui.list_menu, &mut ui.li_stop)?;
    item("재시작", &ui.list_menu, &mut ui.li_restart)?;
    item("창 보이기", &ui.list_menu, &mut ui.li_window)?;
    separator(&ui.list_menu, &mut ui.separators)?;
    item("편집...", &ui.list_menu, &mut ui.li_edit)?;
    item("삭제", &ui.list_menu, &mut ui.li_delete)?;
    item("로그 폴더 열기", &ui.list_menu, &mut ui.li_logs)?;
    separator(&ui.list_menu, &mut ui.separators)?;
    item("서비스 추가...", &ui.list_menu, &mut ui.li_add)?;

    nwg::Notice::builder().parent(w).build(&mut ui.notice)?;
    Ok(())
}

impl App {
    pub fn build(mut opts: AppOptions) -> Result<Rc<App>, nwg::NwgError> {
        let instance = opts.instance.take();
        let (updates_tx, updates_rx) = crossbeam_channel::unbounded();
        let mut ui = Ui::default();
        build_ui(&mut ui)?;

        let config_path = paths::config_file();
        let config = Config::load_or_create(&config_path);
        zsm_core::logfile::purge_old_logs(&paths::logs_dir(), config.app.log_retention_days);

        let sender = ui.notice.sender();
        let exe = std::env::current_exe().unwrap_or_default();
        let (mut manager, events) = Manager::new(ManagerOptions {
            log_root: paths::logs_dir(),
            notify: Some(Arc::new(move || sender.notice())),
            ctrl_break_helper: Some(CtrlBreakHelper {
                program: exe,
                args: vec!["--ctrl-break".into()],
            }),
        });
        manager.apply(&config);

        let console = ConsoleView::new(
            RichEdit(WindowId::from_raw(
                ui.console.handle.hwnd().unwrap() as isize
            )),
            config.app.scrollback_lines,
        );
        let app = Rc::new(App {
            ui,
            opts,
            manager: RefCell::new(manager),
            events,
            config: RefCell::new(config),
            state: RefCell::new(UiState::default()),
            console: RefCell::new(console),
            rebuilding: Cell::new(false),
            quit_ready: Arc::new(AtomicBool::new(false)),
            updates_tx,
            updates_rx,
            instance: RefCell::new(instance),
            ticker_stop: Arc::new(AtomicBool::new(false)),
            handlers: RefCell::new(Vec::new()),
            event_handler: RefCell::new(None),
        });
        app.bind_events();
        app.start_ticker();
        let app_settings = app.config.borrow().app.clone();
        if app_settings.check_updates {
            updater::schedule(
                app.updates_tx.clone(),
                app.ui.notice.sender(),
                app_settings.update_check_interval_hours,
            );
        }
        app.ui
            .mi_autostart
            .set_checked(win::run_entry(RUN_VALUE).is_some());
        app.layout();
        app.rebuild_rows();
        app.after_start();
        Ok(app)
    }

    fn after_start(&self) {
        {
            let mgr = self.manager.borrow();
            mgr.start_autostart();
            for id in &self.opts.resume_ids {
                if let Some(svc) = mgr.get(id) {
                    svc.start();
                }
            }
        }
        let start_hidden = self.opts.tray_mode || self.config.borrow().app.start_minimized;
        if !start_hidden {
            self.show_window();
        }
        if let Some(err) = self.config.borrow().error.clone() {
            self.balloon("설정 오류", &err, nwg::TrayNotificationFlags::ERROR_ICON);
        }
        self.refresh();
    }

    fn bind_events(self: &Rc<Self>) {
        let weak = Rc::downgrade(self);
        let handler =
            nwg::full_bind_event_handler(&self.ui.window.handle, move |evt, data, handle| {
                if let Some(app) = weak.upgrade() {
                    app.on_event(evt, &data, handle);
                }
            });
        *self.event_handler.borrow_mut() = Some(handler);

        let mut handlers = self.handlers.borrow_mut();
        let weak = Rc::downgrade(self);
        if let Ok(h) =
            nwg::bind_raw_event_handler(&self.ui.window.handle, 0x10000, move |_h, msg, w, _l| {
                let app = weak.upgrade()?;
                guarded(|| app.on_window_message(msg, w)).flatten()
            })
        {
            handlers.push(h);
        }
        let weak = Rc::downgrade(self);
        if let Ok(h) =
            nwg::bind_raw_event_handler(&self.ui.console.handle, 0x10001, move |_h, msg, w, _l| {
                let app = weak.upgrade()?;
                guarded(|| app.on_ctrl_c_key(msg, w)).flatten()
            })
        {
            handlers.push(h);
        }
        let weak: Weak<App> = Rc::downgrade(self);
        if let Ok(h) =
            nwg::bind_raw_event_handler(&self.ui.input.handle, 0x10002, move |_h, msg, w, l| {
                let app = weak.upgrade()?;
                guarded(|| {
                    app.on_ctrl_c_key(msg, w)
                        .or_else(|| app.on_input_key(msg, w, l))
                })
                .flatten()
            })
        {
            handlers.push(h);
        }
    }

    fn start_ticker(&self) {
        let sender = self.ui.notice.sender();
        let stop = self.ticker_stop.clone();
        std::thread::Builder::new()
            .name("ui-ticker".into())
            .spawn(move || {
                while !stop.load(Ordering::Relaxed) {
                    std::thread::sleep(UI_TICK);
                    sender.notice();
                }
            })
            .expect("spawn ui ticker");
    }

    fn on_event(&self, evt: nwg::Event, data: &nwg::EventData, handle: nwg::ControlHandle) {
        let ui = &self.ui;
        match evt {
            nwg::Event::OnNotice => self.refresh(),
            nwg::Event::OnResize | nwg::Event::OnWindowMaximize => self.layout(),
            nwg::Event::OnMinMaxInfo => {
                if let nwg::EventData::OnMinMaxInfo(info) = data {
                    info.set_min_size(820, 460);
                }
            }
            nwg::Event::OnWindowClose if handle == ui.window.handle => {
                if let nwg::EventData::OnWindowClose(close) = data {
                    close.close(false);
                }
                self.hide_window();
            }
            nwg::Event::OnMousePress(nwg::MousePressEvent::MousePressLeftUp)
                if handle == ui.tray.handle =>
            {
                if ui.window.visible() {
                    self.hide_window();
                } else {
                    self.show_window();
                }
            }
            nwg::Event::OnContextMenu if handle == ui.tray.handle => {
                let (x, y) = win::cursor_pos();
                ui.tray_menu.popup(x, y);
            }
            nwg::Event::OnButtonClick => self.on_button(handle),
            nwg::Event::OnMenuItemSelected => self.on_menu(handle),
            nwg::Event::OnListViewItemChanged | nwg::Event::OnListViewClick => {
                self.sync_selection()
            }
            nwg::Event::OnListViewRightClick if handle == ui.list.handle => self.show_list_menu(),
            _ => {}
        }
    }

    fn on_button(&self, handle: nwg::ControlHandle) {
        let ui = &self.ui;
        let action = if handle == ui.btn_start.handle {
            Action::Start
        } else if handle == ui.btn_stop.handle {
            Action::Stop
        } else if handle == ui.btn_restart.handle {
            Action::Restart
        } else if handle == ui.btn_window.handle {
            Action::ToggleWindow
        } else {
            return;
        };
        self.run_action(action);
    }

    fn run_action(&self, action: Action) {
        let Some(id) = self.state.borrow().selected.clone() else {
            return;
        };
        let mgr = self.manager.borrow();
        let Some(svc) = mgr.get(&id) else { return };
        match action {
            Action::Start => svc.start(),
            Action::Stop => svc.stop(StopMode::Normal),
            Action::Restart => svc.restart(),
            Action::ToggleWindow => svc.set_window_visible(!svc.status().window_visible),
        }
    }

    /// Right-click on the service list: select the row under the cursor and show its actions.
    fn show_list_menu(&self) {
        self.sync_selection();
        let ui = &self.ui;
        let selected = self.state.borrow().selected.clone();
        let (state, gui, visible) = {
            let mgr = self.manager.borrow();
            match selected.as_deref().and_then(|id| mgr.get(id)) {
                Some(s) => {
                    let st = s.status();
                    (
                        Some(st.state),
                        s.config().kind == Kind::Gui,
                        st.window_visible,
                    )
                }
                None => (None, false, false),
            }
        };
        let can = allowed(state, gui);
        let has = selected.is_some();
        ui.li_start.set_enabled(can.start);
        ui.li_stop.set_enabled(can.stop);
        ui.li_restart.set_enabled(can.restart);
        ui.li_window.set_enabled(can.window);
        if let Some((menu, id)) = ui.li_window.handle.hmenu_item() {
            win::set_menu_item_text(menu as isize, id, window_label(visible));
        }
        ui.li_edit.set_enabled(has);
        ui.li_delete.set_enabled(has);
        ui.li_logs.set_enabled(has);
        let (x, y) = win::cursor_pos();
        ui.list_menu.popup(x, y);
    }

    fn on_menu(&self, handle: nwg::ControlHandle) {
        let ui = &self.ui;
        let list_action = [
            (&ui.li_start, Action::Start),
            (&ui.li_stop, Action::Stop),
            (&ui.li_restart, Action::Restart),
            (&ui.li_window, Action::ToggleWindow),
        ]
        .into_iter()
        .find(|(item, _)| item.handle == handle);
        if let Some((_, action)) = list_action {
            self.run_action(action);
        } else if handle == ui.li_edit.handle {
            self.edit_selected();
        } else if handle == ui.li_delete.handle {
            self.delete_selected();
        } else if handle == ui.li_add.handle {
            self.add_service();
        } else if handle == ui.li_logs.handle {
            self.open_service_logs();
        } else if handle == ui.ti_open.handle {
            self.show_window();
        } else if handle == ui.ti_start_all.handle {
            self.manager.borrow().start_all();
        } else if handle == ui.mi_check_update.handle || handle == ui.ti_check_update.handle {
            updater::check_now(self.updates_tx.clone(), ui.notice.sender(), true);
        } else if handle == ui.mi_install_update.handle {
            self.install_update();
        } else if handle == ui.ti_stop_all.handle {
            self.manager.borrow().stop_all(StopMode::Normal);
        } else if handle == ui.ti_exit.handle || handle == ui.mi_exit.handle {
            self.request_quit();
        } else if handle == ui.mi_add.handle {
            self.add_service();
        } else if handle == ui.mi_edit.handle {
            self.edit_selected();
        } else if handle == ui.mi_delete.handle {
            self.delete_selected();
        } else if handle == ui.mi_open_config.handle {
            open_in_editor(&paths::config_file().to_string_lossy());
        } else if handle == ui.mi_reload.handle {
            self.reload_config(true);
        } else if handle == ui.mi_autostart.handle {
            self.toggle_autostart();
        } else if handle == ui.mi_logs.handle {
            let dir = paths::logs_dir();
            let _ = std::fs::create_dir_all(&dir);
            let _ = win::shell_open(&dir.to_string_lossy());
        } else if handle == ui.mi_about.handle {
            nwg::modal_info_message(
                &ui.window,
                APP_NAME,
                &format!(
                    "{APP_NAME} v{VERSION}\n\n설정: {}\n로그: {}",
                    paths::config_file().display(),
                    paths::logs_dir().display()
                ),
            );
        }
    }

    fn on_window_message(&self, msg: u32, w: usize) -> Option<isize> {
        if msg == self.opts.activate_message {
            self.show_window();
            return Some(0);
        }
        match msg {
            WM_QUERYENDSESSION => {
                let hwnd = self.window_id();
                let _ = win::block_shutdown(hwnd, "서비스를 안전하게 중지하는 중");
                log::info!("세션 종료 요청, 모든 서비스 중지");
                self.manager.borrow().stop_all(StopMode::Shutdown);
                Some(1)
            }
            WM_ENDSESSION => {
                if w != 0 {
                    let stopped = self
                        .manager
                        .borrow()
                        .activity()
                        .wait_inactive(SESSION_END_WAIT);
                    log::info!("세션 종료 처리 완료 (모두 중지: {stopped})");
                }
                win::unblock_shutdown(self.window_id());
                Some(0)
            }
            _ => None,
        }
    }

    fn on_ctrl_c_key(&self, msg: u32, w: usize) -> Option<isize> {
        let is_ctrl_c = (msg == WM_KEYDOWN && w == 'C' as usize && win::ctrl_down())
            || (msg == WM_CHAR && w == 3);
        if !is_ctrl_c {
            return None;
        }
        if msg == WM_KEYDOWN {
            self.on_ctrl_c();
        }
        Some(0)
    }

    fn on_ctrl_c(&self) {
        let edit = self.console.borrow().edit();
        if edit.has_selection() {
            log::info!("Ctrl+C: 콘솔 선택 영역 복사");
            edit.copy();
            return;
        }
        log::info!("Ctrl+C: 선택 없음, 중지 확인");
        let Some(id) = self.state.borrow().selected.clone() else {
            return;
        };
        let (name, active) = {
            let mgr = self.manager.borrow();
            let Some(svc) = mgr.get(&id) else { return };
            (
                svc.config().display_name().to_owned(),
                svc.status().state.is_active(),
            )
        };
        if !active {
            return;
        }
        let choice = nwg::modal_message(
            &self.ui.window,
            &nwg::MessageParams {
                title: APP_NAME,
                content: &format!("\"{name}\" 서비스를 중지할까?"),
                buttons: nwg::MessageButtons::YesNo,
                icons: nwg::MessageIcons::Question,
            },
        );
        log::info!("Ctrl+C 확인 결과 yes={}", choice == nwg::MessageChoice::Yes);
        if choice == nwg::MessageChoice::Yes {
            if let Some(svc) = self.manager.borrow().get(&id) {
                svc.stop(StopMode::Normal);
            }
        }
    }

    fn on_input_key(&self, msg: u32, w: usize, l: isize) -> Option<isize> {
        match (msg, w) {
            // The NWG message loop runs IsDialogMessage, which eats Enter and arrows unless
            // the control claims them.
            (WM_GETDLGCODE, _) => match win::dialog_code_message(l) {
                Some((WM_KEYDOWN, VK_RETURN | VK_UP | VK_DOWN)) => Some(DLGC_EDIT_WANT_MESSAGE),
                _ => None,
            },
            (WM_KEYDOWN, VK_RETURN) => {
                self.submit_input();
                Some(0)
            }
            (WM_CHAR, 0x0D) => Some(0),
            (WM_KEYDOWN, VK_UP) | (WM_KEYDOWN, VK_DOWN) => {
                self.recall_history(w == VK_UP);
                Some(0)
            }
            _ => None,
        }
    }

    fn submit_input(&self) {
        let text = self.ui.input.text();
        let Some(id) = self.state.borrow().selected.clone() else {
            return;
        };
        if let Some(svc) = self.manager.borrow().get(&id) {
            svc.send_line(&text);
        }
        self.ui.input.set_text("");
        let mut st = self.state.borrow_mut();
        st.history_pos = None;
        if !text.trim().is_empty() && st.history.last() != Some(&text) {
            st.history.push(text);
            if st.history.len() > HISTORY_MAX {
                st.history.remove(0);
            }
        }
    }

    fn recall_history(&self, older: bool) {
        let mut st = self.state.borrow_mut();
        if st.history.is_empty() {
            return;
        }
        let last = st.history.len() - 1;
        let pos = match (st.history_pos, older) {
            (None, true) => Some(last),
            (None, false) => None,
            (Some(0), true) => Some(0),
            (Some(p), true) => Some(p - 1),
            (Some(p), false) if p >= last => None,
            (Some(p), false) => Some(p + 1),
        };
        st.history_pos = pos;
        let text = pos.map(|p| st.history[p].clone()).unwrap_or_default();
        drop(st);
        self.ui.input.set_text(&text);
        let len = text.encode_utf16().count() as u32;
        self.ui.input.set_selection(len..len);
    }

    fn window_id(&self) -> WindowId {
        WindowId::from_raw(self.ui.window.handle.hwnd().map_or(0, |h| h as isize))
    }

    fn show_window(&self) {
        self.ui.window.set_visible(true);
        win::bring_to_front(self.window_id());
        self.refresh();
    }

    fn hide_window(&self) {
        self.ui.window.set_visible(false);
        let first = !std::mem::replace(&mut self.state.borrow_mut().hide_hint_shown, true);
        if first {
            self.balloon(
                APP_NAME,
                "창을 닫아도 트레이에서 계속 실행 중이야. 트레이 아이콘을 클릭하면 다시 열려.",
                nwg::TrayNotificationFlags::INFO_ICON,
            );
        }
    }

    fn balloon(&self, title: &str, text: &str, flags: nwg::TrayNotificationFlags) {
        self.ui.tray.show(
            text,
            Some(title),
            Some(flags | nwg::TrayNotificationFlags::QUIET),
            None,
        );
    }

    fn layout(&self) {
        let ui = &self.ui;
        let (w, h) = ui.window.size();
        let (w, h) = (w as i32, h as i32);
        let m = 8;
        let list_w = (w * 45 / 100).clamp(560, 700).min(w - 300);
        ui.list.set_position(m, m);
        ui.list
            .set_size(list_w.max(100) as u32, (h - 2 * m).max(50) as u32);

        let x = m + list_w + m;
        let right_w = (w - x - m).max(100);
        let buttons = [&ui.btn_start, &ui.btn_stop, &ui.btn_restart, &ui.btn_window];
        for (i, b) in buttons.iter().enumerate() {
            b.set_position(x + i as i32 * 96, m);
        }
        let input_h = 26;
        let console_y = m + 28 + m;
        let console_h = (h - console_y - input_h - 2 * m).max(50);
        ui.console.set_position(x, console_y);
        ui.console.set_size(right_w as u32, console_h as u32);
        ui.input.set_position(x, console_y + console_h + m);
        ui.input.set_size(right_w as u32, input_h as u32);
    }

    fn rebuild_rows(&self) {
        let ui = &self.ui;
        // ListView calls fire selection-change events synchronously; ignore them until done.
        self.rebuilding.set(true);
        let (names, index) = {
            let mgr = self.manager.borrow();
            let names: Vec<String> = mgr
                .services()
                .iter()
                .map(|s| s.config().display_name().to_owned())
                .collect();
            let selected = self.state.borrow().selected.clone();
            let index = selected
                .and_then(|id| mgr.services().iter().position(|s| s.id() == id))
                .or((!names.is_empty()).then_some(0));
            (names, index)
        };
        self.state.borrow_mut().rows = vec![Default::default(); names.len()];
        while ui.list.len() > names.len() {
            ui.list.remove_item(ui.list.len() - 1);
        }
        for (i, name) in names.into_iter().enumerate() {
            let item = nwg::InsertListViewItem {
                index: Some(i as i32),
                column_index: 0,
                text: Some(name),
            };
            if i < ui.list.len() {
                ui.list.update_item(i, item);
            } else {
                ui.list.insert_item(item);
            }
        }
        if let Some(i) = index {
            ui.list.select_item(i, true);
        }
        self.rebuilding.set(false);
        self.sync_selection();
    }

    fn sync_selection(&self) {
        if self.rebuilding.get() {
            return;
        }
        let selected = {
            let mgr = self.manager.borrow();
            self.ui
                .list
                .selected_item()
                .and_then(|i| mgr.services().get(i))
                .map(|s| s.id().to_owned())
        };
        if selected != self.state.borrow().selected {
            self.state.borrow_mut().selected = selected.clone();
            self.ui.input.set_text("");
        }
        let mgr = self.manager.borrow();
        let svc = selected.as_deref().and_then(|id| mgr.get(id));
        let console = &self.ui.console;
        self.console.borrow_mut().show(svc, || console.clear());
        drop(mgr);
        self.update_buttons();
    }

    fn update_buttons(&self) {
        let ui = &self.ui;
        let selected = self.state.borrow().selected.clone();
        let mgr = self.manager.borrow();
        let svc = selected.as_deref().and_then(|id| mgr.get(id));
        let (state, kind, visible) = match svc {
            Some(s) => {
                let st = s.status();
                (Some(st.state), s.config().kind, st.window_visible)
            }
            None => (None, Kind::Console, false),
        };
        let set = |b: &nwg::Button, on: bool| {
            if b.enabled() != on {
                b.set_enabled(on);
            }
        };
        let can = allowed(state, kind == Kind::Gui);
        set(&ui.btn_start, can.start);
        set(&ui.btn_stop, can.stop);
        set(&ui.btn_restart, can.restart);
        set(&ui.btn_window, can.window);
        let label = window_label(visible);
        if ui.btn_window.text() != label {
            ui.btn_window.set_text(label);
        }
        if ui.input.enabled() != can.input {
            ui.input.set_enabled(can.input);
        }
        let has = svc.is_some();
        ui.mi_edit.set_enabled(has);
        ui.mi_delete.set_enabled(has);
    }

    fn refresh(&self) {
        self.drain_events();
        self.drain_updates();
        self.refresh_rows();
        self.refresh_tray();
        if self.ui.window.visible() {
            self.update_buttons();
            self.pull_console();
        }
        let ready = self.state.borrow().quitting && self.quit_ready.load(Ordering::SeqCst);
        if ready {
            self.ticker_stop.store(true, Ordering::Relaxed);
            nwg::stop_thread_dispatch();
        }
    }

    fn drain_events(&self) {
        for evt in self.events.try_iter() {
            log::info!("{} -> {:?} (exit {:?})", evt.id, evt.state, evt.exit_code);
            match evt.state {
                State::Crashed => {
                    let code = evt
                        .exit_code
                        .map_or("시작 실패".into(), |c| format!("종료 코드 {c}"));
                    self.balloon(
                        "서비스 크래시",
                        &format!("{} ({code})", evt.name),
                        nwg::TrayNotificationFlags::WARNING_ICON,
                    );
                }
                State::Failed => {
                    self.balloon(
                        "서비스 실패",
                        &format!("{}: 재시작 한도를 넘어 중단했어", evt.name),
                        nwg::TrayNotificationFlags::ERROR_ICON,
                    );
                }
                _ => {}
            }
        }
    }

    fn refresh_rows(&self) {
        let mgr = self.manager.borrow();
        if mgr.services().len() != self.state.borrow().rows.len() {
            drop(mgr);
            self.rebuild_rows();
            return;
        }
        let rows: Vec<[String; 7]> = mgr.services().iter().map(row_texts).collect();
        drop(mgr);
        let changes: Vec<(usize, usize, String)> = {
            let mut st = self.state.borrow_mut();
            let mut changes = Vec::new();
            for (i, row) in rows.into_iter().enumerate() {
                for (col, text) in row.iter().enumerate() {
                    if st.rows[i][col] != *text {
                        changes.push((i, col, text.clone()));
                    }
                }
                st.rows[i] = row;
            }
            changes
        };
        for (row, col, text) in changes {
            self.ui.list.update_item(
                row,
                nwg::InsertListViewItem {
                    index: Some(row as i32),
                    column_index: col as i32,
                    text: Some(text),
                },
            );
        }
    }

    fn refresh_tray(&self) {
        let mgr = self.manager.borrow();
        let states: Vec<(State, bool)> = mgr
            .services()
            .iter()
            .map(|s| (s.status().state, s.config().autostart))
            .collect();
        drop(mgr);
        let summary = summarize(&states);
        let mut st = self.state.borrow_mut();
        if st.tray_color != Some(summary.color) {
            st.tray_color = Some(summary.color);
            log::info!("트레이 아이콘: {:?}", summary.color);
            let idx = match summary.color {
                TrayColor::Gray => 0,
                TrayColor::Green => 1,
                TrayColor::Yellow => 2,
                TrayColor::Red => 3,
            };
            self.ui.tray.set_icon(&self.ui.icons[idx]);
        }
        let tip = if st.quitting {
            format!("{APP_NAME} — 서비스 중지 중...")
        } else {
            format!(
                "{APP_NAME} — 실행 {} / 전체 {}",
                summary.running, summary.total
            )
        };
        if st.tray_tip != tip {
            self.ui.tray.set_tip(&tip);
            st.tray_tip = tip;
        }
    }

    fn pull_console(&self) {
        let Some(id) = self.state.borrow().selected.clone() else {
            return;
        };
        let mgr = self.manager.borrow();
        let Some(svc) = mgr.get(&id) else { return };
        let console = &self.ui.console;
        self.console.borrow_mut().pull(svc, || console.clear());
    }

    fn drain_updates(&self) {
        let msgs: Vec<UpdateMsg> = self.updates_rx.try_iter().collect();
        for msg in msgs {
            match msg {
                UpdateMsg::Checked { manual, result } => self.on_update_checked(manual, result),
                UpdateMsg::Downloaded(Ok(installer)) => self.stop_for_update(installer),
                UpdateMsg::Downloaded(Err(e)) => {
                    log::error!("업데이트 다운로드 실패: {e}");
                    self.state.borrow_mut().update_busy = false;
                    self.balloon("업데이트 중단", &e, nwg::TrayNotificationFlags::ERROR_ICON);
                }
                UpdateMsg::ServicesStopped(installer) => self.run_installer(&installer),
            }
        }
    }

    fn on_update_checked(&self, manual: bool, result: Result<Option<Release>, String>) {
        match result {
            Ok(Some(release)) => {
                let known = self
                    .state
                    .borrow()
                    .release
                    .as_ref()
                    .map(|r| r.version.clone());
                let label = format!("업데이트 설치 (v{})", release.version);
                if let Some((menu, id)) = self.ui.mi_install_update.handle.hmenu_item() {
                    win::set_menu_item_text(menu as isize, id, &label);
                }
                self.ui.mi_install_update.set_enabled(true);
                if known.as_ref() != Some(&release.version) {
                    log::info!("새 버전 발견: v{}", release.version);
                    self.balloon(
                        "업데이트 있음",
                        &format!(
                            "v{} 를 설치할 수 있어. 도움말 메뉴에서 설치해줘.",
                            release.version
                        ),
                        nwg::TrayNotificationFlags::INFO_ICON,
                    );
                }
                self.state.borrow_mut().release = Some(release);
                if manual {
                    self.install_update();
                }
            }
            Ok(None) if manual => {
                nwg::modal_info_message(
                    &self.ui.window,
                    APP_NAME,
                    &format!("최신 버전이야 (v{VERSION})."),
                );
            }
            Ok(None) => {}
            Err(e) if manual => {
                nwg::modal_error_message(
                    &self.ui.window,
                    APP_NAME,
                    &format!("업데이트 확인 실패: {e}"),
                );
            }
            Err(_) => {}
        }
    }

    fn install_update(&self) {
        let Some(release) = self.state.borrow().release.clone() else {
            return;
        };
        if self.state.borrow().update_busy {
            return;
        }
        if !update::is_installed() {
            nwg::modal_info_message(
                &self.ui.window,
                APP_NAME,
                &format!(
                    "포터블 실행이라 자동 설치는 하지 않아. v{} 릴리스 페이지를 열게.",
                    release.version
                ),
            );
            let page = if release.page_url.is_empty() {
                update::RELEASES_PAGE
            } else {
                &release.page_url
            };
            let _ = win::shell_open(page);
            return;
        }
        let active = self.manager.borrow().active_ids().len();
        let choice = nwg::modal_message(
            &self.ui.window,
            &nwg::MessageParams {
                title: APP_NAME,
                content: &format!(
                    "v{} 로 업데이트할까?\n\n실행 중인 서비스 {active}개가 정상 중지되고, 설치가 끝나면 다시 시작돼.",
                    release.version
                ),
                buttons: nwg::MessageButtons::YesNo,
                icons: nwg::MessageIcons::Question,
            },
        );
        if choice != nwg::MessageChoice::Yes {
            return;
        }
        self.state.borrow_mut().update_busy = true;
        log::info!("업데이트 v{} 다운로드 시작", release.version);
        updater::download(self.updates_tx.clone(), self.ui.notice.sender(), release);
    }

    fn stop_for_update(&self, installer: std::path::PathBuf) {
        let mgr = self.manager.borrow();
        let active = mgr.active_ids();
        if let Err(e) = update::write_resume(&paths::resume_file(), &active) {
            log::error!("resume.json 기록 실패: {e}");
        }
        log::info!("업데이트 설치 준비: 서비스 {active:?} 중지");
        self.state.borrow_mut().quitting = true;
        mgr.stop_all(StopMode::Normal);
        updater::wait_then_install(
            self.updates_tx.clone(),
            self.ui.notice.sender(),
            mgr.activity(),
            installer,
        );
    }

    fn run_installer(&self, installer: &std::path::Path) {
        // Release the single-instance mutex so the installer's AppMutex check passes.
        drop(self.instance.borrow_mut().take());
        let spawned = {
            let _guard = win::spawn_lock();
            std::process::Command::new(installer)
                .args(update::INSTALLER_ARGS)
                .spawn()
        };
        match spawned {
            Ok(_) => {
                log::info!("설치 프로그램 실행: {}", installer.display());
                self.quit_ready.store(true, Ordering::SeqCst);
            }
            Err(e) => {
                log::error!("설치 프로그램 실행 실패: {e}");
                let _ = std::fs::remove_file(paths::resume_file());
                self.state.borrow_mut().quitting = false;
                self.state.borrow_mut().update_busy = false;
                nwg::modal_error_message(
                    &self.ui.window,
                    APP_NAME,
                    &format!("설치 프로그램 실행 실패: {e}"),
                );
            }
        }
    }

    fn request_quit(&self) {
        if self.state.borrow().dialog_open {
            nwg::modal_info_message(
                &self.ui.window,
                APP_NAME,
                "열려 있는 대화상자를 먼저 닫아줘.",
            );
            return;
        }
        if self.state.borrow().quitting {
            return;
        }
        let active = self.manager.borrow().active_ids();
        if !active.is_empty() {
            let choice = nwg::modal_message(
                &self.ui.window,
                &nwg::MessageParams {
                    title: APP_NAME,
                    content: &format!(
                        "실행 중인 서비스 {}개를 정상 중지한 뒤 종료할까?",
                        active.len()
                    ),
                    buttons: nwg::MessageButtons::YesNo,
                    icons: nwg::MessageIcons::Question,
                },
            );
            if choice != nwg::MessageChoice::Yes {
                return;
            }
        }
        self.state.borrow_mut().quitting = true;
        log::info!("종료 요청, 실행 중 서비스 {:?} 중지", active);
        let mgr = self.manager.borrow();
        mgr.stop_all(StopMode::Normal);
        let activity = mgr.activity();
        let ready = self.quit_ready.clone();
        let sender = self.ui.notice.sender();
        std::thread::spawn(move || {
            activity.wait_inactive(QUIT_WAIT);
            ready.store(true, Ordering::SeqCst);
            sender.notice();
        });
    }

    fn save_config(&self) -> bool {
        let path = paths::config_file();
        match self.config.borrow().save(&path) {
            Ok(()) => true,
            Err(e) => {
                nwg::modal_error_message(
                    &self.ui.window,
                    APP_NAME,
                    &format!("설정 저장 실패: {e}"),
                );
                false
            }
        }
    }

    fn apply_config(&self) {
        let config = self.config.borrow();
        self.manager.borrow_mut().apply(&config);
        self.console
            .borrow_mut()
            .set_capacity(config.app.scrollback_lines);
        drop(config);
        self.rebuild_rows();
    }

    fn reload_config(&self, interactive: bool) {
        let config = Config::load_or_create(&paths::config_file());
        let mut problems: Vec<String> = config.error.iter().cloned().collect();
        problems.extend(config.services.iter().filter_map(|e| {
            e.error
                .as_ref()
                .map(|err| format!("{}: {err}", e.config.id))
        }));
        *self.config.borrow_mut() = config;
        self.apply_config();
        if interactive && !problems.is_empty() {
            nwg::modal_error_message(&self.ui.window, "설정 오류", &problems.join("\n"));
        }
    }

    fn run_dialog(
        &self,
        initial: &ServiceConfig,
        taken: Vec<String>,
        title: &str,
    ) -> Option<ServiceConfig> {
        self.state.borrow_mut().dialog_open = true;
        self.show_window();
        let result = edit_service(&self.ui.window, initial, taken, title);
        self.state.borrow_mut().dialog_open = false;
        result
    }

    fn add_service(&self) {
        let taken: Vec<String> = self
            .config
            .borrow()
            .services
            .iter()
            .map(|e| e.config.id.clone())
            .collect();
        let mut initial = ServiceConfig::new("", "");
        initial.stop_command = String::new();
        let Some(cfg) = self.run_dialog(&initial, taken, "서비스 추가") else {
            return;
        };
        let id = cfg.id.clone();
        self.config
            .borrow_mut()
            .services
            .push(ServiceEntry::valid(cfg));
        if self.save_config() {
            self.state.borrow_mut().selected = Some(id);
            self.apply_config();
        }
    }

    fn edit_selected(&self) {
        let Some(id) = self.state.borrow().selected.clone() else {
            return;
        };
        let Some(index) = self
            .config
            .borrow()
            .services
            .iter()
            .position(|e| e.config.id == id)
        else {
            return;
        };
        let initial = self.config.borrow().services[index].config.clone();
        let taken: Vec<String> = self
            .config
            .borrow()
            .services
            .iter()
            .enumerate()
            .filter(|(i, _)| *i != index)
            .map(|(_, e)| e.config.id.clone())
            .collect();
        let Some(cfg) = self.run_dialog(&initial, taken, "서비스 편집") else {
            return;
        };
        if cfg.id != id
            && self
                .manager
                .borrow()
                .get(&id)
                .is_some_and(|s| s.status().state.is_active())
        {
            nwg::modal_error_message(
                &self.ui.window,
                APP_NAME,
                "실행 중인 서비스의 id 는 바꿀 수 없어. 먼저 중지해줘.",
            );
            return;
        }
        let new_id = cfg.id.clone();
        self.config.borrow_mut().services[index] = ServiceEntry::valid(cfg);
        if self.save_config() {
            self.state.borrow_mut().selected = Some(new_id);
            self.apply_config();
        }
    }

    fn delete_selected(&self) {
        let Some(id) = self.state.borrow().selected.clone() else {
            return;
        };
        let choice = nwg::modal_message(
            &self.ui.window,
            &nwg::MessageParams {
                title: APP_NAME,
                content: &format!(
                    "\"{id}\" 서비스를 삭제할까? 실행 중이면 정상 중지 절차 후 제거돼."
                ),
                buttons: nwg::MessageButtons::YesNo,
                icons: nwg::MessageIcons::Warning,
            },
        );
        if choice != nwg::MessageChoice::Yes {
            return;
        }
        self.config
            .borrow_mut()
            .services
            .retain(|e| e.config.id != id);
        if self.save_config() {
            self.state.borrow_mut().selected = None;
            self.apply_config();
        }
    }

    fn open_service_logs(&self) {
        let Some(id) = self.state.borrow().selected.clone() else {
            return;
        };
        let dir = paths::logs_dir().join(id);
        let _ = std::fs::create_dir_all(&dir);
        let _ = win::shell_open(&dir.to_string_lossy());
    }

    fn toggle_autostart(&self) {
        let enable = !self.ui.mi_autostart.checked();
        let result = if enable {
            let exe = std::env::current_exe().unwrap_or_default();
            win::set_run_entry(RUN_VALUE, &format!("\"{}\" --tray", exe.display()))
        } else {
            win::remove_run_entry(RUN_VALUE)
        };
        match result {
            Ok(()) => self.ui.mi_autostart.set_checked(enable),
            Err(e) => {
                nwg::modal_error_message(
                    &self.ui.window,
                    APP_NAME,
                    &format!("시작 프로그램 등록 실패: {e}"),
                );
            }
        }
    }

    /// Detaches handlers after the message loop ends; dropping the app then kills leftovers.
    pub fn shutdown(&self) {
        self.ticker_stop.store(true, Ordering::Relaxed);
        for h in self.handlers.borrow_mut().drain(..) {
            let _ = nwg::unbind_raw_event_handler(&h);
        }
        if let Some(h) = self.event_handler.borrow_mut().take() {
            nwg::unbind_event_handler(&h);
        }
    }
}

fn row_texts(svc: &Service) -> [String; 7] {
    let st = svc.status();
    let cfg = svc.config();
    let running = st.state == State::Running;
    [
        cfg.display_name().to_owned(),
        state_label(st.state).to_owned(),
        st.pid.map(|p| p.to_string()).unwrap_or_default(),
        if running {
            format!("{:.1}%", st.cpu_percent)
        } else {
            String::new()
        },
        if running {
            memory(st.memory_bytes)
        } else {
            String::new()
        },
        st.started_at
            .map(|t| uptime(t.elapsed()))
            .unwrap_or_default(),
        st.restarts.to_string(),
    ]
}

fn open_in_editor(path: &str) {
    if win::shell_open(path).is_err() {
        let _ = std::process::Command::new("notepad.exe").arg(path).spawn();
    }
}

/// Window procedures cannot unwind; a panic is logged by the hook and the event dropped.
fn guarded<T>(f: impl FnOnce() -> T) -> Option<T> {
    catch_unwind(AssertUnwindSafe(f)).ok()
}
