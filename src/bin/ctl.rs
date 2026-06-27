//! `ctl` — aurum-control's standalone controller tester. Built only with
//! `--features harness` (it pulls in MIDI I/O + a web server the lib never needs).
//!
//! Usage:
//!   ctl list          list MIDI input ports
//!   ctl ui [port]     run the web MIDI monitor (default port 7700)

fn main() {
    let args: Vec<String> = std::env::args().collect();
    match args.get(1).map(String::as_str) {
        Some("list") => midi::harness::list_ports(),
        None | Some("ui") => {
            let port = args.get(2).and_then(|s| s.parse().ok()).unwrap_or(7700);
            midi::harness::run_ui(port);
        }
        Some(other) => {
            eprintln!("unknown command: {other}\nusage: ctl [list | ui [port]]");
            std::process::exit(2);
        }
    }
}
