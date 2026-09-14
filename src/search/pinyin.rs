//! 拼音预计算：索引阶段一次生成，搜索只做字符串比较。

use pinyin::ToPinyin;

/// 返回 (全拼无空格, 首字母)。非汉字转为小写字符；空白忽略。
pub fn precompute(text: &str) -> (String, String) {
    let mut full = String::new();
    let mut initials = String::new();

    for ch in text.chars() {
        if ch.is_whitespace() {
            continue;
        }
        let mut converted = false;
        // ToPinyin 实现在 &str 上
        for py in ch.to_string().as_str().to_pinyin() {
            if let Some(py) = py {
                let plain = py.plain();
                full.push_str(plain);
                if let Some(c) = plain.chars().next() {
                    initials.push(c);
                }
                converted = true;
                break;
            }
        }
        if !converted {
            let lower = ch.to_lowercase().next().unwrap_or(ch);
            full.push(lower);
            initials.push(lower);
        }
    }
    (full, initials)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn weixin_pinyin() {
        let (full, ini) = precompute("微信");
        assert_eq!(full, "weixin");
        assert_eq!(ini, "wx");
    }

    #[test]
    fn wechat_dev_tool_initials() {
        let (full, ini) = precompute("微信开发者工具");
        assert_eq!(full, "weixinkaifazhegongju");
        assert_eq!(ini, "wxkfzgj");
    }

    #[test]
    fn control_words_appear_inside_full_initials() {
        let (_, kzmb) = precompute("控制面板");
        assert_eq!(kzmb, "kzmb");
        assert!(kzmb.starts_with("kz"));

        let (_, xrkyckz) = precompute("向日葵远程控制");
        assert_eq!(xrkyckz, "xrkyckz");
        assert!(xrkyckz.contains("kz"), "「控制」首字母在整名末尾，不是前缀");

        let (_, jpkzsbsz) = precompute("键盘控制鼠标设置");
        assert!(jpkzsbsz.contains("kz"), "「控制」首字母在整名中间");
    }
}
