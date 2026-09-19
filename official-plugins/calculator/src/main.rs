//! 官方 Calculator 插件：prefix `=`，Panel，默认 Enter 复制结果。
//!
//! 兼容常见输入：
//! - 整数/小数、千分位 `,`/`_`、全角符号
//! - `+ - * / %`，`× ÷`，`^`/`**` 幂，`√` 开方
//! - 括号 `()` `[]`（含全角），一元正负号
//! - **隐式乘法**：`1+1(1+1)`、`2(3+4)`、`(1+2)(3)`、`2[3]`

use serde_json::{json, Value};

fn main() {
    let stdout = std::io::stdout();
    let mut stdout = stdout.lock();
    let _ = kite_plugin_sdk::serve_loop(
        &mut stdout,
        |_provider, query| eval_panel(query),
        |_action_id, _payload| Ok(json!({"ok": true})),
    );
}

fn eval_panel(query: &str) -> Value {
    let expr = query.trim();
    if expr.is_empty() {
        return kite_plugin_sdk::empty_response();
    }
    match eval_expression(expr) {
        Ok(v) => {
            let text = format_number(v);
            kite_plugin_sdk::panel_response(json!({
                "blocks": [
                    { "type": "text", "text": pretty_expr(expr), "style": "secondary" },
                    { "type": "value", "value": text, "selectable": true }
                ],
                "actions": [{
                    "id": "copy",
                    "label": "复制结果",
                    "shortcut": "Enter",
                    "default": true,
                    "action": { "type": "copy_text", "text": text }
                }]
            }))
        }
        Err(msg) => kite_plugin_sdk::panel_response(json!({
            "blocks": [{ "type": "notice", "level": "error", "text": msg }],
            "actions": []
        })),
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

/// 符号归一；保留空格（`1 2` 靠解析器识别，不静默拼成 12）。
fn normalize_expr(expr: &str) -> String {
    let mut out = String::with_capacity(expr.len());
    let chars: Vec<char> = expr.chars().collect();
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        // `**` → `^`
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
            // 兼容部分输入把中括号当分组
            '[' | '〔' => out.push('('),
            ']' | '〕' => out.push(')'),
            '√' => out.push('√'),
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
    // 正常协议传入的是 effective query（不含 `=`），因此这里只剥离一个前导前缀。
    normalized.strip_prefix('=').unwrap_or(normalized).trim().to_string()
}

pub(crate) fn eval_expression(expr: &str) -> Result<f64, &'static str> {
    let src = normalize_expr(expr);
    if src.is_empty() {
        return Err("表达式不完整");
    }
    let chars: Vec<char> = src.chars().collect();
    let mut p = Parser { chars, pos: 0 };
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

struct Parser {
    chars: Vec<char>,
    pos: usize,
}

impl Parser {
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

    /// expr := term (('+'|'-') term)*
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

    /// term := unary ((*|/|%) unary | 隐式乘法 unary)*
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
                // 隐式乘法：2(3) / (1+2)(3) / 2(3+4) / )( / 2√9
                Some(c) if starts_implicit_factor(c) => {
                    left *= self.parse_unary()?;
                }
                _ => break,
            }
        }
        Ok(left)
    }

    /// unary := ('+'|'-')* power
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

    /// power := primary ('^' unary)?  右结合，支持 2^-1、2^3^2
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

    /// primary := number | '(' expr ')' | '√' unary
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
        // 千分位 `,` / `_`（如 1,000,000 或 1000,000）
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

/// 因子开头：可被隐式乘法连接（数字/分组/开方）。
fn starts_implicit_factor(c: char) -> bool {
    c == '(' || c == '√' || c.is_ascii_digit() || c == '.'
}

#[cfg(test)]
mod tests {
    use super::*;

    fn approx(expr: &str, expect: f64) {
        let got = eval_expression(expr).unwrap_or_else(|e| panic!("{expr} => {e}"));
        assert!(
            (got - expect).abs() < 1e-9,
            "{expr} => {got}, expect {expect}"
        );
    }

    #[test]
    fn basic_ops() {
        approx("1+2", 3.0);
        approx("100*1.13", 113.0);
        approx("2*3", 6.0);
        approx("10/4", 2.5);
        approx("7-10", -3.0);
        approx("-5+3", -2.0);
        approx("+8", 8.0);
        approx("3.5*2", 7.0);
        approx("7%3", 1.0);
    }

    #[test]
    fn precedence_and_parens() {
        approx("1+1*(2+1)", 4.0);
        approx("1+2*3", 7.0);
        approx("(1+2)*(3+4)", 21.0);
        approx("2*(3+4)/2", 7.0);
        approx("((1+2))*3", 9.0);
        approx("10-2*3", 4.0);
        approx("(10-2)*3", 24.0);
        approx("1+2*3-4/2", 5.0);
        approx("-(2+3)", -5.0);
        approx("2*-3", -6.0);
    }

    #[test]
    fn implicit_multiplication() {
        approx("1+1(1+1)", 3.0); // 1 + 1*(1+1) = 3
        approx("2(3+4)", 14.0);
        approx("(1+2)(3+4)", 21.0);
        approx("(1+2)3", 9.0);
        approx("2(3)", 6.0);
        approx("2 (3+4)", 14.0);
        approx("1/2(3)", 1.5); // (1/2)*3
        approx("2[3+4]", 14.0);
        approx("(2+1)(2+1)", 9.0);
        approx("3(2)(4)", 24.0);
    }

    #[test]
    fn power_and_sqrt() {
        approx("2^3", 8.0);
        approx("2**3", 8.0);
        approx("2^3^2", 512.0); // 右结合
        approx("2^-1", 0.5);
        approx("√9", 3.0);
        approx("√(9)", 3.0);
        approx("√9+1", 4.0);
        approx("2√9", 6.0); // 隐式乘
        approx("(√16)^2", 16.0);
    }

    #[test]
    fn accepts_calculator_prefix_from_legacy_host() {
        approx("=1^2", 1.0);
        approx("=1(2)", 2.0);
        approx("=1+1(2+2)", 5.0);
        approx("=1/1", 1.0);
    }

    #[test]
    fn thousand_separators() {
        approx("1000,000+1", 1_000_001.0);
        approx("1,000,000+1", 1_000_001.0);
        approx("1,000+2", 1002.0);
        approx("1_000_000-1", 999_999.0);
        approx("1,000.5*2", 2001.0);
        approx("1000，000+1", 1_000_001.0);
        approx("2,500*4", 10_000.0);
        approx("(1,000+500)*2", 3000.0);
    }

    #[test]
    fn fullwidth_and_symbols() {
        approx("1+1×(2+1)", 4.0);
        approx("（2+1）*2", 6.0);
        approx("6÷2", 3.0);
        approx("1＋2＊（3＋1）", 9.0);
        approx("1+1(2+1)×2", 7.0); // 1 + 1*(2+1)*2 = 7
    }

    #[test]
    fn errors() {
        assert_eq!(eval_expression(""), Err("表达式不完整"));
        assert_eq!(eval_expression("1+"), Err("表达式不完整"));
        assert_eq!(eval_expression("(1+2"), Err("表达式不完整"));
        assert_eq!(eval_expression("()"), Err("表达式不完整"));
        assert_eq!(eval_expression("1++"), Err("表达式不完整"));
        assert_eq!(eval_expression("abc"), Err("表达式无效"));
        assert_eq!(eval_expression("1 2"), Ok(2.0)); // 隐式乘 1*2
        assert_eq!(eval_expression("1/0"), Err("不能除以 0"));
        assert_eq!(eval_expression("√(-1)"), Err("负数不能开方"));
    }

    #[test]
    fn panel_and_format() {
        assert_eq!(format_number(113.0), "113");
        assert_eq!(format_number(2.5), "2.5");
        let panel = eval_panel("1+1(1+1)");
        assert_eq!(panel["type"], "panel");
        assert_eq!(panel["panel"]["blocks"][1]["value"], "3");
        assert_eq!(panel["panel"]["actions"][0]["action"]["type"], "copy_text");
        assert_eq!(eval_expression("1+2"), Ok(3.0));
        assert_eq!(eval_expression("1+1(2+1)"), Ok(4.0));
    }
}
