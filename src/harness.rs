//! Standalone tester harness (only built with `--features harness`): a tiny local
//! web app for plugging in a controller and seeing exactly what it sends. The
//! pure lib does the decoding ([`crate::MonitorEvent`]); this module adds the MIDI
//! I/O ([`midir`]) and the web server ([`tiny_http`]) the AURUM app never needs.
//!
//! Server→browser is plain polling (`GET /api/events?since=N`): simple, robust,
//! and ~100 ms is imperceptible when you're watching messages scroll.

use crate::MonitorEvent;
use std::io::Cursor;
use std::sync::{Arc, Mutex};

/// Most recent events kept for polling clients (a spinning knob is bursty).
const LOG_CAP: usize = 2000;

/// Shared event log: a monotonic `seq` per event so a polling client can ask for
/// "everything after seq N" and never miss or double-count.
#[derive(Default)]
struct Log {
    events: Vec<(u64, MonitorEvent)>,
    next_seq: u64,
}

impl Log {
    fn push(&mut self, ev: MonitorEvent) {
        let seq = self.next_seq;
        self.next_seq += 1;
        self.events.push((seq, ev));
        let overflow = self.events.len().saturating_sub(LOG_CAP);
        if overflow > 0 {
            self.events.drain(0..overflow);
        }
    }
    /// `{ "events": [ {seq, kind, channel, data1, data2, raw}... ], "next": M }`
    fn since_json(&self, since: u64) -> String {
        let items: Vec<String> = self
            .events
            .iter()
            .filter(|(seq, _)| *seq >= since)
            .map(|(seq, ev)| {
                let body = serde_json::to_string(ev).unwrap_or_else(|_| "{}".into());
                format!("{{\"seq\":{seq},\"event\":{body}}}")
            })
            .collect();
        format!(
            "{{\"events\":[{}],\"next\":{}}}",
            items.join(","),
            self.next_seq
        )
    }
}

/// Print the available MIDI input ports (the `list` subcommand).
pub fn list_ports() {
    match midir::MidiInput::new("aurum-ctl-list") {
        Ok(mi) => {
            let ports = mi.ports();
            if ports.is_empty() {
                println!("No MIDI input ports found.");
            }
            for (i, p) in ports.iter().enumerate() {
                let name = mi.port_name(p).unwrap_or_else(|_| "<unknown>".into());
                println!("{i}: {name}");
            }
        }
        Err(e) => eprintln!("MIDI init failed: {e}"),
    }
}

/// Run the tester web UI on `127.0.0.1:<port>` and open the browser.
pub fn run_ui(port: u16) {
    let log: Arc<Mutex<Log>> = Arc::new(Mutex::new(Log::default()));
    // Hold the active input connection so its callback keeps firing; replaced on
    // each /api/connect.
    let conn: Arc<Mutex<Option<midir::MidiInputConnection<()>>>> = Arc::new(Mutex::new(None));

    let addr = format!("127.0.0.1:{port}");
    let server = match tiny_http::Server::http(&addr) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("could not bind {addr}: {e}");
            std::process::exit(1);
        }
    };
    let url = format!("http://{addr}/");
    println!("aurum-control tester UI → {url}");
    open_browser(&url);

    for req in server.incoming_requests() {
        let url = req.url().to_string();
        let (path, query) = url.split_once('?').unwrap_or((url.as_str(), ""));
        match path {
            "/" => respond_html(req, INDEX_HTML),
            "/api/ports" => respond_json(req, &ports_json()),
            "/api/events" => {
                let since = query_value(query, "since")
                    .and_then(|v| v.parse().ok())
                    .unwrap_or(0);
                let body = log.lock().unwrap().since_json(since);
                respond_json(req, &body);
            }
            "/api/connect" => {
                let idx = query_value(query, "port").and_then(|v| v.parse::<usize>().ok());
                let body = connect_port(idx, &log, &conn);
                respond_json(req, &body);
            }
            _ => {
                let _ = req.respond(tiny_http::Response::empty(404));
            }
        }
    }
}

/// (Re)open MIDI input port `idx`, wiring its callback to push decoded events into
/// the shared log. Returns a small JSON status the page shows.
fn connect_port(
    idx: Option<usize>,
    log: &Arc<Mutex<Log>>,
    conn: &Arc<Mutex<Option<midir::MidiInputConnection<()>>>>,
) -> String {
    let Some(idx) = idx else {
        return "{\"ok\":false,\"error\":\"missing port index\"}".into();
    };
    let mi = match midir::MidiInput::new("aurum-ctl-in") {
        Ok(mi) => mi,
        Err(e) => return format!("{{\"ok\":false,\"error\":\"{}\"}}", esc(&e.to_string())),
    };
    let ports = mi.ports();
    let Some(p) = ports.get(idx) else {
        return "{\"ok\":false,\"error\":\"port index out of range\"}".into();
    };
    let name = mi.port_name(p).unwrap_or_else(|_| "<unknown>".into());
    let log_cb = Arc::clone(log);
    match mi.connect(
        p,
        "aurum-ctl-in",
        move |_stamp, bytes, _| {
            log_cb.lock().unwrap().push(MonitorEvent::from_midi(bytes));
        },
        (),
    ) {
        Ok(c) => {
            *conn.lock().unwrap() = Some(c); // drop any previous connection
            format!("{{\"ok\":true,\"name\":\"{}\"}}", esc(&name))
        }
        Err(e) => format!("{{\"ok\":false,\"error\":\"{}\"}}", esc(&e.to_string())),
    }
}

fn ports_json() -> String {
    let names = midir::MidiInput::new("aurum-ctl-ports")
        .map(|mi| {
            mi.ports()
                .iter()
                .map(|p| mi.port_name(p).unwrap_or_else(|_| "<unknown>".into()))
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    let items: Vec<String> = names.iter().map(|n| format!("\"{}\"", esc(n))).collect();
    format!("[{}]", items.join(","))
}

fn query_value<'a>(query: &'a str, key: &str) -> Option<&'a str> {
    query.split('&').find_map(|kv| {
        kv.split_once('=')
            .filter(|(k, _)| *k == key)
            .map(|(_, v)| v)
    })
}

/// Minimal JSON string escaping for the few values we interpolate (port names,
/// error messages).
fn esc(s: &str) -> String {
    s.replace('\\', "\\\\").replace('"', "\\\"")
}

fn respond_json(req: tiny_http::Request, body: &str) {
    let header =
        tiny_http::Header::from_bytes(&b"Content-Type"[..], &b"application/json"[..]).unwrap();
    let _ = req.respond(tiny_http::Response::from_string(body).with_header(header));
}

fn respond_html(req: tiny_http::Request, html: &str) {
    let header =
        tiny_http::Header::from_bytes(&b"Content-Type"[..], &b"text/html; charset=utf-8"[..])
            .unwrap();
    let _ = req.respond(tiny_http::Response::new(
        200.into(),
        vec![header],
        Cursor::new(html.as_bytes().to_vec()),
        Some(html.len()),
        None,
    ));
}

fn open_browser(url: &str) {
    #[cfg(target_os = "macos")]
    let cmd = "open";
    #[cfg(target_os = "linux")]
    let cmd = "xdg-open";
    #[cfg(target_os = "windows")]
    let cmd = "explorer";
    #[cfg(not(any(target_os = "macos", target_os = "linux", target_os = "windows")))]
    let cmd = "";
    if !cmd.is_empty() {
        let _ = std::process::Command::new(cmd).arg(url).spawn();
    }
}

/// The single-page tester UI (vendored; no Node build).
const INDEX_HTML: &str = include_str!("harness/index.html");

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn log_caps_and_serialises_since_a_cursor() {
        let mut log = Log::default();
        log.push(MonitorEvent::from_midi(&[0xB0, 21, 64])); // seq 0
        log.push(MonitorEvent::from_midi(&[0x92, 60, 100])); // seq 1
        let json = log.since_json(1);
        assert!(json.contains("\"next\":2"), "{json}");
        assert!(json.contains("\"seq\":1"), "{json}");
        assert!(
            !json.contains("\"seq\":0"),
            "since=1 must exclude seq 0: {json}"
        );
    }

    #[test]
    fn log_drops_oldest_past_capacity_but_keeps_seq_monotonic() {
        let mut log = Log::default();
        for _ in 0..(LOG_CAP + 5) {
            log.push(MonitorEvent::from_midi(&[0xB0, 1, 1]));
        }
        assert_eq!(log.events.len(), LOG_CAP);
        assert_eq!(log.next_seq, (LOG_CAP + 5) as u64);
        // Oldest retained seq is 5 (0..4 dropped).
        assert_eq!(log.events.first().unwrap().0, 5);
    }

    #[test]
    fn query_value_extracts_keys() {
        assert_eq!(query_value("since=7&port=2", "since"), Some("7"));
        assert_eq!(query_value("since=7&port=2", "port"), Some("2"));
        assert_eq!(query_value("since=7", "port"), None);
    }
}
