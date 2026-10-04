# Z Service Manager

콘솔 프로그램과 GUI 프로그램을 자식 프로세스로 띄워 관리하는 Windows 트레이 앱.

- 콘솔 프로그램의 출력을 실시간으로 보여주고, 입력창으로 명령을 보낼 수 있다.
- GUI 프로그램은 창을 숨긴 채 실행하고, 필요할 때 다시 보이게 한다.
- 상태, PID, CPU, 메모리, 가동 시간, 재시작 횟수를 추적하고, 크래시 시 정책에 따라 재시작한다.
- Windows 로그온 시 트레이로 자동 시작할 수 있다.
- GitHub Releases 를 통해 자동 업데이트한다.

## 설치

[Releases](https://github.com/Zeliper/Z-Service-Manager/releases/latest) 에서 받는다.

- `ZServiceManager-Setup-x.y.z.exe`: 설치 프로그램. 관리자 권한 없이 `%LOCALAPPDATA%\Programs\ZServiceManager` 에 설치된다. 설치 중 "Windows 시작 시 실행" 을 고를 수 있다.
- `zsm-x.y.z-x86_64-pc-windows-msvc.zip`: 포터블. 압축을 풀고 `zsm.exe` 를 실행한다. 포터블은 자동 설치 대신 릴리스 페이지만 연다.
- `SHA256SUMS.txt`: 위 파일들의 SHA256.

코드 서명이 없어서 처음 실행할 때 SmartScreen 경고("Windows의 PC 보호")가 뜰 수 있다. "추가 정보" → "실행" 을 누르면 된다.

## 사용법

- 창을 닫으면(X) 트레이로 숨는다. 트레이 아이콘 왼쪽 클릭으로 열고 닫고, 오른쪽 클릭으로 메뉴(열기, 모두 시작, 모두 중지, 업데이트 확인, 종료)를 연다.
- 트레이 아이콘 색: 회색(실행 중인 서비스 없음), 초록(autostart 서비스 모두 실행 중), 노랑(시작/중지/재시작 대기 중이거나 autostart 서비스 일부가 꺼져 있음), 빨강(실패 또는 설정 오류).
- `zsm.exe --tray` 는 창 없이 트레이로 시작한다. 이미 실행 중일 때 다시 실행하면 기존 창이 앞으로 나온다.
- "파일 → Windows 시작 시 실행" 은 `HKCU\Software\Microsoft\Windows\CurrentVersion\Run` 에 등록/해제한다.

### Ctrl+C 동작

Ctrl+C 는 자식 프로세스에 바로 전달하지 않는다.

- 콘솔 출력 영역에 선택된 텍스트가 있으면 복사한다.
- 선택이 없으면 `"<이름>" 서비스를 중지할까?` 확인 창을 띄우고, "예" 를 누르면 아래 정상 중지 절차를 밟는다.

### 정상 중지 절차

콘솔 서비스:

1. `stop_command` 가 있으면 입력으로 보내고 `stop_timeout_secs` 동안 기다린다.
2. 살아 있으면 Ctrl+C 를 보낸다(PTY: `\x03`, pipe: Ctrl+Break) → `ctrl_c_timeout_secs` 대기.
3. 그래도 살아 있으면 Job Object 를 종료한다.

GUI 서비스는 최상위 창에 `WM_CLOSE` → `stop_timeout_secs` 대기 → Job 종료 순서다. 각 단계는 콘솔에 `[zsm] ...` 줄로 남는다.

Windows 로그오프/종료 때도 같은 절차를 밟되, 단계별 대기는 최대 20초로 줄인다.

## 설정

`%APPDATA%\ZServiceManager\config.toml`. 없으면 기본값으로 만든다. 메뉴의 서비스 추가/편집으로 고칠 수 있고, 저장 전 `config.toml.bak` 을 남긴다(사용자 주석은 보존하지 않는다). 직접 고쳤다면 "파일 → 설정 다시 불러오기". 경로의 `%VAR%` 는 환경 변수로 확장된다. 잘못된 서비스 항목은 그 서비스만 "설정 오류" 로 표시된다.

Vintage Story 전용 서버 예시:

```toml
[app]
start_minimized = true            # 시작 시 창 없이 트레이로
check_updates = true
update_check_interval_hours = 6
scrollback_lines = 10000
log_retention_days = 14

[[service]]
id = "vintagestory"               # 영문/숫자/-/_ , 고유
name = "Vintage Story Server"
kind = "console"                  # console | gui
command = '%APPDATA%\VintageStory\VintageStoryServer.exe'
args = []                         # 예: ["--dataPath", 'D:\VSData']
working_dir = '%APPDATA%\VintageStory'
env = {}
io_mode = "pty"                   # pty | pipe   (console 전용)
autostart = true                  # 앱 시작 시 자동 실행
restart = "on-failure"            # never | on-failure | always
restart_max = 5                   # restart_window_secs 안에서 이 횟수 초과 시 실패
restart_window_secs = 600
stop_command = "/stop"            # 비어 있으면 이 단계 건너뜀
stop_timeout_secs = 60
ctrl_c_timeout_secs = 15
```

GUI 프로그램 예시:

```toml
[[service]]
id = "notepad"
name = "Notepad"
kind = "gui"
command = 'C:\Windows\System32\notepad.exe'
restart = "never"
stop_timeout_secs = 10
```

재시작 대기 시간은 1, 2, 4 … 최대 60초이고, 60초 이상 정상 실행되면 처음부터 다시 센다. 종료 코드 0 은 정상 종료로 보고 `restart = "always"` 일 때만 재시작한다.

## 파일 위치

| 내용 | 위치 |
|---|---|
| 설정 | `%APPDATA%\ZServiceManager\config.toml` |
| 서비스 로그 | `%LOCALAPPDATA%\ZServiceManager\logs\<id>\YYYY-MM-DD.log` (escape 제거, 타임스탬프 접두, `log_retention_days` 지나면 시작 시 삭제) |
| 앱 로그 | `%LOCALAPPDATA%\ZServiceManager\zsm.log` |

## 자동 업데이트

시작 30초 뒤, 이후 `update_check_interval_hours` 마다 GitHub 최신 릴리스를 확인한다(도움말 → 업데이트 확인은 즉시). 새 버전이면 알림을 띄우고 "도움말 → 업데이트 설치" 가 켜진다. 설치하면 설치 프로그램과 `SHA256SUMS.txt` 를 받아 해시를 검증하고, 실행 중이던 서비스를 정상 중지한 뒤 조용히 설치하고, 새 버전이 그 서비스들을 다시 실행한다. 해시가 맞지 않으면 중단한다.

## 알려진 제약

- ConPTY(`io_mode = "pty"`)는 프로세스를 일시 정지 상태로 만들 수 없어서, 자식이 시작된 직후 Job Object 에 들어가기 전 아주 짧은 순간이 있다. 그 사이에 자식이 손자 프로세스를 만들면 손자는 Job 밖에 남을 수 있다. `pipe` 모드와 `gui` 는 일시 정지 생성 후 Job 에 넣으므로 이 문제가 없다.
- ConPTY 는 폭(250열)을 넘는 줄을 강제로 나눈다. 아주 긴 줄은 여러 줄로 보인다.
- 줄 단위로 로그를 출력하는 프로그램이 대상이다. vim, htop 같은 전체 화면 TUI 는 제대로 보이지 않는다. 줄바꿈 없이 출력된 프롬프트는 줄이 끝날 때까지 표시되지 않는다.
- pipe 모드 출력은 UTF-8 로 해석하고, 잘못된 바이트는 대체 문자로 바꾼다.
- pipe 모드의 Ctrl+Break 는 `zsm.exe --ctrl-break <pid>` 헬퍼 프로세스가 자식 콘솔에 붙어서 보낸다.
- GUI 서비스는 처음 실행한 프로세스가 끝나도 같은 Job 에 프로세스가 남아 있으면 계속 실행 중으로 본다(Windows 11 메모장처럼 런처가 실제 앱을 띄우고 끝나는 경우).
- 코드 서명이 없어 SmartScreen 경고가 뜬다.
- Windows 서비스(Session 0)로는 동작하지 않는다. 로그온한 사용자 세션에서 돈다.

## 빌드

요구 사항: Rust stable (`x86_64-pc-windows-msvc`), Visual Studio Build Tools (VC++ 워크로드), 설치 프로그램은 Inno Setup 6.

```
cargo build --workspace
cargo test --workspace
cargo build --release
ISCC /DAppVersion=0.1.0 installer\zsm.iss
```

결과물: `target\release\zsm.exe`, `installer\Output\ZServiceManager-Setup-0.1.0.exe`.

트레이 아이콘은 `crates/zsm/res/icons/generate.ps1` 로 다시 만들 수 있다.

릴리스: `[workspace.package] version` 을 올리고 같은 버전의 `vX.Y.Z` 태그를 push 하면 `.github/workflows/release.yml` 이 설치 프로그램, 포터블 zip, `SHA256SUMS.txt` 를 올린 릴리스를 만든다.

## 라이선스

MIT
