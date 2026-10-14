// Copyright (C) 2026 Wilson Glasser and Oryxis contributors
// SPDX-License-Identifier: AGPL-3.0-or-later
// Source: crates/oryxis-app/src/i18n/mod.rs (detect_os_language)
// Oryxis 16ad2af83b424780ff60b6420a6c20a3b30c7143.
// Modified 2026-10-14: two-language application setting, primary OS locale only,
// shared JSON presentation catalog; no global mutable language or iced glue.
use serde::{Deserialize, Serialize};
use std::{collections::HashMap, sync::OnceLock};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum Language {
    #[default]
    #[serde(rename = "system")]
    System,
    #[serde(rename = "en")]
    English,
    #[serde(rename = "zh-CN")]
    Chinese,
}

impl Language {
    pub fn for_locale(locale: &str) -> Self {
        if locale
            .split(['-', '_', '.'])
            .next()
            .is_some_and(|s| s.eq_ignore_ascii_case("zh"))
        {
            Self::Chinese
        } else {
            Self::English
        }
    }

    pub fn resolved(self) -> Self {
        match self {
            Self::System => detect_os_language(),
            other => other,
        }
    }
}

/// Like Oryxis, cache the OS baseline for the process. Only the primary locale
/// is considered: non-Chinese systems use English, even with Chinese secondary.
pub fn detect_os_language() -> Language {
    static DETECTED: OnceLock<Language> = OnceLock::new();
    *DETECTED.get_or_init(|| {
        sys_locale::get_locale()
            .map(|tag| Language::for_locale(&tag))
            .unwrap_or(Language::English)
    })
}

pub fn text(language: Language, english: &str) -> &str {
    if language.resolved() != Language::Chinese {
        return english;
    }
    static CHINESE: OnceLock<HashMap<String, String>> = OnceLock::new();
    CHINESE
        .get_or_init(|| {
            serde_json::from_str(include_str!("../../../src/locales/zh-CN.json"))
                .expect("valid translation catalog")
        })
        .get(english)
        .map(String::as_str)
        .unwrap_or(english)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn primary_locale_and_explicit_choices() {
        for tag in ["zh", "zh-CN", "zh-TW", "zh-Hant-HK", "ZH_cn.UTF-8"] {
            assert_eq!(Language::for_locale(tag), Language::Chinese);
        }
        for tag in ["en-US", "ja-JP", "de-DE", "", "zho", "fr-CH"] {
            assert_eq!(Language::for_locale(tag), Language::English);
        }
        assert_eq!(Language::English.resolved(), Language::English);
        assert_eq!(Language::Chinese.resolved(), Language::Chinese);
        assert_eq!(text(Language::Chinese, "Start"), "启动");
        assert_eq!(text(Language::English, "Start"), "Start");
        assert_eq!(
            text(Language::Chinese, "Unmodified SSH diagnostic"),
            "Unmodified SSH diagnostic"
        );
    }
}
