# TokenBar

Menu-bar / system-tray monitor for **Cursor**, **Claude**, and **OpenAI** usage.

The tray title looks like `R42% C72% O1.2k`:

| Prefix | Product | Why |
| --- | --- | --- |
| `R` | Cursor | The **R** in Cu**R**sor, so it does not share `C` with Claude |
| `C` | Claude | Five-hour OAuth utilization when the API answers |
| `O` | OpenAI / Codex | Quota percent from Codex, otherwise a compact token count (`1.2k`) |

A percent means quota **used**. A compact number means tokens. Click the icon to open a GPUI panel with **Refresh**, **Settings**, and **Quit**.

The UI is [gpui-kit](https://github.com/longbridge/gpui-kit) plus [gpui-tray](https://github.com/domenkozar/gpui-tray) (`gpui-kit` feature). There is no Swift and no Liquid Glass shell. macOS builds are accessory apps (`LSUIElement`, no Dock icon).

TokenBar is **Apache-2.0**. See `LICENSE` and `NOTICE`. Provider ideas are reimplemented from the MIT-licensed [Syrtis](https://github.com/Nanako0129/syrtis) project; that source is not copied.

## Build

Stable Rust (1.85+) is enough. The repo pins the stable toolchain in `rust-toolchain.toml`.

```bash
cargo test -p tokenbar-core
cargo build -p tokenbar --release
./target/release/tokenbar
```

Linux needs the libraries listed in `packaging/linux/apt.txt` (gpui links X11, Wayland, and libxkbcommon):

```bash
sudo apt-get install -y $(cat packaging/linux/apt.txt)
```

macOS needs the Xcode command-line tools. Windows needs the MSVC build tools that come with the GitHub `windows-latest` image or a Visual Studio Build Tools install.

## Run

```bash
./target/release/tokenbar            # tray
./target/release/tokenbar --check    # JSON snapshot, no window
./target/release/tokenbar --help
```

`--check` exits before GPUI starts, so it works over SSH and in CI.

```bash
TOKENBAR_OFFLINE=1 ./target/release/tokenbar --check
```

`TOKENBAR_OFFLINE=1` skips the network and local credentials and prints a stub whose title is exactly `R42% C72% O1.2k`.

## Configure

Settings in the panel reads the clipboard. Nothing you paste is written into the JSON snapshot.

| Secret | What to paste | Stored |
| --- | --- | --- |
| Cursor session | `WorkosCursorSessionToken` value, `user_…::jwt`, or the access token | macOS Keychain via `keyring`; elsewhere a `0600` file |
| Claude OAuth | Claude Code access token (`sk-ant-oat…`) | same |
| OpenAI key | `sk-…` (optional organization usage) | same |

On macOS the service name is `TokenBar`. If the keychain refuses the write, TokenBar falls back to the same `0600` file. Set `TOKENBAR_SECRETS_PATH` to force that file on any OS.

Auto-detected sources, used when you have not pasted a replacement:

- **Cursor.** `state.vscdb` (`cursorAuth/accessToken`, and `cursorAuth/cachedScopedProfile` when the token is opaque). The cookie sent to Cursor is `WorkosCursorSessionToken`. Quota percent is `GET https://cursor.com/api/usage-summary`. Token totals are `POST https://cursor.com/api/dashboard/get-filtered-usage-events` (epoch-millisecond `startDate` / `endDate` strings, `Origin: https://cursor.com`). Override with `CURSOR_SESSION_TOKEN`.
- **Claude.** `~/.claude/.credentials.json` (`claudeAiOauth.accessToken`), or `CLAUDE_CONFIG_DIR`. On macOS, the `Claude Code-credentials` keychain item is next. Live quota is `GET https://api.anthropic.com/api/oauth/usage` with `anthropic-beta: oauth-2025-04-20`. Claude Code JSONL under `projects/` is summed.
- **OpenAI / Codex.** `~/.codex/auth.json` or `CODEX_HOME` (`tokens.access_token`, optional `account_id`). Subscription quota is `GET https://chatgpt.com/backend-api/wham/usage`. An `sk-` key, pasted or `OPENAI_API_KEY` in `auth.json`, calls `GET https://api.openai.com/v1/organization/usage/completions`. Codex `sessions/**/*.jsonl` (and `archived_sessions`) contribute tokens; `last_token_usage` wins over cumulative totals.

The tray refreshes about every five minutes. A failed provider keeps the others; the row's `error` field explains the miss. Windows shells ignore the native tray title (a gpui-tray limitation). The tooltip, the panel, and `--check` still show the same string.

## macOS app bundle

```bash
./packaging/macos/package.sh
open dist/tokenbar-aarch64-apple-darwin/TokenBar.app
```

The bundle's `Info.plist` sets `LSUIElement` to true. On launch the process also calls AppKit `-[NSApplication setActivationPolicy:]` with `NSApplicationActivationPolicyAccessory`, which keeps a menu-bar app out of the Dock while the panel can still open. gpui-kit 0.6 does not expose that switch, so TokenBar calls AppKit directly.

### Intel Mac (`x86_64-apple-darwin`)

On an Intel Mac, a normal `cargo build --release` or `./packaging/macos/package.sh` (no `TARGET`) produces an `x86_64` binary. The script names the folder `dist/tokenbar-x86_64-apple-darwin`.

To cross-compile from Apple Silicon:

```bash
rustup target add x86_64-apple-darwin
TARGET=x86_64-apple-darwin ./packaging/macos/package.sh
```

The Release workflow does that cross build. GitHub no longer provides the Intel-hosted `macos-13` runner, so the `x86_64-apple-darwin` asset is built on `macos-latest` with the universal macOS SDK. The asset name is `tokenbar-x86_64-apple-darwin.tar.gz`.

## CI

| Workflow | When | What |
| --- | --- | --- |
| `CI` | pull request and push to `main` | `cargo test -p tokenbar-core` on Ubuntu, macOS, and Windows. `cargo check -p tokenbar` on each. Ubuntu also builds the binary and runs `TOKENBAR_OFFLINE=1 tokenbar --check`. |
| `Release` | tag `v*` or **Run workflow** | Release binaries for `aarch64-apple-darwin`, `x86_64-apple-darwin`, `x86_64-unknown-linux-gnu`, and `x86_64-pc-windows-msvc`. Linux and the other jobs run the offline `--check` before upload. |

Release asset names:

- `tokenbar-aarch64-apple-darwin.tar.gz` (contains `TokenBar.app`)
- `tokenbar-x86_64-apple-darwin.tar.gz` (Intel Mac build, cross-compiled)
- `tokenbar-x86_64-unknown-linux-gnu.tar.gz`
- `tokenbar-x86_64-pc-windows-msvc.zip`

Each archive includes `LICENSE`, `NOTICE`, and this README.

## Layout

```
crates/tokenbar/        gpui-kit + gpui-tray app
crates/tokenbar-core/   providers, secret store, --check snapshot
packaging/macos/        LSUIElement .app
packaging/linux/        apt packages for a gpui build
```

## Privacy

Usage calls go only to Cursor, Anthropic, and OpenAI, using a credential you already have. TokenBar does not add an account or a telemetry endpoint. `--check` JSON includes percents, token totals, and error strings. It does not include secrets.

---

# TokenBar（中文）

菜单栏 / 系统托盘里的 **Cursor**、**Claude**、**OpenAI** 用量监视器。

托盘标题形如 `R42% C72% O1.2k`：

| 前缀 | 产品 | 说明 |
| --- | --- | --- |
| `R` | Cursor | 取 Cu**R**sor 里的 R，避免和 Claude 的 `C` 冲突 |
| `C` | Claude | 有 OAuth 用量时显示 5 小时窗口已用百分比 |
| `O` | OpenAI / Codex | 有 Codex 额度时显示百分比，否则显示紧凑 token 数（`1.2k`） |

百分号表示**已用**额度。紧凑数字表示 token。点击图标打开 GPUI 面板，里面有 **Refresh**（刷新）、**Settings**（设置）、**Quit**（退出）。

界面使用 [gpui-kit](https://github.com/longbridge/gpui-kit) 和 [gpui-tray](https://github.com/domenkozar/gpui-tray)（`gpui-kit` feature）。没有 Swift，也没有 Liquid Glass。macOS 以附件应用运行（`LSUIElement`，不出现在程序坞）。

本项目许可证是 **Apache-2.0**，见 `LICENSE` 与 `NOTICE`。用量读取的思路来自 MIT 许可的 [Syrtis](https://github.com/Nanako0129/syrtis)，实现是重写的，没有复制其源码。

## 构建

需要稳定版 Rust（1.85 或更新）。`rust-toolchain.toml` 已指定 stable。

```bash
cargo test -p tokenbar-core
cargo build -p tokenbar --release
./target/release/tokenbar
```

Linux 需要 `packaging/linux/apt.txt` 里的库（gpui 会链接 X11、Wayland 和 libxkbcommon）：

```bash
sudo apt-get install -y $(cat packaging/linux/apt.txt)
```

macOS 需要 Xcode 命令行工具。Windows 需要 MSVC 构建工具。

## 运行

```bash
./target/release/tokenbar            # 托盘
./target/release/tokenbar --check    # 打印 JSON，不开窗口
./target/release/tokenbar --help
```

`--check` 在启动 GPUI 之前就退出，可以在无显示器的环境里用。

```bash
TOKENBAR_OFFLINE=1 ./target/release/tokenbar --check
```

`TOKENBAR_OFFLINE=1` 不访问网络，也不读本机凭据，标题固定为 `R42% C72% O1.2k`。

## 配置

在面板的 Settings 里从剪贴板粘贴。粘贴的内容不会出现在 JSON 快照里。

| 密钥 | 粘贴内容 | 保存位置 |
| --- | --- | --- |
| Cursor session | `WorkosCursorSessionToken` 的值、`user_…::jwt`，或 access token | macOS 用 `keyring` 写入登录钥匙串；其他系统写入权限 `0600` 的文件 |
| Claude OAuth | Claude Code 的 access token（`sk-ant-oat…`） | 同上 |
| OpenAI key | `sk-…`（可选的组织用量） | 同上 |

macOS 上钥匙串服务名是 `TokenBar`。钥匙串写入失败时会退回到同一个 `0600` 文件。设置 `TOKENBAR_SECRETS_PATH` 可以在任何系统上强制使用该文件。

没有粘贴时，会自动读取这些位置：

- **Cursor。** `state.vscdb` 里的 `cursorAuth/accessToken`；token 不是 JWT 时再读 `cursorAuth/cachedScopedProfile`。请求带 `WorkosCursorSessionToken` cookie。额度百分比来自 `GET https://cursor.com/api/usage-summary`。token 合计来自 `POST https://cursor.com/api/dashboard/get-filtered-usage-events`（`startDate` / `endDate` 是毫秒时间戳字符串，并带 `Origin: https://cursor.com`）。可用 `CURSOR_SESSION_TOKEN` 覆盖。
- **Claude。** `~/.claude/.credentials.json` 的 `claudeAiOauth.accessToken`，或 `CLAUDE_CONFIG_DIR`。macOS 上接着尝试钥匙串项 `Claude Code-credentials`。实时额度是 `GET https://api.anthropic.com/api/oauth/usage`，请求头 `anthropic-beta: oauth-2025-04-20`。`projects/` 下的 Claude Code JSONL 会累加 token。
- **OpenAI / Codex。** `~/.codex/auth.json` 或 `CODEX_HOME`（`tokens.access_token`，可选 `account_id`）。订阅额度是 `GET https://chatgpt.com/backend-api/wham/usage`。粘贴的或 `auth.json` 里的 `sk-` 密钥会请求 `GET https://api.openai.com/v1/organization/usage/completions`。Codex 的 `sessions/**/*.jsonl` 和 `archived_sessions` 提供 token；优先使用 `last_token_usage`，而不是把累计总量重复相加。

托盘大约每五分钟刷新一次。某一个提供方失败不会清掉其他提供方；对应行的 `error` 会写明原因。Windows 托盘不显示标题（gpui-tray 的平台限制），提示文字、面板和 `--check` 仍是同一串。

## macOS 应用包

```bash
./packaging/macos/package.sh
open dist/tokenbar-aarch64-apple-darwin/TokenBar.app
```

`Info.plist` 把 `LSUIElement` 设为 true。启动时进程还会调用 AppKit 的 `-[NSApplication setActivationPolicy:]`，策略为 `NSApplicationActivationPolicyAccessory`，这样菜单栏应用不会进程序坞，但仍然可以打开面板。gpui-kit 0.6 没有暴露这个开关，所以 TokenBar 直接调用 AppKit。

### Intel Mac（`x86_64-apple-darwin`）

在 Intel Mac 上直接 `cargo build --release` 或运行不带 `TARGET` 的 `./packaging/macos/package.sh`，得到的就是 `x86_64` 二进制。输出目录是 `dist/tokenbar-x86_64-apple-darwin`。

在 Apple Silicon 上交叉编译：

```bash
rustup target add x86_64-apple-darwin
TARGET=x86_64-apple-darwin ./packaging/macos/package.sh
```

Release 工作流就是这样交叉编译的。GitHub 已经撤掉 Intel 托管的 `macos-13` runner，所以 `x86_64-apple-darwin` 构建跑在 `macos-latest` 上，使用通用 macOS SDK。发布文件名是 `tokenbar-x86_64-apple-darwin.tar.gz`。

## CI

| 工作流 | 触发 | 内容 |
| --- | --- | --- |
| `CI` | PR，以及推送到 `main` | 在 Ubuntu、macOS、Windows 上运行 `cargo test -p tokenbar-core`，并 `cargo check -p tokenbar`。Ubuntu 还会编译二进制并执行 `TOKENBAR_OFFLINE=1 tokenbar --check`。 |
| `Release` | 标签 `v*`，或手动 **Run workflow** | 产出 `aarch64-apple-darwin`、`x86_64-apple-darwin`、`x86_64-unknown-linux-gnu`、`x86_64-pc-windows-msvc` 四个发布包。上传前会跑离线 `--check`。 |

发布资产名称：

- `tokenbar-aarch64-apple-darwin.tar.gz`（内含 `TokenBar.app`）
- `tokenbar-x86_64-apple-darwin.tar.gz`（Intel Mac，交叉编译）
- `tokenbar-x86_64-unknown-linux-gnu.tar.gz`
- `tokenbar-x86_64-pc-windows-msvc.zip`

每个压缩包都带有 `LICENSE`、`NOTICE` 和本 README。

## 目录

```
crates/tokenbar/        gpui-kit + gpui-tray 应用
crates/tokenbar-core/   提供方、密钥存储、--check 快照
packaging/macos/        LSUIElement .app
packaging/linux/        编译 gpui 所需的 apt 包
```

## 隐私

用量请求只发往 Cursor、Anthropic 和 OpenAI，使用的是你本机已有的凭据。TokenBar 没有自己的账号，也没有遥测地址。`--check` 的 JSON 只有百分比、token 合计和错误说明，不含密钥。
