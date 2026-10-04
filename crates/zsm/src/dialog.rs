use std::cell::RefCell;
use std::collections::BTreeMap;
use std::rc::Rc;

use native_windows_gui as nwg;
use zsm_core::config::{IoMode, Kind, RestartPolicy, ServiceConfig};

use crate::cmdline::{join_args, split_args};

const LABEL_W: i32 = 110;
const ROW_H: i32 = 30;
const FIELD_X: i32 = 16 + LABEL_W;
const FIELD_W: i32 = 400;
const BROWSE_W: i32 = 70;

#[derive(Default)]
struct Controls {
    window: nwg::Window,
    labels: Vec<nwg::Label>,
    id: nwg::TextInput,
    name: nwg::TextInput,
    kind_console: nwg::RadioButton,
    kind_gui: nwg::RadioButton,
    command: nwg::TextInput,
    command_browse: nwg::Button,
    args: nwg::TextInput,
    working_dir: nwg::TextInput,
    working_dir_browse: nwg::Button,
    env: nwg::TextInput,
    io_pty: nwg::RadioButton,
    io_pipe: nwg::RadioButton,
    autostart: nwg::CheckBox,
    restart_never: nwg::RadioButton,
    restart_failure: nwg::RadioButton,
    restart_always: nwg::RadioButton,
    restart_max: nwg::TextInput,
    restart_window: nwg::TextInput,
    stop_command: nwg::TextInput,
    stop_timeout: nwg::TextInput,
    ctrl_c_timeout: nwg::TextInput,
    save: nwg::Button,
    cancel: nwg::Button,
    file_dialog: nwg::FileDialog,
    folder_dialog: nwg::FileDialog,
}

fn y(row: i32) -> i32 {
    12 + row * ROW_H
}

fn label(c: &mut Controls, text: &str, row: i32) -> Result<(), nwg::NwgError> {
    let mut l = nwg::Label::default();
    nwg::Label::builder()
        .text(text)
        .position((16, y(row) + 3))
        .size((LABEL_W - 8, 22))
        .parent(c.window.handle)
        .build(&mut l)?;
    c.labels.push(l);
    Ok(())
}

fn input(
    parent: nwg::ControlHandle,
    out: &mut nwg::TextInput,
    text: &str,
    pos: (i32, i32),
    width: i32,
) -> Result<(), nwg::NwgError> {
    nwg::TextInput::builder()
        .text(text)
        .position(pos)
        .size((width, 24))
        .parent(parent)
        .build(out)
}

fn radio(
    parent: nwg::ControlHandle,
    out: &mut nwg::RadioButton,
    text: &str,
    pos: (i32, i32),
    first: bool,
    checked: bool,
) -> Result<(), nwg::NwgError> {
    let mut flags = nwg::RadioButtonFlags::VISIBLE;
    if first {
        flags |= nwg::RadioButtonFlags::GROUP;
    }
    nwg::RadioButton::builder()
        .text(text)
        .flags(flags)
        .position(pos)
        .size((110, 24))
        .check_state(if checked {
            nwg::RadioButtonState::Checked
        } else {
            nwg::RadioButtonState::Unchecked
        })
        .parent(parent)
        .build(out)
}

fn button(
    parent: nwg::ControlHandle,
    out: &mut nwg::Button,
    text: &str,
    pos: (i32, i32),
    width: i32,
) -> Result<(), nwg::NwgError> {
    nwg::Button::builder()
        .text(text)
        .position(pos)
        .size((width, 26))
        .parent(parent)
        .build(out)
}

fn env_to_text(env: &BTreeMap<String, String>) -> String {
    env.iter()
        .map(|(k, v)| format!("{k}={v}"))
        .collect::<Vec<_>>()
        .join("; ")
}

fn text_to_env(text: &str) -> Result<BTreeMap<String, String>, String> {
    text.split(';')
        .map(str::trim)
        .filter(|p| !p.is_empty())
        .map(|p| match p.split_once('=') {
            Some((k, v)) if !k.trim().is_empty() => Ok((k.trim().to_owned(), v.trim().to_owned())),
            _ => Err(format!("환경 변수 `{p}` 는 KEY=VALUE 형식이어야 해")),
        })
        .collect()
}

fn build(
    c: &mut Controls,
    cfg: &ServiceConfig,
    title: &str,
    parent: &nwg::Window,
) -> Result<(), nwg::NwgError> {
    nwg::Window::builder()
        .title(title)
        .size((FIELD_X + FIELD_W + BROWSE_W + 30, y(14) + 60))
        .center(true)
        .flags(nwg::WindowFlags::WINDOW)
        .parent(Some(parent))
        .build(&mut c.window)?;

    let w = c.window.handle;
    let rows = [
        "ID",
        "이름",
        "종류",
        "실행 파일",
        "인자",
        "작업 폴더",
        "환경 변수",
        "입출력",
        "자동 시작",
        "재시작 정책",
        "재시작 한도",
        "중지 명령",
        "중지 대기",
    ];
    for (i, text) in rows.iter().enumerate() {
        label(c, text, i as i32)?;
    }
    input(w, &mut c.id, &cfg.id, (FIELD_X, y(0)), 200)?;
    input(w, &mut c.name, &cfg.name, (FIELD_X, y(1)), FIELD_W)?;
    radio(
        w,
        &mut c.kind_console,
        "콘솔",
        (FIELD_X, y(2)),
        true,
        cfg.kind == Kind::Console,
    )?;
    radio(
        w,
        &mut c.kind_gui,
        "GUI",
        (FIELD_X + 120, y(2)),
        false,
        cfg.kind == Kind::Gui,
    )?;
    input(w, &mut c.command, &cfg.command, (FIELD_X, y(3)), FIELD_W)?;
    button(
        w,
        &mut c.command_browse,
        "찾기...",
        (FIELD_X + FIELD_W + 6, y(3) - 1),
        BROWSE_W,
    )?;
    input(
        w,
        &mut c.args,
        &join_args(&cfg.args),
        (FIELD_X, y(4)),
        FIELD_W,
    )?;
    input(
        w,
        &mut c.working_dir,
        &cfg.working_dir,
        (FIELD_X, y(5)),
        FIELD_W,
    )?;
    button(
        w,
        &mut c.working_dir_browse,
        "찾기...",
        (FIELD_X + FIELD_W + 6, y(5) - 1),
        BROWSE_W,
    )?;
    input(
        w,
        &mut c.env,
        &env_to_text(&cfg.env),
        (FIELD_X, y(6)),
        FIELD_W,
    )?;
    radio(
        w,
        &mut c.io_pty,
        "PTY (ConPTY)",
        (FIELD_X, y(7)),
        true,
        cfg.io_mode == IoMode::Pty,
    )?;
    radio(
        w,
        &mut c.io_pipe,
        "Pipe",
        (FIELD_X + 120, y(7)),
        false,
        cfg.io_mode == IoMode::Pipe,
    )?;
    nwg::CheckBox::builder()
        .text("앱 시작 시 자동 실행")
        .position((FIELD_X, y(8)))
        .size((250, 24))
        .check_state(if cfg.autostart {
            nwg::CheckBoxState::Checked
        } else {
            nwg::CheckBoxState::Unchecked
        })
        .parent(w)
        .build(&mut c.autostart)?;
    radio(
        w,
        &mut c.restart_never,
        "안 함",
        (FIELD_X, y(9)),
        true,
        cfg.restart == RestartPolicy::Never,
    )?;
    radio(
        w,
        &mut c.restart_failure,
        "실패 시",
        (FIELD_X + 120, y(9)),
        false,
        cfg.restart == RestartPolicy::OnFailure,
    )?;
    radio(
        w,
        &mut c.restart_always,
        "항상",
        (FIELD_X + 240, y(9)),
        false,
        cfg.restart == RestartPolicy::Always,
    )?;
    input(
        w,
        &mut c.restart_window,
        &cfg.restart_window_secs.to_string(),
        (FIELD_X, y(10)),
        60,
    )?;
    let mut l = nwg::Label::default();
    nwg::Label::builder()
        .text("초 안에 최대")
        .position((FIELD_X + 66, y(10) + 3))
        .size((80, 22))
        .parent(w)
        .build(&mut l)?;
    c.labels.push(l);
    input(
        w,
        &mut c.restart_max,
        &cfg.restart_max.to_string(),
        (FIELD_X + 150, y(10)),
        50,
    )?;
    let mut l = nwg::Label::default();
    nwg::Label::builder()
        .text("회")
        .position((FIELD_X + 206, y(10) + 3))
        .size((30, 22))
        .parent(w)
        .build(&mut l)?;
    c.labels.push(l);
    input(
        w,
        &mut c.stop_command,
        &cfg.stop_command,
        (FIELD_X, y(11)),
        200,
    )?;
    input(
        w,
        &mut c.stop_timeout,
        &cfg.stop_timeout_secs.to_string(),
        (FIELD_X, y(12)),
        60,
    )?;
    let mut l = nwg::Label::default();
    nwg::Label::builder()
        .text("초,  Ctrl+C 후")
        .position((FIELD_X + 66, y(12) + 3))
        .size((90, 22))
        .parent(w)
        .build(&mut l)?;
    c.labels.push(l);
    input(
        w,
        &mut c.ctrl_c_timeout,
        &cfg.ctrl_c_timeout_secs.to_string(),
        (FIELD_X + 160, y(12)),
        60,
    )?;
    let mut l = nwg::Label::default();
    nwg::Label::builder()
        .text("초")
        .position((FIELD_X + 226, y(12) + 3))
        .size((30, 22))
        .parent(w)
        .build(&mut l)?;
    c.labels.push(l);
    let right = FIELD_X + FIELD_W + BROWSE_W + 6;
    button(w, &mut c.save, "저장", (right - 180, y(13) + 6), 85)?;
    button(w, &mut c.cancel, "취소", (right - 85, y(13) + 6), 85)?;

    nwg::FileDialog::builder()
        .title("실행 파일 선택")
        .action(nwg::FileDialogAction::Open)
        .filters("실행 파일(*.exe;*.bat;*.cmd)|모든 파일(*.*)")
        .build(&mut c.file_dialog)?;
    nwg::FileDialog::builder()
        .title("작업 폴더 선택")
        .action(nwg::FileDialogAction::OpenDirectory)
        .build(&mut c.folder_dialog)?;
    Ok(())
}

fn number<T: std::str::FromStr>(input: &nwg::TextInput, what: &str) -> Result<T, String> {
    input
        .text()
        .trim()
        .parse()
        .map_err(|_| format!("{what} 는 0 이상의 정수여야 해"))
}

fn collect(c: &Controls, base: &ServiceConfig) -> Result<ServiceConfig, String> {
    let mut cfg = base.clone();
    cfg.id = c.id.text().trim().to_owned();
    cfg.name = c.name.text().trim().to_owned();
    cfg.kind = if c.kind_gui.check_state() == nwg::RadioButtonState::Checked {
        Kind::Gui
    } else {
        Kind::Console
    };
    cfg.command = c.command.text().trim().to_owned();
    cfg.args = split_args(&c.args.text());
    cfg.working_dir = c.working_dir.text().trim().to_owned();
    cfg.env = text_to_env(&c.env.text())?;
    cfg.io_mode = if c.io_pipe.check_state() == nwg::RadioButtonState::Checked {
        IoMode::Pipe
    } else {
        IoMode::Pty
    };
    cfg.autostart = c.autostart.check_state() == nwg::CheckBoxState::Checked;
    cfg.restart = if c.restart_never.check_state() == nwg::RadioButtonState::Checked {
        RestartPolicy::Never
    } else if c.restart_always.check_state() == nwg::RadioButtonState::Checked {
        RestartPolicy::Always
    } else {
        RestartPolicy::OnFailure
    };
    cfg.restart_max = number(&c.restart_max, "재시작 한도")?;
    cfg.restart_window_secs = number(&c.restart_window, "재시작 기간")?;
    cfg.stop_command = c.stop_command.text().trim().to_owned();
    cfg.stop_timeout_secs = number(&c.stop_timeout, "중지 대기")?;
    cfg.ctrl_c_timeout_secs = number(&c.ctrl_c_timeout, "Ctrl+C 대기")?;
    cfg.validate()?;
    Ok(cfg)
}

/// Shows the add/edit dialog in a nested message loop. `taken_ids` are ids the result may not use.
pub fn edit_service(
    parent: &nwg::Window,
    initial: &ServiceConfig,
    taken_ids: Vec<String>,
    title: &str,
) -> Option<ServiceConfig> {
    let mut controls = Controls::default();
    if let Err(e) = build(&mut controls, initial, title, parent) {
        nwg::modal_error_message(
            parent,
            "Z Service Manager",
            &format!("대화상자 생성 실패: {e}"),
        );
        return None;
    }
    let controls = Rc::new(controls);
    let result: Rc<RefCell<Option<ServiceConfig>>> = Rc::new(RefCell::new(None));
    let base = initial.clone();

    let c = Rc::downgrade(&controls);
    let res = result.clone();
    let handler =
        nwg::full_bind_event_handler(&controls.window.handle, move |evt, _data, handle| {
            let Some(c) = c.upgrade() else { return };
            match evt {
                nwg::Event::OnButtonClick if handle == c.command_browse.handle => {
                    if c.file_dialog.run(Some(&c.window)) {
                        if let Ok(path) = c.file_dialog.get_selected_item() {
                            c.command.set_text(&path.to_string_lossy());
                        }
                    }
                }
                nwg::Event::OnButtonClick if handle == c.working_dir_browse.handle => {
                    if c.folder_dialog.run(Some(&c.window)) {
                        if let Ok(path) = c.folder_dialog.get_selected_item() {
                            c.working_dir.set_text(&path.to_string_lossy());
                        }
                    }
                }
                nwg::Event::OnButtonClick if handle == c.save.handle => match collect(&c, &base) {
                    Ok(cfg) if taken_ids.contains(&cfg.id) => {
                        nwg::modal_error_message(
                            &c.window,
                            "입력 오류",
                            &format!("id `{}` 는 이미 쓰고 있어", cfg.id),
                        );
                    }
                    Ok(cfg) => {
                        *res.borrow_mut() = Some(cfg);
                        c.window.close();
                    }
                    Err(e) => {
                        nwg::modal_error_message(&c.window, "입력 오류", &e);
                    }
                },
                nwg::Event::OnButtonClick if handle == c.cancel.handle => c.window.close(),
                nwg::Event::OnWindowClose if handle == c.window.handle => {
                    nwg::stop_thread_dispatch()
                }
                _ => {}
            }
        });

    parent.set_enabled(false);
    controls.window.set_visible(true);
    controls.id.set_focus();
    nwg::dispatch_thread_events();
    parent.set_enabled(true);
    parent.set_focus();
    nwg::unbind_event_handler(&handler);
    let out = result.borrow_mut().take();
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn env_text_round_trip() {
        let env = text_to_env("A=1; B = two words ;").unwrap();
        assert_eq!(env.get("B").map(String::as_str), Some("two words"));
        assert_eq!(text_to_env(&env_to_text(&env)).unwrap(), env);
        assert!(text_to_env("novalue").is_err());
    }
}
