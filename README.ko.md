# Bbiribarabu

Bbiribarabu는 Git 레포지토리에서 브랜치별로 로그를 남기는 기록 도구입니다. 터미널이 포함된 분할 TUI와 음성 로그(Whisper) 기능을 제공합니다.

## 주요 기능
- 브랜치별 로그 저장 (`.bbiribarabu/logs`)
- 쉘 + 로그 리스트 + 입력 패널로 구성된 TUI
- CLI로 로그 추가/조회/음성 기록
- 필요한 경우 Whisper 모델을 자동 다운로드

## 다운로드 및 설치

### 사전 빌드 바이너리 (권장)

GitHub Releases에서 **Linux x86_64 실행 파일**을 다운로드할 수 있습니다.

https://github.com/hefxpzwk/Bbiribarabu/releases

### 명령어로 빠르게 설치하려면:

```bash
curl -L -o Bbiribarabu \
https://github.com/hefxpzwk/Bbiribarabu/releases/download/v1.0.0/Bbiribarabu

chmod +x Bbiribarabu
./Bbiribarabu
```
## 요구 사항
- Rust toolchain (edition 2024)
- Git 레포지토리 내부에서 실행
- 음성 기능 사용 시 마이크 필요
- 첫 음성 사용 시 네트워크 필요 (또는 `WHISPER_MODEL` 지정)

## 빠른 시작
### TUI
```bash
cargo run
```

### CLI
```bash
cargo run -- add "플레이키 테스트 수정"
cargo run -- list
cargo run -- voice --seconds 5
```

## TUI 조작키
- `Esc`: 터미널/로그 패널 포커스 전환
- 로그 패널 (일반 모드)
  - `i`: 새 로그 입력
  - `e`: 선택한 로그 편집
  - `d`: 선택한 로그 삭제 (`y`/`n` 확인)
  - `/`: 로그 검색
  - `v`: 음성 로그 (다시 `v` 누르면 종료, 그 외 키는 취소)
  - `q`: 종료
  - 방향키 / PageUp / PageDown: 선택 이동
  - Left / Right / Home: 로그 가로 스크롤

## 데이터 저장 위치
- `.bbiribarabu/logs/<branch>.json`에 브랜치별로 저장됩니다
- 브랜치명에 `/`가 있으면 `__`로 치환됩니다

## 음성 모델
- Whisper base 모델을 `models/ggml-base.bin`에 다운로드합니다
- `WHISPER_MODEL=/path/to/ggml-base.bin`로 경로를 지정할 수 있습니다

## 라이선스
MIT. `LICENSE` 참고.
