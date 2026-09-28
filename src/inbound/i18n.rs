//! Stateless UI translation catalogs for 29 languages.

use std::collections::HashMap;
use std::sync::LazyLock;

const CATALOGS_JSON: &[(&str, &str)] = &[
    ("bg", include_str!("../../assets/i18n/bg.json")),
    ("ca", include_str!("../../assets/i18n/ca.json")),
    ("cs", include_str!("../../assets/i18n/cs.json")),
    ("da", include_str!("../../assets/i18n/da.json")),
    ("de", include_str!("../../assets/i18n/de.json")),
    ("el", include_str!("../../assets/i18n/el.json")),
    ("en", include_str!("../../assets/i18n/en.json")),
    ("es", include_str!("../../assets/i18n/es.json")),
    ("fi", include_str!("../../assets/i18n/fi.json")),
    ("fr", include_str!("../../assets/i18n/fr.json")),
    ("hr", include_str!("../../assets/i18n/hr.json")),
    ("hu", include_str!("../../assets/i18n/hu.json")),
    ("it", include_str!("../../assets/i18n/it.json")),
    ("ja", include_str!("../../assets/i18n/ja.json")),
    ("ko", include_str!("../../assets/i18n/ko.json")),
    ("nl", include_str!("../../assets/i18n/nl.json")),
    ("no", include_str!("../../assets/i18n/no.json")),
    ("pb", include_str!("../../assets/i18n/pb.json")),
    ("pl", include_str!("../../assets/i18n/pl.json")),
    ("pt", include_str!("../../assets/i18n/pt.json")),
    ("ro", include_str!("../../assets/i18n/ro.json")),
    ("ru", include_str!("../../assets/i18n/ru.json")),
    ("sk", include_str!("../../assets/i18n/sk.json")),
    ("sr", include_str!("../../assets/i18n/sr.json")),
    ("sv", include_str!("../../assets/i18n/sv.json")),
    ("tr", include_str!("../../assets/i18n/tr.json")),
    ("tw", include_str!("../../assets/i18n/tw.json")),
    ("uk", include_str!("../../assets/i18n/uk.json")),
    ("zh", include_str!("../../assets/i18n/zh.json")),
];

const LANGUAGE_NAMES_JSON: &str = include_str!("../../assets/i18n/languages.json");

static CATALOGS: LazyLock<HashMap<String, HashMap<String, String>>> = LazyLock::new(|| {
    CATALOGS_JSON
        .iter()
        .map(|(code, json)| {
            let catalog: HashMap<String, String> =
                serde_json::from_str(json).expect("embedded i18n catalogs are valid JSON");
            ((*code).to_owned(), catalog)
        })
        .collect()
});
static LANGUAGE_NAMES: LazyLock<HashMap<String, String>> = LazyLock::new(|| {
    serde_json::from_str(LANGUAGE_NAMES_JSON).expect("embedded i18n names are valid JSON")
});

fn normalize_language(language: &str) -> String {
    let normalized = language.trim().to_ascii_lowercase().replace('_', "-");
    let primary = normalized
        .split('-')
        .next()
        .filter(|code| !code.is_empty())
        .unwrap_or("en");
    let code = if primary == "pt" && normalized.split('-').any(|part| part == "br") {
        "pb"
    } else {
        primary
    };
    if CATALOGS.contains_key(code) {
        code.to_owned()
    } else {
        "en".to_owned()
    }
}

/// An owned, deterministic translation context. It can be held by each UI or
/// request independently, so one caller cannot change another caller's locale.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Translator {
    language: String,
}

impl Default for Translator {
    fn default() -> Self {
        Self::new("en")
    }
}

impl Translator {
    pub fn new(language: &str) -> Self {
        Self {
            language: normalize_language(language),
        }
    }

    /// Change only this translator's locale and return its normalized code.
    pub fn set_language(&mut self, language: &str) -> String {
        self.language = normalize_language(language);
        self.language.clone()
    }

    pub fn language(&self) -> &str {
        &self.language
    }

    pub fn t(&self, key: &str) -> String {
        translate(&self.language, key)
    }

    pub fn t_args(&self, key: &str, arguments: &[(&str, &str)]) -> String {
        translate_args(&self.language, key, arguments)
    }
}

/// Translate with explicit language state. Unknown languages and keys fall
/// back to English, then to the key itself.
pub fn translate(language: &str, key: &str) -> String {
    let language = normalize_language(language);
    let canonical = match key {
        "button.open" => "open",
        "button.save" => "output.save",
        "button.save_plus" => "save_as",
        "button.save_config" => "save_config",
        "button.reprocess" => "reprocess",
        "button.preview" => "preview",
        "button.scan" => "scan",
        "button.scan_plus" => return format!("{} +1", translate(&language, "scan")),
        "button.cancel" => "cancel",
        "button.rotate_l" => return "↶".into(),
        "button.rotate_r" => return "↷".into(),
        "button.zoom_in" => return "+".into(),
        "button.zoom_out" => return "−".into(),
        "button.prev_frame" => return "‹".into(),
        "button.next_frame" => return "›".into(),
        "button.ocr" => "ocr",
        "menu.view" => "menu.image",
        other => other,
    };
    CATALOGS
        .get(&language)
        .and_then(|catalog| catalog.get(canonical))
        .or_else(|| {
            CATALOGS
                .get("en")
                .and_then(|catalog| catalog.get(canonical))
        })
        .cloned()
        .unwrap_or_else(|| canonical.to_owned())
}

/// Translate and substitute the archived catalog's named `{placeholder}` values.
/// A malformed or incomplete argument list leaves the translated template intact.
pub fn translate_args(language: &str, key: &str, arguments: &[(&str, &str)]) -> String {
    let mut text = translate(language, key);
    for (name, value) in arguments {
        text = text.replace(&format!("{{{name}}}"), value);
    }
    text
}

pub fn available_languages() -> Vec<String> {
    let mut languages = CATALOGS.keys().cloned().collect::<Vec<_>>();
    languages.sort_unstable();
    languages
}

pub fn language_name(code: &str) -> String {
    LANGUAGE_NAMES
        .get(&normalize_language(code))
        .cloned()
        .unwrap_or_else(|| code.to_owned())
}

/// Return catalog codes whose key set diverges from the English authority.
pub fn validate_catalogs() -> Vec<String> {
    let english = CATALOGS.get("en").expect("English catalog is embedded");
    let mut problems = CATALOGS
        .iter()
        .filter(|(_, catalog)| {
            catalog.len() != english.len() || !catalog.keys().all(|key| english.contains_key(key))
        })
        .map(|(code, _)| code.clone())
        .collect::<Vec<_>>();
    problems.sort_unstable();
    problems
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn languages_and_catalogs_cover_the_same_codes() {
        let mut catalog_codes = CATALOGS.keys().cloned().collect::<Vec<_>>();
        let mut name_codes = LANGUAGE_NAMES.keys().cloned().collect::<Vec<_>>();
        catalog_codes.sort_unstable();
        name_codes.sort_unstable();
        assert_eq!(catalog_codes, name_codes);
    }

    #[test]
    fn every_catalog_shares_the_english_key_set() {
        assert!(
            validate_catalogs().is_empty(),
            "catalogs diverging from the English key set: {:?}",
            validate_catalogs()
        );
    }

    #[test]
    fn known_lookups_match_the_previously_embedded_catalogs() {
        assert_eq!(translate("en", "scan"), "Scan");
        assert_eq!(translate("en", "cancel"), "Cancel");
        assert_eq!(translate("en", "save_as"), "Save As…");
        assert_eq!(translate("de", "scan"), "Scannen");
        assert_eq!(translate("de", "cancel"), "Abbrechen");
        assert_eq!(translate("de", "save_as"), "Speichern unter…");
        assert_eq!(translate("ja", "scan"), "スキャン");
        assert_eq!(translate("bg", "scan"), "Сканиране");
        assert_eq!(translate("fr", "save_as"), "Enregistrer sous…");
        assert_eq!(translate("ru", "cancel"), "Отмена");
        assert_eq!(language_name("de"), "Deutsch");
        assert_eq!(language_name("ja"), "日本語");
        assert_eq!(language_name("bg"), "Български");
    }
}
