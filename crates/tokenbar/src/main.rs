//! TokenBar menu-bar application.
//!
//! `tokenbar --check` prints a JSON snapshot and never opens a window, so it
//! runs on headless CI. The tray UI starts only when no subcommand is given.

mod ui;

const HELP: &str = "\
TokenBar — Cursor, Claude, and OpenAI usage in the menu bar

USAGE:
    tokenbar                 Start the tray app
    tokenbar --check         Print a JSON usage snapshot and exit
    tokenbar --help          Show this help
    tokenbar --version       Print the version

ENVIRONMENT:
    TOKENBAR_OFFLINE=1       Skip the network and local credentials; emit the
                             stub title R42% C72% O1.2k
    TOKENBAR_SECRETS_PATH    Force the 0600 JSON secret file (any OS)
    CURSOR_SESSION_TOKEN     Override the Cursor session cookie
    CLAUDE_CONFIG_DIR        Claude Code config directory (default ~/.claude)
    CODEX_HOME               Codex home (default ~/.codex)

The tray title looks like R42% C72% O1.2k. Cursor is R (the R in CuRsor) so
it does not share C with Claude. A percent is quota used; a compact number
is a token total.

On macOS, tokens you paste are stored in the login keychain. Other platforms
use a mode 0600 file under the config directory. See README.md.
";

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.iter().any(|arg| arg == "--help" || arg == "-h") {
        print!("{HELP}");
        return;
    }
    if args.iter().any(|arg| arg == "--version" || arg == "-V") {
        println!("tokenbar {}", env!("CARGO_PKG_VERSION"));
        return;
    }
    if args == ["--check"] {
        match tokenbar_core::check_report() {
            Ok(json) => {
                println!("{json}");
            }
            Err(err) => {
                eprintln!("tokenbar --check: {err}");
                std::process::exit(1);
            }
        }
        return;
    }
    if !args.is_empty() {
        eprintln!("unrecognized arguments: {}", args.join(" "));
        eprintln!("try `tokenbar --help`");
        std::process::exit(2);
    }
    ui::run();
}
