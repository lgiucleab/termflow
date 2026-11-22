//! Dev server: `cargo run -p mock-provider -- <fixture> [port]` serves one fixture for every
//! request (any path), paced like a real stream. Point a Custom provider at it in the app.
#![allow(clippy::print_stdout)] // CLI output

use std::time::Duration;

use mock_provider::{MockProvider, Script};

#[tokio::main]
async fn main() -> std::io::Result<()> {
    let mut args = std::env::args().skip(1);
    let Some(path) = args.next() else {
        eprintln!("usage: mock-provider <fixture.sse|.ndjson|.json>");
        std::process::exit(2);
    };
    let mock = MockProvider::start().await?;
    mock.set_default(Script::fixture(&path)?.paced(Duration::from_millis(40)));
    println!("mock provider serving {path} at {}  (Ctrl+C to stop)", mock.url());
    tokio::signal::ctrl_c().await?;
    Ok(())
}
