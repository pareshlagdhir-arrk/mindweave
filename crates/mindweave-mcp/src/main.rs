//! `mindweave-mcp`: the MCP server binary. It reads JSON-RPC messages
//! from stdin and writes replies to stdout; logs go to stderr.

use std::io::{stdin, stdout};
use std::path::PathBuf;

use mindweave_mcp::{Context, Server};

/// Environment variable naming the user libraries folder.
const LIBRARIES_ENV: &str = "MINDWEAVE_LIBRARIES_DIR";

const HELP: &str = "mindweave-mcp: MCP server for Mindweave documents (stdio)

Usage: mindweave-mcp [--libraries-dir DIR]

Options:
  --libraries-dir DIR  Folder of user shape libraries (default: the app's,
                       or $MINDWEAVE_LIBRARIES_DIR)
  -h, --help           Show this help
  -V, --version        Show the version

Configure it in an MCP client as a stdio server running this binary.";

/// The libraries folder from the arguments or the environment; exits on
/// `--help`, `--version` or a bad argument.
fn libraries_dir() -> Option<PathBuf> {
    let mut args = std::env::args().skip(1);
    let mut dir = std::env::var_os(LIBRARIES_ENV).map(PathBuf::from);
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
            other => match other.strip_prefix("--libraries-dir=") {
                Some(d) => dir = Some(PathBuf::from(d)),
                None => {
                    eprintln!("Unknown argument `{other}`.\n\n{HELP}");
                    std::process::exit(2);
                }
            },
        }
    }
    dir
}

fn main() {
    let dir = libraries_dir();
    let (ctx, notices) = Context::new(dir.as_deref());
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
