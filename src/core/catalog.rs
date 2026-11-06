//! Read-only catalogs: Herdr's actual layout and Codex's `model/list` RPC.

use std::collections::HashSet;
use std::io::{BufRead, BufReader, Write};
use std::process::Stdio;
use std::sync::mpsc::{self, Receiver, RecvTimeoutError};
use std::thread;
use std::time::{Duration, Instant};

use indexmap::IndexMap;
use serde_json::{Value, json};

use super::error::{BridgeError, Result};
use super::herdr::{Terminals, own_terminal, status, unwrap};
use super::platform::executable;
use super::process::{RunRequest, command, run};
use super::text::{first_text, truthy, value_text};
use super::translator::codex_argv;

pub fn herdr_executable() -> String {
    executable("herdr-app")
}

/// `None` means unknown; an empty advertised list means no effort override.
pub fn reasoning_efforts(model: Option<&Value>) -> Option<Vec<String>> {
    let supported = model?.get("supportedReasoningEfforts")?.as_array()?;
    let mut choices: Vec<String> = Vec::new();
    for item in supported {
        if let Some(effort) = item.get("reasoningEffort").and_then(Value::as_str)
            && !effort.is_empty()
            && !choices.iter().any(|c| c == effort)
        {
            choices.push(effort.to_string());
        }
    }
    Some(choices)
}

/// One selectable terminal, grouped by machine / workspace / tab.
#[derive(Clone, Debug, PartialEq)]
pub struct TerminalRow {
    pub address: String,
    /// (stable key, label) from the outermost group inwards.
    pub groups: Vec<(String, String)>,
    pub name: String,
    pub app: String,
    pub status: String,
    pub cwd: String,
    /// The complete source records, never trimmed.
    pub raw: Value,
}

impl TerminalRow {
    /// Case-insensitive search over every field, including the raw records.
    pub fn matches(&self, query: &str) -> bool {
        let query = query.trim().to_lowercase();
        if query.is_empty() {
            return true;
        }
        let haystack = json!({
            "address": self.address, "groups": self.groups, "name": self.name,
            "app": self.app, "status": self.status, "cwd": self.cwd, "raw": self.raw,
        })
        .to_string()
        .to_lowercase();
        haystack.contains(&query)
    }
}

fn record_address(record: &Value) -> String {
    match record.get("address") {
        Some(address) => address.as_str().unwrap_or_default().to_string(),
        None => record.get("terminal").and_then(Value::as_str).unwrap_or_default().to_string(),
    }
}

struct CatalogBuilder<'a> {
    indexed: IndexMap<String, &'a Value>,
    seen: HashSet<String>,
    rows: Vec<TerminalRow>,
}

impl CatalogBuilder<'_> {
    fn add(
        &mut self,
        address: &str,
        groups: Vec<(String, String)>,
        view: Option<&Value>,
        space: Option<&Value>,
        tab: Option<&Value>,
    ) {
        if address.is_empty() || !self.seen.insert(address.to_string()) {
            return;
        }
        let null = Value::Null;
        let raw = self.indexed.get(address).copied().unwrap_or(&null);
        let view_or_empty = view.cloned().unwrap_or_else(|| json!({}));
        let view = &view_or_empty;
        let state = raw.get("status").filter(|s| s.is_object());
        let foreground = raw.get("foreground");
        let status_text = status(raw).filter(|s| !s.is_empty());
        let name =
            first_text([raw.get("name"), raw.get("title"), view.get("name")]).unwrap_or_else(|| address.to_string());
        let row = TerminalRow {
            address: address.to_string(),
            groups: groups.clone(),
            name: name.clone(),
            app: first_text([
                state.and_then(|s| s.get("app")),
                raw.get("agent"),
                view.get("app"),
                foreground.and_then(|f| f.get("name")),
            ])
            .unwrap_or_else(|| "未上报".into()),
            status: status_text.or_else(|| first_text([view.get("status")])).unwrap_or_else(|| "unknown".into()),
            cwd: first_text([
                raw.get("cwd"),
                foreground.and_then(|f| f.get("cwd")),
                space.and_then(|s| s.get("folder")),
            ])
            .unwrap_or_default(),
            raw: json!({
                "terminal": if raw.is_null() { json!({}) } else { raw.clone() },
                "herdr_view": view,
                "space": space,
                "tab": tab,
            }),
        };
        self.rows.push(row);

        let mut children: Vec<String> = view
            .get("sub_agents")
            .and_then(Value::as_array)
            .map(|agents| agents.iter().filter_map(Value::as_str).map(String::from).collect())
            .unwrap_or_default();
        for (child, record) in &self.indexed {
            if record.get("parent").and_then(Value::as_str) == Some(address) && !children.contains(child) {
                children.push(child.clone());
            }
        }
        for child in children {
            let mut nested = groups.clone();
            nested.push((format!("parent:{address}"), format!("子 agent · {name}")));
            self.add(&child, nested, None, space, tab);
        }
    }
}

/// Preserves source records, ordering, duplicate display names and full addresses.
pub fn terminal_catalog(records: &[Value], world: &Value) -> Vec<TerminalRow> {
    let mut indexed = IndexMap::new();
    for record in records {
        indexed.insert(record_address(record), record);
    }
    let mut builder = CatalogBuilder { indexed, seen: HashSet::new(), rows: Vec::new() };
    let spaces = world.get("spaces").and_then(Value::as_array).cloned().unwrap_or_default();
    for space in &spaces {
        let machine = first_text([space.get("machine")]).unwrap_or_else(|| "本机".into());
        for tab in space.get("tabs").and_then(Value::as_array).into_iter().flatten() {
            let tab_label = first_text([tab.get("name")]).unwrap_or_else(|| "未命名标签页".into())
                + if tab.get("settled").is_some_and(truthy) { " · 已收起" } else { "" };
            let groups = vec![
                (format!("machine:{machine}"), machine.clone()),
                (
                    format!("space:{}", value_text(space.get("id").unwrap_or(&Value::Null))),
                    first_text([space.get("name"), space.get("folder")]).unwrap_or_else(|| "未命名工作区".into()),
                ),
                (format!("tab:{}", value_text(tab.get("id").unwrap_or(&Value::Null))), tab_label),
            ];
            for terminal in tab.get("terminals").and_then(Value::as_array).into_iter().flatten() {
                let address = terminal.get("terminal").and_then(Value::as_str).unwrap_or_default();
                builder.add(address, groups.clone(), Some(terminal), Some(space), Some(tab));
            }
        }
    }
    let unmapped: Vec<String> = builder.indexed.keys().cloned().collect();
    for address in unmapped {
        if address.is_empty() || builder.seen.contains(&address) {
            continue;
        }
        let machine = address.rsplit_once('/').map_or("本机", |(m, _)| m).to_string();
        let groups =
            vec![(format!("machine:{machine}"), machine), ("unmapped".into(), "未关联到当前 Herdr 布局".into())];
        builder.add(&address, groups, None, None, None);
    }
    builder.rows
}

/// Lists terminals with Herdr's layout, minus the one running this bridge.
/// A layout failure only adds a warning.
pub fn load_terminals(core: &dyn Terminals, app_binary: &str) -> Result<(Vec<TerminalRow>, String)> {
    let records = core.terminals()?;
    let world = run(&RunRequest::new(vec![executable(app_binary), "ls".into()], 30))
        .and_then(|text| unwrap(&text))
        .and_then(|world| {
            if world.get("spaces").is_some_and(Value::is_array) {
                Ok(world)
            } else {
                Err(BridgeError::new("Herdr 布局返回格式不符"))
            }
        });
    let (mut rows, warning) = match world {
        Ok(world) => (terminal_catalog(&records, &world), String::new()),
        Err(error) => {
            (terminal_catalog(&records, &json!({})), format!("暂时无法读取 Herdr 布局，终端仍完整列出：{error}"))
        }
    };
    if let Some(own) = own_terminal() {
        rows.retain(|row| row.address != own);
    }
    Ok((rows, warning))
}

const TIMEOUT: &str = "查询模型超时；可以手动输入模型 ID。";

struct Rpc<'a> {
    write: &'a mut dyn FnMut(&Value) -> Result<()>,
    messages: &'a Receiver<Value>,
    deadline: Instant,
    sequence: i64,
}

impl Rpc<'_> {
    fn request(&mut self, method: &str, params: Value) -> Result<Value> {
        self.sequence += 1;
        (self.write)(&json!({"id": self.sequence, "method": method, "params": params}))?;
        loop {
            let left = self.deadline.saturating_duration_since(Instant::now());
            let message = match self.messages.recv_timeout(left) {
                Ok(message) => message,
                Err(RecvTimeoutError::Timeout) => return Err(BridgeError::new(TIMEOUT)),
                Err(RecvTimeoutError::Disconnected) => {
                    return Err(BridgeError::new("Codex 模型服务已退出，请检查 Codex 版本或登录状态。"));
                }
            };
            if message.get("method").is_some() && message.get("id").is_some() {
                // A server request: this client is read-only and refuses it.
                (self.write)(&json!({"id": message["id"],
                    "error": {"code": -32601, "message": "Read-only model catalog client"}}))?;
                continue;
            }
            if message.get("id") != Some(&json!(self.sequence)) {
                continue;
            }
            if let Some(error) = message.get("error") {
                return Err(BridgeError::new(value_text(error)));
            }
            return Ok(message.get("result").cloned().unwrap_or_else(|| json!({})));
        }
    }
}

/// The JSON-RPC client half of `model/list`, independent of the transport.
pub fn query_models(
    write: &mut dyn FnMut(&Value) -> Result<()>,
    messages: &Receiver<Value>,
    deadline: Instant,
) -> Result<Vec<Value>> {
    let mut rpc = Rpc { write, messages, deadline, sequence: 0 };
    rpc.request(
        "initialize",
        json!({"clientInfo": {"name": "herdr_zh_bridge", "title": "Herdr Chinese translator",
                              "version": env!("CARGO_PKG_VERSION")}, "capabilities": {}}),
    )?;
    (rpc.write)(&json!({"method": "initialized", "params": {}}))?;
    let (mut models, mut cursor, mut seen) = (Vec::new(), None::<Value>, HashSet::new());
    for _ in 0..20 {
        let mut params = json!({"limit": 100, "includeHidden": false});
        if let Some(cursor) = &cursor {
            params["cursor"] = cursor.clone();
        }
        let result = rpc.request("model/list", params)?;
        for model in result.get("data").and_then(Value::as_array).into_iter().flatten() {
            if let Some(slug) = first_text([model.get("model"), model.get("id")])
                && seen.insert(slug)
            {
                models.push(model.clone());
            }
        }
        let Some(next) = result.get("nextCursor").cloned().filter(truthy) else {
            return Ok(models);
        };
        if Some(&next) == cursor.as_ref() {
            return Err(BridgeError::new("模型分页游标未变化。"));
        }
        cursor = Some(next);
    }
    Err(BridgeError::new("模型目录页数过多，请手动输入模型 ID。"))
}

/// No credentials are read by this app; Codex uses its own login and config.
pub fn list_models(binary: &str, timeout: Duration) -> Result<Vec<Value>> {
    let mut argv = codex_argv(binary)?;
    argv.push("app-server".into());
    let mut child = command(&argv)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|e| BridgeError::new(format!("无法启动 {}：{e}", argv[0])))?;
    let (sender, messages) = mpsc::channel();
    let stdout = child.stdout.take().unwrap();
    thread::spawn(move || {
        for line in BufReader::new(stdout).lines() {
            let Ok(line) = line else { break };
            if let Ok(message) = serde_json::from_str::<Value>(&line)
                && sender.send(message).is_err()
            {
                break;
            }
        }
    });
    let mut stdin = child.stdin.take().unwrap();
    let mut write = |message: &Value| -> Result<()> {
        writeln!(stdin, "{message}")
            .and_then(|_| stdin.flush())
            .map_err(|_| BridgeError::new("Codex 模型服务已退出，请检查 Codex 版本或登录状态。"))
    };
    let result = query_models(&mut write, &messages, Instant::now() + timeout);
    drop(stdin);
    let _ = child.kill();
    let _ = child.wait();
    result
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    pub fn world() -> Value {
        json!({"spaces": [
            {"id": "s1", "name": "DemoProject", "machine": null, "folder": "C:/work/DemoProject", "tabs": [
                {"id": "tab1", "name": "开发", "terminals": [{"terminal": "t1", "sub_agents": ["t2"]}]},
                {"id": "tab2", "name": "开发", "terminals": [{"terminal": "t3"}]}]},
            {"id": "remote/s1", "name": "DemoProject", "machine": "remote", "folder": "/work/DemoProject", "tabs": [
                {"id": "remote/tab1", "name": "开发", "terminals": [{"terminal": "remote/t1"}]}]}
        ]})
    }

    pub fn records() -> Vec<Value> {
        vec![
            json!({"address": "t1", "name": "Codex", "cwd": "C:/work/DemoProject",
                   "status": {"state": "idle", "app": "codex"}, "extra": {"keep": "all"}}),
            json!({"address": "t2", "name": "Reviewer", "parent": "t1", "status": "working"}),
            json!({"address": "t3", "name": "Codex", "status": "idle"}),
            json!({"address": "remote/t1", "name": "Codex", "cwd": "/work/DemoProject", "status": "done"}),
            json!({"address": "t9", "name": "Other", "status": null}),
        ]
    }

    #[test]
    fn actual_layout_order_and_children() {
        let rows = terminal_catalog(&records(), &world());
        let addresses: Vec<&str> = rows.iter().map(|r| r.address.as_str()).collect();
        assert_eq!(addresses, ["t1", "t2", "t3", "remote/t1", "t9"]);
        assert_eq!(rows[1].groups.last().unwrap().0, "parent:t1");
        assert_ne!(rows[0].groups, rows[2].groups);
        assert_eq!(rows[3].groups[0].1, "remote");
        assert_eq!(rows[0].raw["terminal"]["extra"]["keep"], "all");
        assert_eq!(rows[0].app, "codex");
        assert_eq!(rows[1].status, "working");
        assert!(rows.last().unwrap().groups.last().unwrap().1.contains("未关联"));
    }

    #[test]
    fn cycles_do_not_duplicate_or_loop() {
        let records = vec![json!({"address": "t1", "parent": "t2"}), json!({"address": "t2", "parent": "t1"})];
        // Includes the layout-only t3 and remote/t1.
        assert_eq!(terminal_catalog(&records, &world()).len(), 4);
    }

    #[test]
    fn real_hc_records_without_status() {
        let records = vec![json!({"address": "t7", "agent": "zsh", "cwd": "/Users/demo", "state": "live", "turn": 0})];
        let rows = terminal_catalog(&records, &json!({}));
        assert_eq!((rows[0].app.as_str(), rows[0].status.as_str()), ("zsh", "unknown"));
        assert!(rows[0].matches("/users/demo"));
    }

    #[test]
    fn effort_catalog_preserves_supported_levels_without_guessing() {
        assert_eq!(reasoning_efforts(None), None);
        assert_eq!(reasoning_efforts(Some(&json!({}))), None);
        assert_eq!(reasoning_efforts(Some(&json!({"supportedReasoningEfforts": []}))), Some(vec![]));
        let model = json!({"supportedReasoningEfforts": [
            {"reasoningEffort": "high"}, {"reasoningEffort": "max"}, {"reasoningEffort": "high"},
            {"reasoningEffort": "future-level"}]});
        assert_eq!(reasoning_efforts(Some(&model)).unwrap(), ["high", "max", "future-level"]);
    }

    /// An in-process app-server that checks the handshake and pages results.
    fn fake_server() -> (impl FnMut(&Value) -> Result<()>, Receiver<Value>) {
        let (sender, receiver) = mpsc::channel();
        let mut initialized = false;
        let write = move |message: &Value| -> Result<()> {
            match message["method"].as_str() {
                Some("initialize") => {
                    sender.send(json!({"method": "server/notice", "params": {}})).unwrap();
                    sender.send(json!({"id": 99, "method": "server/ask", "params": {}})).unwrap();
                    sender.send(json!({"id": message["id"], "result": {}})).unwrap();
                }
                Some("initialized") => initialized = true,
                Some("model/list") => {
                    assert!(initialized);
                    let result = if message["params"].get("cursor").is_some() {
                        json!({"data": [{"id": "m2", "model": "m2", "displayName": "Model 2"}], "nextCursor": null})
                    } else {
                        json!({"data": [{"id": "m1", "model": "m1"}, {"id": "m1", "model": "m1"}], "nextCursor": "page2"})
                    };
                    sender.send(json!({"id": message["id"], "result": result})).unwrap();
                }
                _ => assert_eq!(message["error"]["code"], -32601),
            }
            Ok(())
        };
        (write, receiver)
    }

    #[test]
    fn model_rpc_handshake_and_pagination() {
        let (mut write, messages) = fake_server();
        let models = query_models(&mut write, &messages, Instant::now() + Duration::from_secs(5)).unwrap();
        let slugs: Vec<&str> = models.iter().map(|m| m["model"].as_str().unwrap()).collect();
        assert_eq!(slugs, ["m1", "m2"]);
    }

    #[test]
    fn model_rpc_timeout() {
        let (_sender, messages) = mpsc::channel::<Value>();
        let mut write = |_: &Value| Ok(());
        let error = query_models(&mut write, &messages, Instant::now() + Duration::from_millis(50)).unwrap_err();
        assert_eq!(error.message(), TIMEOUT);
    }
}
