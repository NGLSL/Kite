//! 当前 Windows 的设置页搜索词。只读取 XML 中已有白名单 URI 的页面级资源，
//! 不把系统搜索索引里的控件或未验证 URI 变成 Kite 结果。

use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::PathBuf;

use roxmltree::{Document, Node};
use windows::core::PCWSTR;
use windows::Win32::UI::Shell::SHLoadIndirectString;

const RESOURCE_PREFIX: &str =
    "@{windows?ms-resource://Windows.UI.SettingsAppThreshold/SearchResources/";

pub(super) fn load_search_terms<'a>(
    allowed_uris: impl IntoIterator<Item = &'a str>,
) -> HashMap<String, Vec<String>> {
    let allowed: HashSet<String> = allowed_uris
        .into_iter()
        .filter_map(|uri| uri.strip_prefix("ms-settings:"))
        .map(str::to_string)
        .collect();
    if allowed.is_empty() {
        return HashMap::new();
    }

    let windows_root = std::env::var_os("SystemRoot")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(r"C:\Windows"));
    let settings_dir = windows_root.join("ImmersiveControlPanel").join("Settings");
    let Ok(entries) = fs::read_dir(settings_dir) else {
        return HashMap::new();
    };
    let mut files: Vec<PathBuf> = entries
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .filter(|path| {
            path.file_name()
                .and_then(|name| name.to_str())
                .is_some_and(|name| {
                    name.starts_with("AllSystemSettings_") && name.ends_with(".xml")
                })
        })
        .collect();
    files.sort();

    let mut out = HashMap::new();
    let mut page_ids = HashMap::new();
    let mut ambiguous = HashSet::new();
    for file in files {
        let Ok(xml) = fs::read_to_string(file) else {
            continue;
        };
        parse_page_terms(
            &xml,
            &allowed,
            &mut resolve_indirect_string,
            &mut out,
            &mut page_ids,
            &mut ambiguous,
        );
    }
    out.retain(|uri, _| !ambiguous.contains(uri));
    out
}

fn parse_page_terms<F: FnMut(&str) -> Option<String>>(
    xml: &str,
    allowed: &HashSet<String>,
    resolve: &mut F,
    out: &mut HashMap<String, Vec<String>>,
    page_ids: &mut HashMap<String, String>,
    ambiguous: &mut HashSet<String>,
) {
    let Ok(doc) = Document::parse(xml) else {
        return;
    };
    for content in doc
        .descendants()
        .filter(|node| node.has_tag_name("SearchableContent"))
    {
        let Some(filename) = child_text(content, "Filename") else {
            continue;
        };
        let Some(identity) = child(content, "SettingIdentity") else {
            continue;
        };
        let Some(paths) = child(identity, "SettingPaths") else {
            continue;
        };
        let Some(info) = child(content, "SettingInformation") else {
            continue;
        };
        for path in paths.children().filter(|node| node.has_tag_name("Path")) {
            let Some(page_id) = child_text(path, "PageID") else {
                continue;
            };
            // Page-level metadata only. Child controls reuse PolicyIds and are too noisy.
            if filename != format!("AAA_{page_id}") {
                continue;
            }
            let Some(policy_ids) = child_text(path, "PolicyIds") else {
                continue;
            };
            let ids: Vec<&str> = policy_ids
                .split(';')
                .map(str::trim)
                .filter(|id| allowed.contains(*id))
                .collect();
            if ids.is_empty() {
                continue;
            }

            let mut terms = Vec::new();
            for field in ["Description", "HighKeywords"] {
                let Some(reference) = child_text(info, field) else {
                    continue;
                };
                if !reference.starts_with(RESOURCE_PREFIX) {
                    continue;
                }
                let Some(value) = resolve(reference) else {
                    continue;
                };
                if value.starts_with('@') {
                    continue;
                }
                for term in value.split(';').map(str::trim) {
                    if (2..=80).contains(&term.chars().count())
                        && !terms.iter().any(|existing| existing == term)
                    {
                        terms.push(term.to_string());
                    }
                }
            }
            for id in ids {
                let uri = format!("ms-settings:{id}");
                if page_ids.get(&uri).is_some_and(|old| old != page_id) {
                    ambiguous.insert(uri);
                    continue;
                }
                page_ids.insert(uri.clone(), page_id.to_string());
                if !terms.is_empty() {
                    let values = out.entry(uri).or_insert_with(Vec::new);
                    for term in &terms {
                        if !values.contains(term) {
                            values.push(term.clone());
                        }
                    }
                }
            }
        }
    }
}

fn child<'a, 'input>(parent: Node<'a, 'input>, name: &str) -> Option<Node<'a, 'input>> {
    parent
        .children()
        .find(|node| node.is_element() && node.tag_name().name() == name)
}

fn child_text<'a, 'input>(parent: Node<'a, 'input>, name: &str) -> Option<&'a str> {
    child(parent, name)?.text().map(str::trim)
}

fn resolve_indirect_string(reference: &str) -> Option<String> {
    let source: Vec<u16> = reference.encode_utf16().chain(std::iter::once(0)).collect();
    let mut output = vec![0u16; 8192];
    unsafe {
        SHLoadIndirectString(PCWSTR(source.as_ptr()), &mut output, None).ok()?;
    }
    let end = output.iter().position(|unit| *unit == 0)?;
    let value = String::from_utf16_lossy(&output[..end]).trim().to_string();
    (!value.is_empty()).then_some(value)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_only_whitelisted_page_level_terms() {
        let xml = r#"
            <Root>
              <SearchableContent>
                <Filename>AAA_SettingsPageStartup</Filename>
                <SettingIdentity><SettingPaths><Path>
                  <PageID>SettingsPageStartup</PageID><PolicyIds>startupapps</PolicyIds>
                </Path></SettingPaths></SettingIdentity>
                <SettingInformation>
                  <Description>@{windows?ms-resource://Windows.UI.SettingsAppThreshold/SearchResources/SettingsPageStartup/Description}</Description>
                  <HighKeywords>@{windows?ms-resource://Windows.UI.SettingsAppThreshold/SearchResources/SettingsPageStartup/HighKeywords}</HighKeywords>
                </SettingInformation>
              </SearchableContent>
              <SearchableContent>
                <Filename>AAA_StartupChildControl</Filename>
                <SettingIdentity><SettingPaths><Path>
                  <PageID>SettingsPageStartup</PageID><PolicyIds>startupapps</PolicyIds>
                </Path></SettingPaths></SettingIdentity>
                <SettingInformation>
                  <HighKeywords>@{windows?ms-resource://Windows.UI.SettingsAppThreshold/SearchResources/Child/HighKeywords}</HighKeywords>
                </SettingInformation>
              </SearchableContent>
              <SearchableContent>
                <Filename>AAA_SettingsPageUnknown</Filename>
                <SettingIdentity><SettingPaths><Path>
                  <PageID>SettingsPageUnknown</PageID><PolicyIds>unknownpage</PolicyIds>
                </Path></SettingPaths></SettingIdentity>
                <SettingInformation>
                  <HighKeywords>@{windows?ms-resource://Windows.UI.SettingsAppThreshold/SearchResources/Unknown/HighKeywords}</HighKeywords>
                </SettingInformation>
              </SearchableContent>
            </Root>
        "#;
        let allowed = HashSet::from(["startupapps".to_string()]);
        let mut resolve = |reference: &str| {
            if reference.contains("/Child/") {
                Some("child only".to_string())
            } else if reference.ends_with("/Description}") {
                Some("启动应用".to_string())
            } else if reference.ends_with("/HighKeywords}") {
                Some("启动任务;startup tasks".to_string())
            } else {
                None
            }
        };
        let mut out = HashMap::new();
        parse_page_terms(
            xml,
            &allowed,
            &mut resolve,
            &mut out,
            &mut HashMap::new(),
            &mut HashSet::new(),
        );
        assert_eq!(
            out.get("ms-settings:startupapps"),
            Some(&vec![
                "启动应用".to_string(),
                "启动任务".to_string(),
                "startup tasks".to_string()
            ])
        );
        assert_eq!(out.len(), 1);
    }

    #[test]
    fn invalid_xml_or_missing_resource_does_not_remove_other_pages() {
        let mut out = HashMap::from([(
            "ms-settings:startupapps".to_string(),
            vec!["启动应用".to_string()],
        )]);
        parse_page_terms(
            "<invalid",
            &HashSet::from(["startupapps".to_string()]),
            &mut |_| None,
            &mut out,
            &mut HashMap::new(),
            &mut HashSet::new(),
        );
        let valid_xml = r#"
            <Root><SearchableContent>
              <Filename>AAA_SettingsPageStartup</Filename>
              <SettingIdentity><SettingPaths><Path>
                <PageID>SettingsPageStartup</PageID><PolicyIds>startupapps</PolicyIds>
              </Path></SettingPaths></SettingIdentity>
              <SettingInformation>
                <Description>@{windows?ms-resource://Windows.UI.SettingsAppThreshold/SearchResources/SettingsPageStartup/Description}</Description>
              </SettingInformation>
            </SearchableContent></Root>
        "#;
        parse_page_terms(
            valid_xml,
            &HashSet::from(["startupapps".to_string()]),
            &mut |_| None,
            &mut out,
            &mut HashMap::new(),
            &mut HashSet::new(),
        );
        assert_eq!(out["ms-settings:startupapps"], ["启动应用"]);
    }

    #[test]
    #[ignore = "Requires a Windows installation with a Settings search resource for startupapps"]
    fn reads_real_windows_startup_terms() {
        let terms = load_search_terms(["ms-settings:startupapps"]);
        assert!(
            terms
                .get("ms-settings:startupapps")
                .is_some_and(|values| !values.is_empty()),
            "current Windows should expose localized startup search terms"
        );
    }
}
