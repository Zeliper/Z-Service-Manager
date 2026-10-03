# Z Service Manager — 구현 목표

이 문서는 Claude Code 에 전달하는 구현 목표다. 저장소 루트(`C:\Users\Zeliper\Workspace\Z-Service-Manager`)에서 실행한다.
마일스톤 순서대로 진행하고, 각 마일스톤의 완료 기준을 실제로 검증한 뒤 커밋한다.
"결정 사항" 은 이미 확정된 것이다. 바꿔야 할 근거가 생기면 바꾸지 말고 먼저 사용자에게 보고한다.

---

## 1. 무엇을 만드는가

콘솔 프로그램과 GUI 프로그램을 **자식 프로세스로 띄워 관리하는 Windows 트레이 앱**.

- 콘솔 프로그램의 stdout/stderr 를 부모가 받아 실시간으로 보여주고, stdin 으로 명령을 입력할 수 있다.
- GUI 프로그램은 창을 숨긴 채(headless) 실행하고, 필요할 때 창을 다시 보이게 할 수 있다.
- 상태(실행 중/중지/크래시/재시작 대기), PID, CPU, 메모리, 가동 시간, 재시작 횟수를 추적한다.
- 크래시 시 정책에 따라 자동 재시작한다.
- Windows 로그온 시 자동 시작되고, 트레이에서 동작한다.
- GitHub Releases 로 배포되며, 설치 프로그램과 자동 업데이트를 지원한다.

첫 번째 실사용 대상은 Vintage Story 전용 서버(`%APPDATA%\VintageStory\VintageStoryServer.exe`, .NET 콘솔 앱)다.

### 범위 밖

- Windows 서비스(Session 0) 모드. 이 앱은 로그온한 사용자 세션에서 돈다.
- 전체 화면 TUI 앱(vim, htop 류)의 정확한 렌더링. 로그를 줄 단위로 출력하는 앱이 대상이다.
- 웹 UI, 원격 제어, 다중 사용자.
- Windows 외 OS.
- 코드 서명(SmartScreen 경고는 README 에 안내만 한다).

---

## 2. 결정 사항

| 항목 | 결정 |
|---|---|
| 언어 | Rust (stable, edition 2021), 타깃 `x86_64-pc-windows-msvc` |
| GUI | [`native-windows-gui`](https://github.com/gabdube/native-windows-gui) (`native-windows-gui` 1.0.x + `native-windows-derive`). feature: `tray-notification`, `menu`, `rich-textbox`, `list-view`, `notice`, `message-window`, `file-dialog`, `high-dpi`, `image-decoder` 중 필요한 것만 |
| 비동기 | tokio 쓰지 않음. `std::thread` + `crossbeam-channel`. 워커 → UI 전달은 `nwg::Notice` + 공유 큐 |
| 콘솔 연결 | 기본 ConPTY (`portable-pty`). 서비스별로 `io_mode = "pipe"` 선택 가능 |
| 출력 해석 | `vte` 기반 줄 단위 처리기 (아래 4.3) |
| 프로세스 정리 | 서비스마다 Job Object, `JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE` |
| Win32 API | `windows` crate |
| 프로세스 지표 | `sysinfo` |
| 설정 | TOML (`serde`, `toml`) |
| 리소스/매니페스트 | `embed-resource` 로 `.rc` (아이콘, comctl32 v6 + PerMonitorV2 DPI 매니페스트) |
| HTTP | `ureq` (rustls) — 업데이트 확인/다운로드 전용 |
| 해시 | `sha2` |
| 버전 비교 | `semver` |
| 로깅(앱 자체) | `log` + 파일 로거 하나 (`simplelog` 또는 동급) |
| 설치 프로그램 | Inno Setup 6, per-user (`PrivilegesRequired=lowest`), 설치 경로 `{localappdata}\Programs\ZServiceManager` |
| 자동 업데이트 | GitHub Releases latest 조회 → 설치 프로그램 다운로드 → SHA256 검증 → silent 설치 |
| 저장소 | GitHub `Zeliper/Z-Service-Manager`, **public**, MIT License, `gh` CLI 로 생성 |
| 실행 파일 이름 | `zsm.exe` |
| 단일 인스턴스 mutex | `Local\ZServiceManager.SingleInstance` (Inno Setup `AppMutex` 와 동일 문자열) |

위 목록에 없는 의존성은 추가 전에 이미 있는 것으로 해결되는지 먼저 확인한다.

---

## 3. 저장소 구조

```
Z-Service-Manager/
├─ Cargo.toml                 # workspace, [workspace.package] version 이 단일 버전 원천
├─ crates/
│  ├─ zsm-core/               # GUI 비의존. 프로세스 감독, PTY/pipe, 출력 처리, 설정, 업데이트 로직
│  │  ├─ src/
│  │  └─ src/bin/zsm-testchild.rs   # 테스트용 자식 프로그램
│  └─ zsm/                    # NWG GUI 앱 (bin: zsm.exe)
│     ├─ build.rs             # embed-resource
│     └─ res/                 # app.rc, app.manifest, icons/*.ico
├─ installer/
│  └─ zsm.iss
├─ .github/workflows/
│  ├─ ci.yml
│  └─ release.yml
├─ README.md
├─ LICENSE                    # MIT
└─ GOAL.md                    # 이 문서
```

`zsm-core` 는 `nwg` 를 의존하지 않는다. GUI 없이 테스트 가능해야 한다.

---

## 4. 기능 명세

### 4.1 서비스 설정

설정 파일: `%APPDATA%\ZServiceManager\config.toml`. 없으면 기본값으로 생성한다.
문자열 경로의 `%VAR%` 는 `ExpandEnvironmentStringsW` 로 확장한다.

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
args = []
working_dir = '%APPDATA%\VintageStory'
env = {}
io_mode = "pty"                   # pty | pipe   (console 전용)
autostart = true                  # 앱 시작 시 자동 실행
restart = "on-failure"            # never | on-failure | always
restart_max = 5                   # restart_window_secs 안에서 이 횟수 초과 시 Failed
restart_window_secs = 600
stop_command = "/stop"            # 비어 있으면 이 단계 건너뜀
stop_timeout_secs = 60
ctrl_c_timeout_secs = 15
```

설정 검증 오류는 앱을 죽이지 않는다. 해당 서비스만 `Invalid` 상태로 표시하고 오류 문구를 보여준다.
"Windows 시작 시 실행" 여부는 설정 파일이 아니라 레지스트리(4.8)가 단일 원천이다.

### 4.2 프로세스 감독 (`zsm-core`)

서비스마다 감독 스레드 하나. 상태 머신:

```
Stopped → Starting → Running → Stopping → Stopped
                       │
                       └─(비정상 종료)→ Crashed → Backoff → Starting
                                                 └─(restart_max 초과)→ Failed
Invalid (설정 오류)
```

- 실행 직후 프로세스를 해당 서비스의 Job Object 에 넣는다. 앱이 어떤 이유로 죽어도 자식·손자가 남지 않아야 한다.
  - `portable-pty` 는 suspended 생성이 안 되므로 할당 전 짧은 race 가 있다. 허용하되 README 의 알려진 제약에 적는다.
  - `kind = "gui"` 와 `io_mode = "pipe"` 는 `CreateProcessW` 를 직접 호출해 `CREATE_SUSPENDED` → Job 할당 → `ResumeThread` 로 race 를 없앤다.
- 재시작 backoff: 1s, 2s, 4s … 최대 60s. 정상 실행이 60초 이상 유지되면 backoff 를 초기화한다.
- 종료 코드 0 이면 정상 종료. `restart = "always"` 일 때만 재시작한다.
- 사용자가 중지한 경우 재시작하지 않는다.
- 지표: 2초마다 Job 의 프로세스 목록(`JobObjectBasicProcessIdList`)을 `sysinfo` 로 합산해 CPU%, 메모리를 갱신한다.

### 4.3 출력 처리

ConPTY 와 pipe 모두 같은 처리기를 통과한다.

- `vte` 파서로 바이트를 해석한다. `LF` 에서 줄을 확정하고, `CR` 은 현재 줄 커서를 0 으로, `BS`, `EL`(erase line) 은 현재 줄에 반영한다. 그 외 커서 이동, 화면 지우기, 모드 변경 시퀀스는 무시한다.
- SGR 색상은 줄 데이터에 span 으로 보존한다(표시는 M4 에서 선택 구현).
- 확정된 줄은 다음 세 곳으로 간다.
  1. 서비스별 링 버퍼 (`scrollback_lines`)
  2. 로그 파일 `%LOCALAPPDATA%\ZServiceManager\logs\<id>\YYYY-MM-DD.log` (escape 제거된 평문, 타임스탬프 접두). `log_retention_days` 지난 파일은 시작 시 삭제
  3. UI (새 줄만 증분 전달)
- ConPTY 는 PTY 폭에서 강제로 줄을 바꾸므로 cols 를 넉넉히(기본 250) 잡는다.
- pipe 모드 인코딩: UTF-8 로 시도하고 실패하면 lossy 변환한다.

### 4.4 입력과 중지

- 입력창에서 Enter 를 누르면 한 줄을 전송한다. PTY 는 `\r`, pipe 는 `\r\n` 을 붙인다. 최근 입력 50개를 위/아래 화살표로 다시 불러올 수 있다.
- **Ctrl+C 는 자식에게 바로 전달하지 않는다.**
  - 콘솔 출력 영역에 선택된 텍스트가 있으면 복사로 동작한다.
  - 선택이 없으면(입력창 포커스 포함) 확인 팝업을 띄운다: `"<name>" 서비스를 중지할까?` [예/아니요]
  - "예" 를 누르면 아래 정상 중지 절차를 실행한다.
- 정상 중지 절차 (console):
  1. `stop_command` 가 있으면 stdin 으로 전송하고 `stop_timeout_secs` 동안 종료를 기다린다
  2. 살아 있으면 Ctrl+C 를 보낸다(PTY: `\x03` 쓰기, pipe: `CREATE_NEW_PROCESS_GROUP` 으로 띄운 뒤 `GenerateConsoleCtrlEvent(CTRL_BREAK_EVENT)`) → `ctrl_c_timeout_secs` 대기
  3. 살아 있으면 Job 종료(`TerminateJobObject`)
  - pipe 모드의 Ctrl 이벤트는 호출자가 자식과 같은 콘솔에 붙어 있어야 동작한다. GUI 앱인 zsm 은 콘솔이 없으므로 `AttachConsole(pid)` → 이벤트 발송 → `FreeConsole` 방식 또는 별도 헬퍼 프로세스가 필요하다. 동작을 테스트로 확인하고, 안 되면 pipe 모드는 2단계를 건너뛰도록 하고 README 에 적는다.
- 정상 중지 절차 (gui): 해당 Job 프로세스들의 최상위 창에 `WM_CLOSE` → `stop_timeout_secs` 대기 → Job 종료.
- 각 단계는 콘솔 출력 영역에 `[zsm] ...` 형식의 시스템 줄로 남긴다.

### 4.5 GUI 프로그램 숨김

- `kind = "gui"` 는 `STARTUPINFOW.wShowWindow = SW_HIDE` + `STARTF_USESHOWWINDOW` 로 시작한다.
- 이를 무시하고 창을 띄우는 앱이 있으므로, 시작 후 10초 동안 500ms 간격으로 Job 소속 PID 의 보이는 최상위 창을 `EnumWindows` 로 찾아 숨긴다.
- "창 보이기/숨기기" 토글을 제공한다(`ShowWindow`, 보일 때는 `SetForegroundWindow`).

### 4.6 메인 창

- 왼쪽: 서비스 `ListView` (이름, 상태, PID, CPU, 메모리, 가동 시간, 재시작 횟수).
- 오른쪽: 선택한 서비스의 콘솔(`RichTextBox`, 읽기 전용, Consolas) + 입력창 + 버튼(시작, 중지, 재시작, 창 보이기[gui 전용]).
  - 출력 추가는 증분으로 한다(끝에 삽입). 줄 수가 `scrollback_lines` 를 넘으면 앞에서 잘라낸다. 사용자가 위로 스크롤해 있으면 자동 스크롤을 멈춘다.
  - UI 갱신은 100ms 타이머로 모아서 한다(출력 폭주 시 UI 멈춤 방지).
- 메뉴: 파일(서비스 추가/편집/삭제, 설정 파일 열기, 설정 다시 불러오기, 종료), 도움말(업데이트 확인, 로그 폴더 열기, 정보).
- 서비스 추가/편집 대화상자: 4.1 의 모든 필드. 실행 파일과 작업 폴더는 파일 대화상자로 고른다. 저장하면 TOML 에 기록한다(사용자 주석 보존은 하지 않아도 된다. 대신 저장 전 `config.toml.bak` 을 만든다).
- 창 닫기(X)는 트레이로 숨김이다. 첫 숨김 때 한 번만 풍선 알림으로 안내한다.

### 4.7 트레이

- `nwg::TrayNotification` 사용. 아이콘은 전체 상태에 따라 바뀐다.
  - 회색: 실행 중인 서비스 없음
  - 초록: 모든 autostart 서비스 실행 중
  - 노랑: Starting/Stopping/Backoff 가 있음
  - 빨강: Failed 또는 Invalid 가 있음
- 툴팁: `Z Service Manager — 실행 2 / 전체 3`
- 왼쪽 클릭: 메인 창 열기/숨기기. 오른쪽 클릭 팝업 메뉴: 열기, 모두 시작, 모두 중지, 업데이트 확인, 종료.
- 크래시, Failed, 업데이트 발견 시 풍선 알림.
- 종료: 실행 중인 서비스가 있으면 확인 → 모든 서비스를 4.4 절차로 병렬 중지 → 종료.
- 아이콘 파일 4종(`res/icons/*.ico`, 16/32/48 px)은 스크립트로 생성해 커밋한다. 생성 스크립트도 저장소에 둔다.

### 4.8 시작 프로그램, 단일 인스턴스, 세션 종료

- "Windows 시작 시 실행" 체크 메뉴 → `HKCU\Software\Microsoft\Windows\CurrentVersion\Run\ZServiceManager = "<설치경로>\zsm.exe" --tray`
- `--tray`: 창 없이 트레이로 시작한다. `autostart = true` 인 서비스를 실행한다.
- 두 번째 실행은 mutex 로 감지하고, 기존 인스턴스의 창을 앞으로 띄운 뒤 종료한다(등록된 window message 또는 `FindWindow`).
- `WM_QUERYENDSESSION`/`WM_ENDSESSION`(NWG raw event handler)에서 모든 서비스를 정상 중지한다. Windows 가 주는 시간이 짧으므로 `ShutdownBlockReasonCreate` 로 사유를 표시하고, 단계별 타임아웃을 최대 20초로 줄여 적용한다.

### 4.9 설치와 릴리스

- `installer/zsm.iss`:
  - per-user 설치, 관리자 권한 불필요. `AppId` 고정 GUID.
  - 시작 메뉴 바로가기, 선택 작업 "Windows 시작 시 실행"(Run 키 등록).
  - `AppMutex=Local\ZServiceManager.SingleInstance` 로 실행 중이면 대기한다.
  - 제거 시 Run 키를 삭제한다. 설정/로그 폴더는 "설정도 삭제할까?" 에 예를 누를 때만 지운다.
  - 버전은 CI 에서 `/DAppVersion=x.y.z` 로 주입한다.
  - 업데이트 모드 파라미터 `/ZSMUPDATE` 가 있으면 설치 후 `zsm.exe --tray --resume` 을 실행한다(silent 에서도).
- `.github/workflows/release.yml` (tag `v*` push):
  1. tag 버전 == `[workspace.package] version` 확인, 다르면 실패
  2. `cargo test --workspace`, `cargo build --release`
  3. Inno Setup 으로 `ZServiceManager-Setup-x.y.z.exe` 생성 (러너에 없으면 `choco install innosetup`)
  4. 포터블 `zsm-x.y.z-x86_64-pc-windows-msvc.zip`
  5. `SHA256SUMS.txt`
  6. `gh release create` 로 위 세 파일 업로드, 릴리스 노트는 자동 생성
- `.github/workflows/ci.yml` (push/PR): `cargo fmt --check`, `cargo clippy --workspace -- -D warnings`, `cargo test --workspace`.

### 4.10 자동 업데이트

- 시작 후 30초, 이후 `update_check_interval_hours` 마다 `https://api.github.com/repos/Zeliper/Z-Service-Manager/releases/latest` 를 조회한다. 메뉴의 "업데이트 확인" 은 즉시 조회한다.
- `semver` 로 현재 버전과 비교한다. prerelease, draft 는 무시한다.
- 새 버전이면 풍선 알림을 띄우고, 메뉴에 "업데이트 설치 (vX.Y.Z)" 를 활성화한다.
- 설치 흐름:
  1. 확인 팝업: 실행 중인 서비스가 중지된다는 것을 명시
  2. 설치 프로그램과 `SHA256SUMS.txt` 를 `%TEMP%` 에 다운로드 → 해시 불일치면 중단하고 알림
  3. 실행 중이던 서비스 id 목록을 `%LOCALAPPDATA%\ZServiceManager\resume.json` 에 기록
  4. 모든 서비스 정상 중지
  5. `Setup.exe /VERYSILENT /SUPPRESSMSGBOXES /NORESTART /ZSMUPDATE` 실행 후 앱 종료
  6. 새 버전이 `--resume` 으로 시작하면 `resume.json` 의 서비스를 실행하고 파일을 삭제
- 포터블(zip) 실행으로 감지되면(설치 경로가 아니면) 설치하지 않고 릴리스 페이지 링크만 연다.
- 네트워크 오류는 앱 로그에만 남기고 사용자를 방해하지 않는다(수동 확인일 때만 팝업).

---

## 5. 마일스톤

각 마일스톤 끝에서 완료 기준을 직접 실행해 확인하고, 실제 출력을 보고한 뒤 커밋한다.

### M0 — 환경과 저장소

1. 개발 환경 확인 및 설치 (이 PC에는 작성 시점에 `cargo` 가 없다):
   - `winget install Rustlang.Rustup` → `rustup default stable-x86_64-pc-windows-msvc`
   - MSVC 빌드 도구가 없으면 `winget install Microsoft.VisualStudio.2022.BuildTools` (VC++ 워크로드). 설치가 사용자 개입을 요구하면 멈추고 보고한다.
   - `winget install JRSoftware.InnoSetup`
2. workspace 골격, `.gitignore`, `LICENSE`(MIT, Zeliper), `README.md`(개요 + 빌드 방법) 작성.
3. `git init` → 첫 커밋.
4. 비밀정보가 커밋에 없는지 확인한 뒤 `gh repo create Zeliper/Z-Service-Manager --public --source=. --remote=origin --push`.
5. `ci.yml` 추가.

완료 기준: `cargo build --workspace` 성공, GitHub 에 public 저장소가 생기고 CI 가 초록.

### M1 — `zsm-core` 프로세스 감독

- 설정 로드/검증, 감독 스레드, 상태 머신, Job Object, ConPTY/pipe, 출력 처리기, 입력, 정상 중지 절차, 재시작 backoff, 지표, 로그 파일.
- `zsm-testchild`: 인자로 동작을 바꾸는 테스트용 콘솔 프로그램. 주기적 출력, stdin 에코, `/stop` 수신 시 종료 코드 0, `crash` 수신 시 종료 코드 1, `ignore-stop` 모드, `\r` 진행률 출력, ANSI 색상 출력, 자식 프로세스 생성.

완료 기준 (`cargo test -p zsm-core` 로 자동 검증):
- PTY 와 pipe 양쪽에서 출력 수신과 입력 에코
- `/stop` → 정상 종료, `ignore-stop` → Ctrl+C 단계 → 강제 종료 단계까지 진행
- `crash` → backoff 후 재시작, `restart_max` 초과 시 Failed
- 감독자 drop 시 testchild 와 그 손자 프로세스가 모두 사라짐
- `\r` 진행률이 한 줄로 합쳐지고 ANSI 가 로그 파일에서 제거됨

### M2 — GUI 와 트레이

- 4.6, 4.7, 4.8 전부. 서비스 편집 대화상자 포함.

완료 기준 (수동 검증 + 스크린샷 또는 관찰 결과 보고):
- `zsm-testchild` 를 서비스로 등록하고 시작/입력/Ctrl+C 팝업/중지/재시작
- 창 닫기 → 트레이 유지, 트레이 아이콘 색이 상태에 따라 바뀜
- `--tray` 실행, 중복 실행 시 기존 창 활성화
- Vintage Story 서버를 실제로 등록해 시작 → 콘솔 명령 입력 → Ctrl+C 팝업 → `/stop` 으로 저장 후 종료되는 것 확인 (서버 데이터가 실제로 저장됐는지 로그로 확인)

### M3 — GUI 프로그램 지원

- 4.5. 메모장(`notepad.exe`)으로 검증.

완료 기준: 숨김 시작, 보이기/숨기기 토글, `WM_CLOSE` 중지, 앱 종료 시 정리.

### M4 — 설치, 릴리스, 자동 업데이트

- 4.9, 4.10. 콘솔 출력 SGR 색상 표시는 이 시점에 시간이 남으면 구현한다.

완료 기준:
- 로컬에서 ISCC 로 설치 프로그램 생성 → 설치 → 실행 → 제거까지 확인
- `v0.1.0` tag push → release.yml 이 세 파일을 올린 릴리스 생성
- `v0.1.1` 을 릴리스한 뒤, 설치된 0.1.0 이 업데이트를 감지하고 → 해시 검증 → silent 설치 → `--resume` 으로 서비스가 다시 실행되는 것 확인
- tag 를 push 하기 전에 사용자에게 확인을 받는다(공개 릴리스 생성이므로)

---

## 6. 작업 규칙

- 커밋은 마일스톤 단위 또는 그보다 작은 의미 단위로 한다. push 는 M0 이후 자유. tag push 와 release 생성은 사용자 확인 후.
- `unsafe` Win32 호출은 `zsm-core::win` 같은 한 모듈에 모으고, 안전한 래퍼만 밖으로 내보낸다.
- UI 스레드에서 블로킹 I/O 금지. 업데이트 다운로드, 프로세스 대기, 파일 쓰기는 워커 스레드에서.
- 패닉은 앱 로그(`%LOCALAPPDATA%\ZServiceManager\zsm.log`)에 남기고, 가능한 한 서비스 감독은 계속한다.
- README 에 포함: 설치, 설정 예시(Vintage Story), Ctrl+C 동작, 알려진 제약(ConPTY race, TUI 미지원, 코드 서명 없음으로 인한 SmartScreen 경고).
- 확인하지 못한 동작은 완료라고 쓰지 않는다. 막히면 시도한 것과 실제 에러를 보고한다.
