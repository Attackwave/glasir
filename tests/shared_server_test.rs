//! Several clients of one tree share one backend, through the shipped
//! executable: what the `serve` relay promises is only visible across processes.
#![cfg(unix)]
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

struct Client {
    child: Child,
    lines: BufReader<std::process::ChildStdout>,
}

impl Client {
    fn start(tree: &Path, state: &Path) -> Client {
        let mut child = Command::new(env!("CARGO_BIN_EXE_glasir"))
            .args(["serve"])
            .arg(tree)
            .args(["--linger", "1"])
            .env("XDG_STATE_HOME", state)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .unwrap();
        let lines = BufReader::new(child.stdout.take().unwrap());
        Client { child, lines }
    }

    fn ask(&mut self, id: u64, method: &str, params: serde_json::Value) -> serde_json::Value {
        let msg =
            serde_json::json!({"jsonrpc": "2.0", "id": id, "method": method, "params": params});
        let stdin = self.child.stdin.as_mut().unwrap();
        writeln!(stdin, "{msg}").unwrap();
        stdin.flush().unwrap();
        loop {
            let mut line = String::new();
            assert!(
                self.lines.read_line(&mut line).unwrap() > 0,
                "the relay closed"
            );
            let reply: serde_json::Value = serde_json::from_str(&line).unwrap();
            if reply["id"] == id {
                return reply;
            }
        }
    }

    fn callers(&mut self, id: u64) -> u64 {
        let reply = self.ask(
            id,
            "tools/call",
            serde_json::json!({"name": "find_callers", "arguments": {"symbol": "src/lib.rs#shared"}}),
        );
        reply["result"]["structuredContent"]["count"]
            .as_u64()
            .unwrap()
    }

    fn close(mut self) {
        drop(self.child.stdin.take());
        assert!(self.child.wait().unwrap().success());
    }
}

/// The backends serving `tree`, by the commands that started them. The tree
/// path is this test's own, so no other process matches.
fn backends(tree: &Path) -> Vec<u32> {
    let out = Command::new("ps")
        .args(["-eo", "pid=,args="])
        .output()
        .unwrap();
    let tree = tree.to_string_lossy();
    String::from_utf8_lossy(&out.stdout)
        .lines()
        .filter(|l| l.contains(" serve ") && l.contains("--backend") && l.contains(&*tree))
        .map(|l| l.split_whitespace().next().unwrap().parse().unwrap())
        .collect()
}

fn wait_until(what: &str, mut done: impl FnMut() -> bool) {
    let deadline = Instant::now() + Duration::from_secs(20);
    while !done() {
        assert!(Instant::now() < deadline, "timed out waiting until {what}");
        std::thread::sleep(Duration::from_millis(50));
    }
}

#[test]
fn clients_of_one_tree_share_one_backend() {
    // Short on every system: a socket address holds about a hundred bytes.
    let base = PathBuf::from(format!("/tmp/gls-{}", std::process::id()));
    // Removed however the test ends, with the backends it started.
    struct Cleanup(PathBuf);
    impl Drop for Cleanup {
        fn drop(&mut self) {
            for pid in backends(&self.0.join("t")) {
                let _ = Command::new("kill").arg(pid.to_string()).status();
            }
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }
    let _cleanup = Cleanup(base.clone());
    let (tree, state): (PathBuf, PathBuf) = (base.join("t"), base.join("s"));
    std::fs::create_dir_all(tree.join("src")).unwrap();
    std::fs::write(
        tree.join("src/lib.rs"),
        "pub fn shared() {}\npub fn first() { shared(); }\npub fn second() { shared(); }\n",
    )
    .unwrap();

    // Started together, as assistants opened side by side start them.
    let mut clients: Vec<Client> = (0..4).map(|_| Client::start(&tree, &state)).collect();
    let handles: Vec<_> = clients
        .drain(..)
        .map(|mut c| {
            std::thread::spawn(move || {
                c.ask(1, "initialize", serde_json::json!({"protocolVersion": "2025-06-18", "capabilities": {}, "clientInfo": {"name": "t", "version": "1"}}));
                assert_eq!(c.callers(2), 2);
                c
            })
        })
        .collect();
    let mut clients: Vec<Client> = handles.into_iter().map(|h| h.join().unwrap()).collect();
    let running = backends(&tree);
    assert_eq!(running.len(), 1, "{running:?}");

    // A backend that dies is started again by the next request.
    Command::new("kill")
        .args(["-9", &running[0].to_string()])
        .status()
        .unwrap();
    wait_until("the backend is gone", || backends(&tree).is_empty());
    assert_eq!(clients[0].callers(3), 2);
    assert_eq!(clients[1].callers(3), 2);
    assert_eq!(backends(&tree).len(), 1);

    // The last client leaving ends it.
    for client in clients.drain(..) {
        client.close();
    }
    wait_until("the backend has ended", || backends(&tree).is_empty());
    let sockets = std::fs::read_dir(state.join("glasir/sockets")).unwrap();
    assert!(
        !sockets
            .flatten()
            .any(|e| e.path().extension().is_some_and(|x| x == "sock"))
    );
}
