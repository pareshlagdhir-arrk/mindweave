//! `mindweave-mcp`: the MCP server binary. It reads JSON-RPC messages
//! from stdin and writes replies to stdout; logs go to stderr.

use std::io::{stdin, stdout};
use std::path::PathBuf;

use mindweave_mcp::{Context, Server};

/// Environment variable naming the user libraries folder.
const LIBRARIES_ENV: &str = "MINDWEAVE_LIBRARIES_DIR";
/// Environment variable naming the icon sets folder.
const ICONS_ENV: &str = "MINDWEAVE_ICONS_DIR";

const HELP: &str = "mindweave-mcp: MCP server for Mindweave documents (stdio)

Usage: mindweave-mcp [--libraries-dir DIR] [--icons-dir DIR]

Options:
  --libraries-dir DIR  Folder of user shape libraries (default: the app's,
                       or $MINDWEAVE_LIBRARIES_DIR)
  --icons-dir DIR      Folder of icon sets (default: the app's, or
                       $MINDWEAVE_ICONS_DIR)
  -h, --help           Show this help
  -V, --version        Show the version

Configure it in an MCP client as a stdio server running this binary.";

/// The libraries and icons folders from the arguments or the
/// environment; exits on `--help`, `--version` or a bad argument.
fn folders() -> (Option<PathBuf>, Option<PathBuf>) {
    let mut args = std::env::args().skip(1);
    let mut dir = std::env::var_os(LIBRARIES_ENV).map(PathBuf::from);
    let mut icons = std::env::var_os(ICONS_ENV).map(PathBuf::from);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "-h" | "--help" => {
                eprintln!("{HELP}");
                std::process::exit(0);
            }
            "-V" | "--version" => {
                eprintln!("mindweave-mcp {}", env!("CARGO_PKG_VERSION"));
                std::process::exit(0);
            }
            "--libraries-dir" => match args.next() {
                Some(d) => dir = Some(PathBuf::from(d)),
                None => {
                    eprintln!("--libraries-dir needs a folder.");
                    std::process::exit(2);
                }
            },
            "--icons-dir" => match args.next() {
                Some(d) => icons = Some(PathBuf::from(d)),
                None => {
                    eprintln!("--icons-dir needs a folder.");
                    std::process::exit(2);
                }
            },
            other => {
                if let Some(d) = other.strip_prefix("--libraries-dir=") {
                    dir = Some(PathBuf::from(d));
                } else if let Some(d) = other.strip_prefix("--icons-dir=") {
                    icons = Some(PathBuf::from(d));
                } else {
                    eprintln!("Unknown argument `{other}`.\n\n{HELP}");
                    std::process::exit(2);
                }
            }
        }
    }
    (dir, icons)
}

fn main() {
    let (dir, icons) = folders();
    let (ctx, notices) = Context::with_icons(dir.as_deref(), icons.as_deref());
    for notice in notices {
        eprintln!("mindweave-mcp: {notice}");
    }
    eprintln!("mindweave-mcp {} ready on stdio", env!("CARGO_PKG_VERSION"));
    let mut server = Server::new(ctx);
    if let Err(e) = server.run(stdin().lock(), stdout().lock()) {
        eprintln!("mindweave-mcp: {e}");
        std::process::exit(1);
    }
}
