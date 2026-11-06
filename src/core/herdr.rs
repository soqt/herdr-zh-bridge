//! herdr-core (`hc`) adapter. Target A is always an existing terminal.

use serde_json::Value;

use super::error::{BridgeError, Result};
use super::platform::executable;
use super::process::{RunRequest, Runner, system_runner};
use super::text::{is_blank, truthy, value_text};

/// Decodes a JSON reply, surfacing `{"error": ...}` and unwrapping `{"result": ...}`.
pub fn unwrap(text: &str) -> Result<Value> {
    let value: Value =
        serde_json::from_str(text).map_err(|e| BridgeError::new(format!("无法解析命令返回的 JSON：{e}")))?;
    if let Value::Object(map) = &value {
        if let Some(error) = map.get("error").filter(|e| truthy(e)) {
            return Err(BridgeError::new(value_text(error)));
        }
        if let Some(result) = map.get("result") {
            return Ok(result.clone());
        }
    }
    Ok(value)
}

/// The agent state: `status.state`, `status.status`, or a plain `status` string.
pub fn status(record: &Value) -> Option<String> {
    match record.get("status")? {
        Value::Object(s) => match s.get("state") {
            Some(state) => state.as_str().map(String::from),
            None => s.get("status")?.as_str().map(String::from),
        },
        Value::String(s) => Some(s.clone()),
        _ => None,
    }
}

/// The hc terminal this process runs in, if any.
pub fn own_terminal() -> Option<String> {
    std::env::var("HC_TERMINAL_ID").ok().filter(|id| !is_blank(id))
}

/// Detects a replaced agent or shell inside a kept terminal.
#[derive(Clone, Debug, PartialEq)]
pub struct Identity(Value, Value, Value);

pub fn identity(record: &Value) -> Identity {
    let field = |value: Option<&Value>| value.cloned().unwrap_or(Value::Null);
    let foreground = record.get("foreground").filter(|f| truthy(f));
    Identity(
        field(record.get("pid")),
        field(foreground.and_then(|f| f.get("pid"))),
        field(foreground.and_then(|f| f.get("name"))),
    )
}

pub fn turn(record: &Value) -> Option<Value> {
    record.get("turn").cloned()
}

/// The terminal operations a conversation needs; swappable for tests.
pub trait Terminals: Send {
    fn set_binary(&mut self, _binary: &str) {}
    fn terminals(&self) -> Result<Vec<Value>>;
    fn get(&self, address: &str) -> Result<Value>;
    fn read(&self, address: &str) -> Result<String>;
    /// Only the visible screen, which never scrolls the program.
    fn screen(&self, address: &str) -> Result<String> {
        self.read(address)
    }
    fn send(&self, address: &str, text: &str) -> Result<()>;
}

pub struct Core {
    pub binary: String,
    runner: Runner,
}

impl Core {
    pub fn new(binary: impl Into<String>) -> Self {
        Self::with_runner(binary, system_runner())
    }

    pub fn with_runner(binary: impl Into<String>, runner: Runner) -> Self {
        Core { binary: binary.into(), runner }
    }

    fn call(&self, args: &[&str], timeout_secs: u64) -> Result<String> {
        let mut argv = vec![executable(&self.binary)];
        argv.extend(args.iter().map(|a| a.to_string()));
        (self.runner)(&RunRequest::new(argv, timeout_secs))
    }

    fn read_lines(&self, address: &str, lines: &str) -> Result<String> {
        match unwrap(&self.call(&["read", "--json", address, "--lines", lines], 30)?)? {
            Value::String(text) => Ok(text),
            Value::Object(map) if map.get("text").is_some_and(Value::is_string) => {
                Ok(map["text"].as_str().unwrap().to_string())
            }
            _ => Err(BridgeError::new("hc read 返回了未知格式，无法提取终端文本。")),
        }
    }
}

pub fn validate_send_text(text: &str) -> Result<()> {
    let control = text.chars().any(|c| (c as u32) < 32 && !matches!(c, '\n' | '\r' | '\t'));
    if is_blank(text) || control {
        return Err(BridgeError::new("译文为空或包含不允许的终端控制字符。"));
    }
    Ok(())
}

impl Terminals for Core {
    fn set_binary(&mut self, binary: &str) {
        self.binary = binary.to_string();
    }

    fn terminals(&self) -> Result<Vec<Value>> {
        let value = unwrap(&self.call(&["ls", "--json"], 30)?)?;
        let records = match &value {
            Value::Object(map) => map.get("terminals").cloned().unwrap_or(Value::Null),
            other => other.clone(),
        };
        match records {
            Value::Array(records) => Ok(records),
            _ => Err(BridgeError::new("hc ls 的格式与当前适配器不符，需要核对本机版本。")),
        }
    }

    fn get(&self, address: &str) -> Result<Value> {
        let value = unwrap(&self.call(&["get", "--json", address], 30)?)?;
        let value = match value {
            Value::Object(mut map) if map.contains_key("terminal") => map.remove("terminal").unwrap(),
            other => other,
        };
        match value {
            Value::Object(_) => Ok(value),
            _ => Err(BridgeError::new("hc get 返回了未知格式。")),
        }
    }

    fn read(&self, address: &str) -> Result<String> {
        self.read_lines(address, "3000")
    }

    fn screen(&self, address: &str) -> Result<String> {
        self.read_lines(address, "0")
    }

    fn send(&self, address: &str, text: &str) -> Result<()> {
        validate_send_text(text)?;
        // No status precondition. No invocation or flags reach target CLI A:
        // hc pastes and submits.
        self.call(&["send", "--", address, text], 45).map(|_| ())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::sync::{Arc, Mutex};

    fn recording(reply: &'static str) -> (Core, Arc<Mutex<Vec<Vec<String>>>>) {
        let calls = Arc::new(Mutex::new(Vec::new()));
        let log = calls.clone();
        let runner: Runner = Arc::new(move |request: &RunRequest| {
            log.lock().unwrap().push(request.argv.clone());
            Ok(reply.to_string())
        });
        (Core::with_runner("/test/hc", runner), calls)
    }

    #[test]
    fn control_characters_rejected_before_running() {
        let (core, calls) = recording("ok");
        assert!(core.send("t3", "\u{1b}[31mhello").is_err());
        assert!(core.send("t3", " \n ").is_err());
        assert!(calls.lock().unwrap().is_empty());
    }

    #[test]
    fn arguments_are_not_shell_code() {
        let (core, calls) = recording("ok");
        let text = "$(whoami) `cmd` \"quote\"\n中文";
        core.send("t3", text).unwrap();
        assert_eq!(calls.lock().unwrap()[0], ["/test/hc", "send", "--", "t3", text]);
    }

    #[test]
    fn read_decodes_structured_text() {
        let (core, _) = recording(r#"{"text":"hello\nworld", "complete":true, "how":"screen"}"#);
        assert_eq!(core.read("t3").unwrap(), "hello\nworld");
    }

    #[test]
    fn listing_and_errors() {
        let (core, _) = recording(r#"{"hidden_finished":6,"terminals":[{"address":"t7"}]}"#);
        assert_eq!(core.terminals().unwrap(), vec![json!({"address": "t7"})]);
        let (core, _) = recording(r#"{"error":"no such terminal"}"#);
        assert_eq!(core.get("t9").unwrap_err().message(), "no such terminal");
        let (core, _) = recording(r#"{"result":{"terminal":{"address":"t1"}}}"#);
        assert_eq!(core.get("t1").unwrap(), json!({"address": "t1"}));
    }

    #[test]
    fn status_and_identity() {
        assert_eq!(status(&json!({"status": {"state": "idle"}})), Some("idle".into()));
        assert_eq!(status(&json!({"status": {"status": "done"}})), Some("done".into()));
        assert_eq!(status(&json!({"status": "working"})), Some("working".into()));
        assert_eq!(status(&json!({"status": null})), None);
        let a = json!({"pid": 10, "foreground": {"pid": 11, "name": "claude"}});
        let b = json!({"pid": 10, "foreground": {"pid": 99, "name": "claude"}});
        assert_ne!(identity(&a), identity(&b));
        assert_eq!(identity(&a), identity(&a.clone()));
    }
}
