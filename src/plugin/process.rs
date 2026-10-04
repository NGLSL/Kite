//! 插件进程后端：可注入 ProcessBackend（测试用 mock，生产用 stdio 子进程）。

use std::collections::VecDeque;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::mpsc::{self, Receiver, TryRecvError};

use super::protocol::{decode_frames, encode_frame};
use super::PLUGIN_LOG_MAX_BYTES;
use serde_json::{json, Value};

pub trait PluginProcess: Send {
    fn send_frame(&mut self, body: &str) -> Result<(), String>;
    /// 非阻塞取一帧 JSON body；无数据返回 Ok(None)。
    fn try_recv_frame(&mut self) -> Result<Option<String>, String>;
    fn is_alive(&mut self) -> bool;
    fn kill(&mut self);
}

pub trait ProcessBackend: Send {
    fn spawn(
        &mut self,
        plugin_id: &str,
        command: &Path,
        args: &[String],
        workdir: &Path,
    ) -> Result<Box<dyn PluginProcess>, String>;
}

/// 生产后端：独立子进程 + 专用读线程（RPC 不跑在 UI 线程）。
pub struct StdioBackend {
    data_dir: PathBuf,
}

impl StdioBackend {
    pub fn new(data_dir: PathBuf) -> Self {
        Self { data_dir }
    }
}

impl ProcessBackend for StdioBackend {
    fn spawn(
        &mut self,
        plugin_id: &str,
        command: &Path,
        args: &[String],
        workdir: &Path,
    ) -> Result<Box<dyn PluginProcess>, String> {
        let mut cmd = Command::new(command);
        cmd.args(args)
            .current_dir(workdir)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        // GUI 宿主拉起控制台子进程：隐藏窗口，避免闪烁/焦点抢夺。
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            const CREATE_NO_WINDOW: u32 = 0x0800_0000;
            cmd.creation_flags(CREATE_NO_WINDOW);
        }
        let mut child = cmd.spawn().map_err(|e| format!("spawn failed: {e}"))?;
        let stdin = child
            .stdin
            .take()
            .ok_or_else(|| "plugin stdin unavailable".to_string())?;
        let stdout = child
            .stdout
            .take()
            .ok_or_else(|| "plugin stdout unavailable".to_string())?;
        let stderr = child.stderr.take();
        let (tx, rx) = mpsc::channel::<String>();
        std::thread::spawn(move || {
            let mut buf = Vec::new();
            let mut stdout = stdout;
            let mut chunk = [0u8; 4096];
            loop {
                match stdout.read(&mut chunk) {
                    Ok(0) => break,
                    Ok(n) => {
                        buf.extend_from_slice(&chunk[..n]);
                        for frame in decode_frames(&mut buf) {
                            if tx.send(frame).is_err() {
                                return;
                            }
                        }
                    }
                    Err(_) => break,
                }
            }
        });
        if let Some(stderr) = stderr {
            let log_path = super::plugin_log_path(&self.data_dir, plugin_id);
            if let Some(parent) = log_path.parent() {
                let _ = std::fs::create_dir_all(parent);
            }
            std::thread::spawn(move || {
                append_stderr_loop(stderr, &log_path);
            });
        }
        Ok(Box::new(ThreadedStdioProcess {
            child,
            stdin,
            rx,
        }))
    }
}

fn append_stderr_loop(mut stderr: std::process::ChildStderr, log_path: &Path) {
    let mut chunk = [0u8; 1024];
    loop {
        match stderr.read(&mut chunk) {
            Ok(0) => break,
            Ok(n) => {
                if let Ok(meta) = std::fs::metadata(log_path) {
                    if meta.len() >= PLUGIN_LOG_MAX_BYTES {
                        let rotated = log_path.with_extension("log.1");
                        let _ = std::fs::rename(log_path, rotated);
                    }
                }
                if let Ok(mut f) = std::fs::OpenOptions::new()
                    .create(true)
                    .append(true)
                    .open(log_path)
                {
                    let _ = f.write_all(&chunk[..n]);
                }
            }
            Err(_) => break,
        }
    }
}

struct ThreadedStdioProcess {
    child: Child,
    stdin: ChildStdin,
    rx: Receiver<String>,
}

impl PluginProcess for ThreadedStdioProcess {
    fn send_frame(&mut self, body: &str) -> Result<(), String> {
        self.stdin
            .write_all(&encode_frame(body))
            .map_err(|e| e.to_string())?;
        self.stdin.flush().map_err(|e| e.to_string())
    }

    fn try_recv_frame(&mut self) -> Result<Option<String>, String> {
        match self.rx.try_recv() {
            Ok(frame) => Ok(Some(frame)),
            Err(TryRecvError::Empty) => Ok(None),
            Err(TryRecvError::Disconnected) => Err("plugin stdout closed".into()),
        }
    }

    fn is_alive(&mut self) -> bool {
        matches!(self.child.try_wait(), Ok(None))
    }

    fn kill(&mut self) {
        let _ = self.child.kill();
    }
}

/// 测试缝 mock：与官方 Calculator 同语义（括号/优先级/隐式乘/幂/开方/千分位）。
/// 生产路径不调用；仅 MemProcess 自动应答使用。
fn mock_calculator_result(query: &str) -> Value {
    let expr = query.trim();
    if expr.is_empty() {
        return json!({"type":"empty"});
    }
    match eval_expression(expr) {
        Ok(v) => {
            let text = format_number(v);
            json!({
                "type": "panel",
                "panel": {
                    "blocks": [
                        { "type": "text", "text": pretty_expr(expr), "style": "secondary" },
                        { "type": "value", "value": text }
                    ],
                    "actions": [{
                        "id": "copy",
                        "label": "复制结果",
                        "shortcut": "Enter",
                        "default": true,
                        "action": { "type": "copy_text", "text": text }
                    }]
                }
            })
        }
        Err(msg) => json!({
            "type": "panel",
            "panel": {
                "blocks": [{ "type": "notice", "level": "error", "text": msg }],
                "actions": []
            }
        }),
    }
}

fn format_number(v: f64) -> String {
    if !v.is_finite() {
        return v.to_string();
    }
    if (v - v.round()).abs() < 1e-9 && v.abs() < 1e15 {
        format!("{}", v.round() as i64)
    } else {
        let s = format!("{v:.10}");
        s.trim_end_matches('0').trim_end_matches('.').to_string()
    }
}

fn pretty_expr(expr: &str) -> String {
    let n = normalize_expr(expr);
    let mut out = String::with_capacity(n.len() + 8);
    for c in n.chars() {
        match c {
            '*' => out.push_str(" × "),
            '/' => out.push_str(" ÷ "),
            '+' => out.push_str(" + "),
            '-' => out.push_str(" - "),
            c => out.push(c),
        }
    }
    out.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn normalize_expr(expr: &str) -> String {
    let mut out = String::with_capacity(expr.len());
    let chars: Vec<char> = expr.chars().collect();
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        if c == '*' && i + 1 < chars.len() && chars[i + 1] == '*' {
            out.push('^');
            i += 2;
            continue;
        }
        match c {
            '×' | '✕' | '✖' | '·' | '＊' => out.push('*'),
            '÷' | '／' => out.push('/'),
            '（' | '【' | '［' => out.push('('),
            '）' | '】' | '］' => out.push(')'),
            '＋' => out.push('+'),
            '－' | '−' | '–' | '—' => out.push('-'),
            '。' => out.push('.'),
            '，' => out.push(','),
            '[' | '〔' => out.push('('),
            ']' | '〕' => out.push(')'),
            c if c.is_whitespace() => {
                if !out.ends_with(' ') {
                    out.push(' ');
                }
            }
            c => out.push(c),
        }
        i += 1;
    }
    let normalized = out.trim();
    // 兼容旧版宿主或直接调用：有些调用链会把触发前缀 `=` 一并传入。
    normalized.strip_prefix('=').unwrap_or(normalized).trim().to_string()
}

fn eval_expression(expr: &str) -> Result<f64, &'static str> {
    let src = normalize_expr(expr);
    if src.is_empty() {
        return Err("表达式不完整");
    }
    let chars: Vec<char> = src.chars().collect();
    let mut p = MockParser { chars, pos: 0 };
    let v = p.parse_expr()?;
    p.skip_ws();
    if p.pos < p.chars.len() {
        return Err("表达式无效");
    }
    if !v.is_finite() {
        return Err("结果无效");
    }
    Ok(v)
}

fn starts_implicit_factor(c: char) -> bool {
    c == '(' || c == '√' || c.is_ascii_digit() || c == '.'
}

struct MockParser {
    chars: Vec<char>,
    pos: usize,
}

impl MockParser {
    fn peek(&self) -> Option<char> {
        self.chars.get(self.pos).copied()
    }

    fn skip_ws(&mut self) {
        while matches!(self.peek(), Some(c) if c.is_whitespace()) {
            self.pos += 1;
        }
    }

    fn eat(&mut self, c: char) -> bool {
        self.skip_ws();
        if self.peek() == Some(c) {
            self.pos += 1;
            true
        } else {
            false
        }
    }

    fn parse_expr(&mut self) -> Result<f64, &'static str> {
        let mut left = self.parse_term()?;
        loop {
            self.skip_ws();
            match self.peek() {
                Some('+') => {
                    self.pos += 1;
                    left += self.parse_term()?;
                }
                Some('-') => {
                    self.pos += 1;
                    left -= self.parse_term()?;
                }
                _ => break,
            }
        }
        Ok(left)
    }

    fn parse_term(&mut self) -> Result<f64, &'static str> {
        let mut left = self.parse_unary()?;
        loop {
            self.skip_ws();
            match self.peek() {
                Some('*') => {
                    self.pos += 1;
                    left *= self.parse_unary()?;
                }
                Some('/') => {
                    self.pos += 1;
                    let right = self.parse_unary()?;
                    if right == 0.0 {
                        return Err("不能除以 0");
                    }
                    left /= right;
                }
                Some('%') => {
                    self.pos += 1;
                    let right = self.parse_unary()?;
                    if right == 0.0 {
                        return Err("不能对 0 取模");
                    }
                    left %= right;
                }
                Some(c) if starts_implicit_factor(c) => {
                    left *= self.parse_unary()?;
                }
                _ => break,
            }
        }
        Ok(left)
    }

    fn parse_unary(&mut self) -> Result<f64, &'static str> {
        self.skip_ws();
        match self.peek() {
            Some('+') => {
                self.pos += 1;
                self.parse_unary()
            }
            Some('-') => {
                self.pos += 1;
                Ok(-self.parse_unary()?)
            }
            _ => self.parse_power(),
        }
    }

    fn parse_power(&mut self) -> Result<f64, &'static str> {
        let base = self.parse_primary()?;
        self.skip_ws();
        if self.peek() == Some('^') {
            self.pos += 1;
            let exp = self.parse_unary()?;
            let v = base.powf(exp);
            if !v.is_finite() {
                return Err("结果无效");
            }
            return Ok(v);
        }
        Ok(base)
    }

    fn parse_primary(&mut self) -> Result<f64, &'static str> {
        self.skip_ws();
        match self.peek() {
            None => Err("表达式不完整"),
            Some(')') => Err("表达式不完整"),
            Some('(') => {
                self.pos += 1;
                let v = self.parse_expr()?;
                self.skip_ws();
                if !self.eat(')') {
                    return Err("表达式不完整");
                }
                Ok(v)
            }
            Some('√') => {
                self.pos += 1;
                let v = self.parse_unary()?;
                if v < 0.0 {
                    return Err("负数不能开方");
                }
                Ok(v.sqrt())
            }
            Some(c) if c.is_ascii_digit() || c == '.' => self.parse_number(),
            Some(_) => Err("表达式无效"),
        }
    }

    fn parse_number(&mut self) -> Result<f64, &'static str> {
        self.skip_ws();
        let start = self.pos;
        while matches!(self.peek(), Some(c) if c.is_ascii_digit() || c == ',' || c == '_') {
            self.pos += 1;
        }
        if self.peek() == Some('.') {
            self.pos += 1;
            while matches!(self.peek(), Some(c) if c.is_ascii_digit()) {
                self.pos += 1;
            }
        }
        if matches!(self.peek(), Some('e') | Some('E')) {
            let save = self.pos;
            self.pos += 1;
            if matches!(self.peek(), Some('+') | Some('-')) {
                self.pos += 1;
            }
            let dig = self.pos;
            while matches!(self.peek(), Some(c) if c.is_ascii_digit()) {
                self.pos += 1;
            }
            if self.pos == dig {
                self.pos = save;
            }
        }
        let raw: String = self.chars[start..self.pos]
            .iter()
            .filter(|c| **c != ',' && **c != '_')
            .collect();
        if raw.is_empty() || raw == "." || raw.chars().all(|c| c == ',' || c == '_') {
            return Err("表达式不完整");
        }
        if !raw.chars().any(|c| c.is_ascii_digit()) {
            return Err("表达式无效");
        }
        raw.parse::<f64>().map_err(|_| "表达式无效")
    }
}

fn mock_window_list_result(query: &str) -> Value {
    let q = query.trim().to_lowercase();
    let windows = [
        ("window-1", "Visual Studio Code", "Kite — Visual Studio Code"),
        ("window-2", "IntelliJ IDEA", "Kite — IntelliJ IDEA"),
        ("window-3", "Explorer", "Kite — Explorer"),
    ];
    let items: Vec<Value> = windows
        .iter()
        .filter(|(_, title, sub)| {
            q.is_empty()
                || title.to_lowercase().contains(&q)
                || sub.to_lowercase().contains(&q)
                || "kite".contains(&q)
        })
        .map(|(id, title, sub)| {
            json!({
                "id": id,
                "title": title,
                "subtitle": sub,
                "priority": 10,
                "action": {
                    "type": "plugin_action",
                    "action_id": "activate_window",
                    "payload": { "hwnd": id }
                }
            })
        })
        .collect();
    json!({ "type": "list", "items": items })
}

fn mock_devtools_result(provider: &str, _query: &str) -> Value {
    match provider {
        "uuid" => json!({
            "type": "panel",
            "panel": {
                "blocks": [{ "type": "value", "label": "UUID", "value": "00000000-0000-4000-8000-000000000001", "selectable": true }],
                "actions": [{
                    "id": "copy", "label": "复制", "default": true,
                    "action": { "type": "copy_text", "text": "00000000-0000-4000-8000-000000000001" }
                }]
            }
        }),
        "hash" => json!({
            "type": "panel",
            "panel": {
                "blocks": [
                    { "type": "key_value", "items": [
                        { "key": "SHA256", "value": "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855" }
                    ]}
                ],
                "actions": []
            }
        }),
        _ => json!({"type":"empty"}),
    }
}

/// 内存进程：测试缝。按 method 预置响应。
pub struct MemProcess {
    pub sent: Vec<String>,
    pub inbox: VecDeque<String>,
    pub alive: bool,
    pub hang: bool,
    pub crash_after: Option<usize>,
}

impl MemProcess {
    pub fn new() -> Self {
        Self {
            sent: Vec::new(),
            inbox: VecDeque::new(),
            alive: true,
            hang: false,
            crash_after: None,
        }
    }

    pub fn push_response(&mut self, body: impl Into<String>) {
        self.inbox.push_back(body.into());
    }
}

impl Default for MemProcess {
    fn default() -> Self {
        Self::new()
    }
}

impl PluginProcess for MemProcess {
    fn send_frame(&mut self, body: &str) -> Result<(), String> {
        if !self.alive {
            return Err("process dead".into());
        }
        self.sent.push(body.to_string());
        if let Some(n) = self.crash_after {
            if self.sent.len() >= n {
                self.alive = false;
                return Err("process crashed".into());
            }
        }
        Ok(())
    }

    fn try_recv_frame(&mut self) -> Result<Option<String>, String> {
        if !self.alive {
            return Err("process dead".into());
        }
        if self.hang {
            return Ok(None);
        }
        Ok(self.inbox.pop_front())
    }

    fn is_alive(&mut self) -> bool {
        self.alive
    }

    fn kill(&mut self) {
        self.alive = false;
    }
}

/// 测试后端：按 plugin_id 返回预注册的 MemProcess，或脚本化响应。
pub struct MemProcessBackend {
    pub spawned: Vec<(String, PathBuf)>,
    /// plugin_id -> process
    processes: std::collections::HashMap<String, std::sync::Arc<std::sync::Mutex<MemProcess>>>,
    /// 自动应答 initialize/query
    pub auto_respond: bool,
}

impl MemProcessBackend {
    pub fn new() -> Self {
        Self {
            spawned: Vec::new(),
            processes: std::collections::HashMap::new(),
            auto_respond: true,
        }
    }

    pub fn register(&mut self, plugin_id: &str, proc: std::sync::Arc<std::sync::Mutex<MemProcess>>) {
        self.processes.insert(plugin_id.to_string(), proc);
    }

    pub fn process(&self, plugin_id: &str) -> Option<std::sync::Arc<std::sync::Mutex<MemProcess>>> {
        self.processes.get(plugin_id).cloned()
    }
}

impl Default for MemProcessBackend {
    fn default() -> Self {
        Self::new()
    }
}

/// 包装 MemProcess，使 send 时自动 enqueue 标准 initialize/query 响应。
pub struct AutoMemProcess {
    inner: std::sync::Arc<std::sync::Mutex<MemProcess>>,
    plugin_id: String,
    auto: bool,
}

impl PluginProcess for AutoMemProcess {
    fn send_frame(&mut self, body: &str) -> Result<(), String> {
        {
            let mut p = self.inner.lock().unwrap();
            p.send_frame(body)?;
        }
        if self.auto {
            if let Ok(msg) = serde_json::from_str::<serde_json::Value>(body) {
                let id = msg.get("id").and_then(|i| i.as_u64()).unwrap_or(0);
                let method = msg.get("method").and_then(|m| m.as_str()).unwrap_or("");
                let response = match method {
                    "plugin/initialize" => serde_json::json!({
                        "jsonrpc": "2.0",
                        "id": id,
                        "result": {
                            "plugin_api": 1,
                            "capabilities": { "query": true, "execute": true, "cancellation": false }
                        }
                    }),
                    "plugin/query" => {
                        let params = msg.get("params").cloned().unwrap_or_default();
                        let provider = params.get("provider_id").and_then(|p| p.as_str()).unwrap_or("");
                        let q = params.get("query").and_then(|p| p.as_str()).unwrap_or("");
                        let result = if provider == "calculate" || self.plugin_id.contains("calculator") {
                            mock_calculator_result(q)
                        } else if provider.starts_with("win") || self.plugin_id.contains("window") {
                            mock_window_list_result(q)
                        } else if self.plugin_id.contains("devtools") {
                            mock_devtools_result(provider, q)
                        } else {
                            serde_json::json!({"type": "empty"})
                        };
                        serde_json::json!({
                            "jsonrpc": "2.0",
                            "id": id,
                            "result": result
                        })
                    }
                    "plugin/execute" => serde_json::json!({
                        "jsonrpc": "2.0",
                        "id": id,
                        "result": { "ok": true }
                    }),
                    _ => serde_json::json!({
                        "jsonrpc": "2.0",
                        "id": id,
                        "error": { "code": -32601, "message": "method not found" }
                    }),
                };
                let mut p = self.inner.lock().unwrap();
                let body = response.to_string();
                p.inbox.push_back(body);
            }
        }
        Ok(())
    }

    fn try_recv_frame(&mut self) -> Result<Option<String>, String> {
        self.inner.lock().unwrap().try_recv_frame()
    }

    fn is_alive(&mut self) -> bool {
        self.inner.lock().unwrap().is_alive()
    }

    fn kill(&mut self) {
        self.inner.lock().unwrap().kill();
    }
}

impl ProcessBackend for MemProcessBackend {
    fn spawn(
        &mut self,
        plugin_id: &str,
        command: &Path,
        _args: &[String],
        _workdir: &Path,
    ) -> Result<Box<dyn PluginProcess>, String> {
        self.spawned.push((plugin_id.to_string(), command.to_path_buf()));
        let proc = self
            .processes
            .entry(plugin_id.to_string())
            .or_insert_with(|| std::sync::Arc::new(std::sync::Mutex::new(MemProcess::new())))
            .clone();
        {
            let mut p = proc.lock().unwrap();
            p.alive = true;
        }
        Ok(Box::new(AutoMemProcess {
            inner: proc,
            plugin_id: plugin_id.to_string(),
            auto: self.auto_respond,
        }))
    }
}

#[cfg(test)]
mod mock_eval_tests {
    use super::*;

    #[test]
    fn eval_simple_expr_cases() {
        let eval = |s: &str| eval_expression(s).ok();
        assert_eq!(eval("1+2"), Some(3.0));
        let v = eval("100*1.13").expect("eval");
        assert!((v - 113.0).abs() < 1e-6, "{v}");
        assert_eq!(eval("2*3"), Some(6.0));
        assert_eq!(eval("1+1*(2+1)"), Some(4.0));
        assert_eq!(eval("(1+2)*(3+4)"), Some(21.0));
        assert_eq!(eval("10/4"), Some(2.5));
        assert_eq!(eval("1/0"), None);
        assert_eq!(eval("1+1(1+1)"), Some(3.0));
        assert_eq!(eval("2(3+4)"), Some(14.0));
        assert_eq!(eval("2^3"), Some(8.0));
        assert_eq!(eval("√9"), Some(3.0));
        assert_eq!(eval("1000,000+1"), Some(1_000_001.0));
        assert_eq!(eval("1,000,000+1"), Some(1_000_001.0));
        assert_eq!(eval("1,000.5*2"), Some(2001.0));
        assert_eq!(eval("=1^2"), Some(1.0));
        assert_eq!(eval("=1(2)"), Some(2.0));
        assert_eq!(eval("=1+1(2+2)"), Some(5.0));
        assert_eq!(eval("=1/1"), Some(1.0));
        assert_eq!(format_number(113.0), "113");
        assert_eq!(format_number(v), "113");
        assert_eq!(format_number(2.5), "2.5");
    }
}
