# Z Service Manager

콘솔 프로그램과 GUI 프로그램을 자식 프로세스로 띄워 관리하는 Windows 트레이 앱.

- 콘솔 프로그램의 출력을 실시간으로 보여주고 stdin 으로 명령을 보낼 수 있다.
- GUI 프로그램은 창을 숨긴 채 실행하고 필요할 때 다시 보이게 한다.
- 상태, PID, CPU, 메모리, 가동 시간, 재시작 횟수를 추적하고 크래시 시 정책에 따라 재시작한다.
- Windows 로그온 시 트레이로 자동 시작한다.

## 빌드

요구 사항: Rust stable (`x86_64-pc-windows-msvc`), Visual Studio Build Tools (VC++ 워크로드).

```
cargo build --workspace
cargo test --workspace
cargo build --release
```

결과물: `target\release\zsm.exe`

## 라이선스

MIT
