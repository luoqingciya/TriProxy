// TriProxy — a modern multi-core proxy client.
// Copyright (C) 2026 luoqingci
// SPDX-License-Identifier: GPL-3.0-or-later
//
// This program is free software: you can redistribute it and/or modify it under
// the terms of the GNU General Public License as published by the Free Software
// Foundation, either version 3 of the License, or (at your option) any later
// version. It is distributed in the hope that it will be useful, but WITHOUT ANY
// WARRANTY; without even the implied warranty of MERCHANTABILITY or FITNESS FOR A
// PARTICULAR PURPOSE. See LICENSE for the full text.
//
// Additional terms under GNU GPL v3 section 7 apply: see ADDITIONAL-TERMS.md.

//! Minimal YAML writer for the Mihomo config.
//!
//! Only the subset Mihomo needs is supported (maps, lists, scalars), and every
//! string is emitted double-quoted. Node names come from subscriptions and can
//! contain `:`, `#`, quotes or CJK text, so "write it plainly and hope" is not
//! an option — a mis-emitted name would silently change the config's meaning.
//!
//! A dedicated writer is used instead of a YAML crate because this project only
//! ever *writes* YAML, and the maintained serialisers all pull in a full parser.

#[derive(Debug, Clone, PartialEq)]
pub enum Yaml {
    Str(String),
    Int(i64),
    Bool(bool),
    List(Vec<Yaml>),
    Map(Vec<(String, Yaml)>),
}

impl Yaml {
    pub fn map() -> Self {
        Yaml::Map(Vec::new())
    }

    pub fn str(value: impl Into<String>) -> Self {
        Yaml::Str(value.into())
    }

    pub fn int(value: impl Into<i64>) -> Self {
        Yaml::Int(value.into())
    }

    pub fn bool(value: bool) -> Self {
        Yaml::Bool(value)
    }

    pub fn list() -> Self {
        Yaml::List(Vec::new())
    }

    pub fn strings<I, S>(values: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        Yaml::List(values.into_iter().map(|v| Yaml::Str(v.into())).collect())
    }

    /// Append a key to a map. Ignored (but not fatal) on other variants.
    pub fn with(mut self, key: impl Into<String>, value: Yaml) -> Self {
        if let Yaml::Map(entries) = &mut self {
            entries.push((key.into(), value));
        }
        self
    }

    /// Append a key only when the value is present.
    pub fn with_opt(self, key: impl Into<String>, value: Option<Yaml>) -> Self {
        match value {
            Some(value) => self.with(key, value),
            None => self,
        }
    }

    pub fn push(mut self, value: Yaml) -> Self {
        if let Yaml::List(items) = &mut self {
            items.push(value);
        }
        self
    }

    pub fn is_empty(&self) -> bool {
        match self {
            Yaml::Map(entries) => entries.is_empty(),
            Yaml::List(items) => items.is_empty(),
            _ => false,
        }
    }
}

/// Render a document. Always ends with a trailing newline.
pub fn emit(document: &Yaml) -> String {
    let mut out = String::new();
    match document {
        Yaml::Map(entries) if entries.is_empty() => out.push_str("{}\n"),
        _ => emit_value(document, 0, &mut out),
    }
    out
}

fn emit_value(value: &Yaml, indent: usize, out: &mut String) {
    match value {
        Yaml::Map(entries) => {
            for (key, item) in entries {
                push_indent(out, indent);
                out.push_str(&quote(key));
                out.push(':');
                emit_after_key(item, indent, out);
            }
        }
        Yaml::List(items) => {
            for item in items {
                push_indent(out, indent);
                out.push_str("- ");
                emit_list_item(item, indent, out);
            }
        }
        scalar => {
            out.push_str(&scalar_text(scalar));
            out.push('\n');
        }
    }
}

/// Emit the value that follows `key:` or `- `.
fn emit_after_key(value: &Yaml, indent: usize, out: &mut String) {
    match value {
        Yaml::Map(entries) if entries.is_empty() => out.push_str(" {}\n"),
        Yaml::List(items) if items.is_empty() => out.push_str(" []\n"),
        Yaml::Map(_) | Yaml::List(_) => {
            out.push('\n');
            emit_value(value, indent + 2, out);
        }
        scalar => {
            out.push(' ');
            out.push_str(&scalar_text(scalar));
            out.push('\n');
        }
    }
}

/// Emit a list item. A map starts on the dash line (`- key: value`) so that the
/// output matches hand-written Mihomo configs.
fn emit_list_item(item: &Yaml, indent: usize, out: &mut String) {
    match item {
        Yaml::Map(entries) if !entries.is_empty() => {
            for (position, (key, value)) in entries.iter().enumerate() {
                if position > 0 {
                    push_indent(out, indent + 2);
                }
                out.push_str(&quote(key));
                out.push(':');
                emit_after_key(value, indent + 2, out);
            }
        }
        Yaml::Map(_) => out.push_str("{}\n"),
        Yaml::List(_) => {
            out.push('\n');
            emit_value(item, indent + 2, out);
        }
        scalar => {
            out.push_str(&scalar_text(scalar));
            out.push('\n');
        }
    }
}

fn push_indent(out: &mut String, indent: usize) {
    for _ in 0..indent {
        out.push(' ');
    }
}

fn scalar_text(value: &Yaml) -> String {
    match value {
        Yaml::Str(text) => quote(text),
        Yaml::Int(number) => number.to_string(),
        Yaml::Bool(flag) => flag.to_string(),
        Yaml::List(_) | Yaml::Map(_) => String::new(),
    }
}

/// Double-quote and escape. Escaping `\` and `"` plus control characters is
/// enough for YAML double-quoted scalars.
fn quote(text: &str) -> String {
    let mut out = String::with_capacity(text.len() + 2);
    out.push('"');
    for ch in text.chars() {
        match ch {
            '\\' => out.push_str("\\\\"),
            '"' => out.push_str("\\\""),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            ch if ch.is_control() => out.push_str(&format!("\\x{:02x}", ch as u32)),
            ch => out.push(ch),
        }
    }
    out.push('"');
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn emits_nested_map_and_list_of_maps() {
        let document = Yaml::map()
            .with("mixed-port", Yaml::int(7890))
            .with("allow-lan", Yaml::bool(false))
            .with(
                "proxies",
                Yaml::list()
                    .push(
                        Yaml::map()
                            .with("name", Yaml::str("Tokyo 01"))
                            .with("port", Yaml::int(443)),
                    )
                    .push(Yaml::map().with("name", Yaml::str("HK"))),
            )
            .with("rules", Yaml::strings(["MATCH,PROXY"]));

        let text = emit(&document);
        assert_eq!(
            text,
            "\
\"mixed-port\": 7890
\"allow-lan\": false
\"proxies\":
  - \"name\": \"Tokyo 01\"
    \"port\": 443
  - \"name\": \"HK\"
\"rules\":
  - \"MATCH,PROXY\"
"
        );
    }

    #[test]
    fn nested_map_inside_list_item_is_indented() {
        let document = Yaml::map().with(
            "proxies",
            Yaml::list().push(
                Yaml::map().with("name", Yaml::str("n")).with(
                    "ws-opts",
                    Yaml::map()
                        .with("path", Yaml::str("/ws"))
                        .with("headers", Yaml::map().with("Host", Yaml::str("cdn"))),
                ),
            ),
        );
        assert_eq!(
            emit(&document),
            "\
\"proxies\":
  - \"name\": \"n\"
    \"ws-opts\":
      \"path\": \"/ws\"
      \"headers\":
        \"Host\": \"cdn\"
"
        );
    }

    #[test]
    fn empty_containers_render_as_flow_collections() {
        let document = Yaml::map()
            .with("proxies", Yaml::list())
            .with("proxy-groups", Yaml::map());
        assert_eq!(emit(&document), "\"proxies\": []\n\"proxy-groups\": {}\n");
    }

    #[test]
    fn strings_with_yaml_metacharacters_are_escaped() {
        let document = Yaml::map().with("name", Yaml::str("a: b #c \"d\" \\e\nf\tg"));
        let text = emit(&document);
        assert!(text.contains(r#""a: b #c \"d\" \\e\nf\tg""#), "{text}");
    }

    #[test]
    fn cjk_names_survive_roundtrip_as_escaped_quotes() {
        let document = Yaml::map().with("name", Yaml::str("日本 01 🇯🇵"));
        assert_eq!(emit(&document), "\"name\": \"日本 01 🇯🇵\"\n");
    }

    #[test]
    fn control_characters_are_escaped() {
        let document = Yaml::map().with("k", Yaml::str("a\u{1}b"));
        assert_eq!(emit(&document), "\"k\": \"a\\x01b\"\n");
    }
}
