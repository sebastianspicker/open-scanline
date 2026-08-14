//! Open scanner manufacturer catalog (public USB VID + name tokens).

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

pub const REQUIRED_MANUFACTURER_IDS: &[&str] = &[
    "epson", "canon", "hp", "brother", "fujitsu", "plustek", "nikon", "kodak", "mustek", "samsung",
];
const CATALOG_SCOPE: &str = "manufacturer-family hints only; not detected device capabilities";

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SupportProfile {
    pub media_kinds: Vec<String>,
    pub supports_adf: bool,
    pub supports_ir: bool,
    pub preferred_backends: Vec<String>,
    pub notes: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ManufacturerEntry {
    pub id: String,
    pub canonical_name: String,
    pub usb_vids: Vec<String>,
    pub name_tokens: Vec<String>,
    pub profile: SupportProfile,
}

impl ManufacturerEntry {
    pub fn as_dict(&self) -> Value {
        json!({
            "id": self.id,
            "canonical_name": self.canonical_name,
            "usb_vids": self.usb_vids,
            "name_tokens": self.name_tokens,
            "profile": {
                "evidence_scope": CATALOG_SCOPE,
                "media_kinds": self.profile.media_kinds,
                "supports_adf": self.profile.supports_adf,
                "supports_ir": self.profile.supports_ir,
                "preferred_backends": self.profile.preferred_backends,
                "notes": self.profile.notes,
            },
        })
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ResolveResult {
    pub status: String,
    pub query: String,
    pub matched_by: Option<String>,
    pub message: String,
    pub manufacturer: Option<ManufacturerEntry>,
}

impl ResolveResult {
    pub fn as_dict(&self) -> Value {
        json!({
            "status": self.status,
            "query": self.query,
            "matched_by": self.matched_by,
            "message": self.message,
            "manufacturer": self.manufacturer.as_ref().map(|m| m.as_dict()),
        })
    }
}

struct SupportProfileSpec {
    media: &'static [&'static str],
    supports_adf: bool,
    supports_ir: bool,
    notes: &'static str,
}

struct ManufacturerSpec {
    id: &'static str,
    canonical_name: &'static str,
    usb_vids: &'static [&'static str],
    name_tokens: &'static [&'static str],
    profile: SupportProfileSpec,
}

impl ManufacturerSpec {
    fn entry(&self) -> ManufacturerEntry {
        ManufacturerEntry {
            id: self.id.into(),
            canonical_name: self.canonical_name.into(),
            usb_vids: self.usb_vids.iter().map(|value| (*value).into()).collect(),
            name_tokens: self
                .name_tokens
                .iter()
                .map(|value| (*value).into())
                .collect(),
            profile: SupportProfile {
                media_kinds: self
                    .profile
                    .media
                    .iter()
                    .map(|value| (*value).into())
                    .collect(),
                supports_adf: self.profile.supports_adf,
                supports_ir: self.profile.supports_ir,
                preferred_backends: vec!["wia".into(), "sane".into(), "escl".into()],
                notes: self.profile.notes.into(),
            },
        }
    }
}

const CATALOG: &[ManufacturerSpec] = &[
    ManufacturerSpec {
        id: "epson",
        canonical_name: "Epson",
        usb_vids: &["04b8"],
        name_tokens: &["epson", "perfection", "workforce"],
        profile: SupportProfileSpec {
            media: &["reflective", "film"],
            supports_adf: true,
            supports_ir: true,
            notes: "Film/transparency + eSCL common.",
        },
    },
    ManufacturerSpec {
        id: "canon",
        canonical_name: "Canon",
        usb_vids: &["04a9"],
        name_tokens: &["canon", "canoscan", "pixma"],
        profile: SupportProfileSpec {
            media: &["reflective", "film"],
            supports_adf: true,
            supports_ir: false,
            notes: "CanoScan film adapters.",
        },
    },
    ManufacturerSpec {
        id: "hp",
        canonical_name: "HP",
        usb_vids: &["03f0"],
        name_tokens: &["hp", "scanjet", "officejet"],
        profile: SupportProfileSpec {
            media: &["reflective"],
            supports_adf: true,
            supports_ir: false,
            notes: "ScanJet/OfficeJet.",
        },
    },
    ManufacturerSpec {
        id: "brother",
        canonical_name: "Brother",
        usb_vids: &["04f9"],
        name_tokens: &["brother", "ads-", "dcp-"],
        profile: SupportProfileSpec {
            media: &["reflective"],
            supports_adf: true,
            supports_ir: false,
            notes: "ADF document scanners.",
        },
    },
    ManufacturerSpec {
        id: "fujitsu",
        canonical_name: "Fujitsu",
        usb_vids: &["04c5"],
        name_tokens: &["fujitsu", "fi-", "scansnap"],
        profile: SupportProfileSpec {
            media: &["reflective"],
            supports_adf: true,
            supports_ir: false,
            notes: "fi- / ScanSnap document ADF.",
        },
    },
    ManufacturerSpec {
        id: "plustek",
        canonical_name: "Plustek",
        usb_vids: &["07b3"],
        name_tokens: &["plustek", "opticfilm"],
        profile: SupportProfileSpec {
            media: &["reflective", "film"],
            supports_adf: false,
            supports_ir: true,
            notes: "OpticFilm dedicated film scanners.",
        },
    },
    ManufacturerSpec {
        id: "nikon",
        canonical_name: "Nikon",
        usb_vids: &["04b0"],
        name_tokens: &["nikon", "coolscan"],
        profile: SupportProfileSpec {
            media: &["film"],
            supports_adf: false,
            supports_ir: true,
            notes: "Coolscan LS film scanners.",
        },
    },
    ManufacturerSpec {
        id: "kodak",
        canonical_name: "Kodak",
        usb_vids: &["040a"],
        name_tokens: &["kodak", "alaris"],
        profile: SupportProfileSpec {
            media: &["reflective"],
            supports_adf: true,
            supports_ir: false,
            notes: "Document production scanners.",
        },
    },
    ManufacturerSpec {
        id: "mustek",
        canonical_name: "Mustek",
        usb_vids: &["055f"],
        name_tokens: &["mustek", "bearpaw"],
        profile: SupportProfileSpec {
            media: &["reflective"],
            supports_adf: false,
            supports_ir: false,
            notes: "Consumer flatbeds.",
        },
    },
    ManufacturerSpec {
        id: "samsung",
        canonical_name: "Samsung",
        usb_vids: &["04e8"],
        name_tokens: &["samsung", "scx-", "xpress"],
        profile: SupportProfileSpec {
            media: &["reflective"],
            supports_adf: true,
            supports_ir: false,
            notes: "MFP scanners.",
        },
    },
];

fn catalog() -> Vec<ManufacturerEntry> {
    CATALOG.iter().map(ManufacturerSpec::entry).collect()
}

pub fn list_manufacturers() -> Vec<ManufacturerEntry> {
    catalog()
}

pub fn resolve_manufacturer(query: &str) -> ResolveResult {
    let q = query.trim();
    if q.is_empty() {
        return unknown_result(q, "empty query");
    }
    let lower = q.to_ascii_lowercase();
    let entries = catalog();
    resolve_by_vid(q, &extract_vid(&lower), &entries)
        .or_else(|| resolve_by_name(q, &lower, &entries))
        .unwrap_or_else(|| unknown_result(q, "no catalog manufacturer matched"))
}

fn unknown_result(query: &str, message: &str) -> ResolveResult {
    ResolveResult {
        status: "unknown".into(),
        query: query.into(),
        matched_by: None,
        message: message.into(),
        manufacturer: None,
    }
}

fn extract_vid(lower_query: &str) -> String {
    lower_query
        .chars()
        .filter(|character| character.is_ascii_hexdigit())
        .take(4)
        .collect()
}

fn resolve_by_vid(query: &str, vid: &str, entries: &[ManufacturerEntry]) -> Option<ResolveResult> {
    entries
        .iter()
        .find(|entry| vid.len() == 4 && entry.usb_vids.iter().any(|value| value == vid))
        .map(|entry| ResolveResult {
            status: "ok".into(),
            query: query.into(),
            matched_by: Some("vid".into()),
            message: format!("matched USB VID {vid}"),
            manufacturer: Some(entry.clone()),
        })
}

fn resolve_by_name(
    query: &str,
    lower_query: &str,
    entries: &[ManufacturerEntry],
) -> Option<ResolveResult> {
    entries
        .iter()
        .find(|entry| {
            entry
                .name_tokens
                .iter()
                .any(|token| token_matches(lower_query, token))
        })
        .map(|entry| ResolveResult {
            status: "ok".into(),
            query: query.into(),
            matched_by: Some("name_token".into()),
            message: format!("matched name token for {}", entry.canonical_name),
            manufacturer: Some(entry.clone()),
        })
}

fn token_matches(lower_query: &str, token: &str) -> bool {
    let lower_token = token.to_ascii_lowercase();
    if lower_token.len() > 3 {
        lower_query.contains(&lower_token)
    } else {
        lower_query
            .split(|character: char| !character.is_alphanumeric())
            .any(|part| part == lower_token)
    }
}

pub fn manufacturer_support_summary() -> Value {
    let entries = list_manufacturers();
    json!({
        "app": "open-scanline",
        "catalog": "open-manufacturer-support",
        "count": entries.len(),
        "required_ids": REQUIRED_MANUFACTURER_IDS,
        "evidence_scope": CATALOG_SCOPE,
        "manufacturers": entries.iter().map(|e| e.as_dict()).collect::<Vec<_>>(),
        "ok": true,
    })
}

pub fn format_manufacturers_text(entries: &[ManufacturerEntry]) -> String {
    let mut out = String::new();
    out.push_str("# manufacturer-family hints only; verify capabilities on the selected device\n");
    out.push_str("id\tname\tvids\tadf\tir\n");
    for e in entries {
        out.push_str(&format!(
            "{}\t{}\t{}\t{}\t{}\n",
            e.id,
            e.canonical_name,
            e.usb_vids.join(","),
            e.profile.supports_adf,
            e.profile.supports_ir
        ));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    type ExpectedManufacturer = (
        &'static str,
        &'static str,
        &'static [&'static str],
        &'static [&'static str],
        &'static [&'static str],
        bool,
        bool,
        &'static str,
    );

    #[test]
    fn catalog_order_and_data_are_stable() {
        let expected: &[ExpectedManufacturer] = &[
            (
                "epson",
                "Epson",
                &["04b8"],
                &["epson", "perfection", "workforce"],
                &["reflective", "film"],
                true,
                true,
                "Film/transparency + eSCL common.",
            ),
            (
                "canon",
                "Canon",
                &["04a9"],
                &["canon", "canoscan", "pixma"],
                &["reflective", "film"],
                true,
                false,
                "CanoScan film adapters.",
            ),
            (
                "hp",
                "HP",
                &["03f0"],
                &["hp", "scanjet", "officejet"],
                &["reflective"],
                true,
                false,
                "ScanJet/OfficeJet.",
            ),
            (
                "brother",
                "Brother",
                &["04f9"],
                &["brother", "ads-", "dcp-"],
                &["reflective"],
                true,
                false,
                "ADF document scanners.",
            ),
            (
                "fujitsu",
                "Fujitsu",
                &["04c5"],
                &["fujitsu", "fi-", "scansnap"],
                &["reflective"],
                true,
                false,
                "fi- / ScanSnap document ADF.",
            ),
            (
                "plustek",
                "Plustek",
                &["07b3"],
                &["plustek", "opticfilm"],
                &["reflective", "film"],
                false,
                true,
                "OpticFilm dedicated film scanners.",
            ),
            (
                "nikon",
                "Nikon",
                &["04b0"],
                &["nikon", "coolscan"],
                &["film"],
                false,
                true,
                "Coolscan LS film scanners.",
            ),
            (
                "kodak",
                "Kodak",
                &["040a"],
                &["kodak", "alaris"],
                &["reflective"],
                true,
                false,
                "Document production scanners.",
            ),
            (
                "mustek",
                "Mustek",
                &["055f"],
                &["mustek", "bearpaw"],
                &["reflective"],
                false,
                false,
                "Consumer flatbeds.",
            ),
            (
                "samsung",
                "Samsung",
                &["04e8"],
                &["samsung", "scx-", "xpress"],
                &["reflective"],
                true,
                false,
                "MFP scanners.",
            ),
        ];

        let entries = list_manufacturers();
        assert_eq!(entries.len(), expected.len());
        assert_eq!(REQUIRED_MANUFACTURER_IDS.len(), expected.len());
        for (entry, expected) in entries.iter().zip(expected) {
            assert_eq!(entry.id, expected.0);
            assert_eq!(entry.canonical_name, expected.1);
            assert_eq!(
                entry
                    .usb_vids
                    .iter()
                    .map(String::as_str)
                    .collect::<Vec<_>>(),
                expected.2
            );
            assert_eq!(
                entry
                    .name_tokens
                    .iter()
                    .map(String::as_str)
                    .collect::<Vec<_>>(),
                expected.3
            );
            assert_eq!(
                entry
                    .profile
                    .media_kinds
                    .iter()
                    .map(String::as_str)
                    .collect::<Vec<_>>(),
                expected.4
            );
            assert_eq!(entry.profile.supports_adf, expected.5);
            assert_eq!(entry.profile.supports_ir, expected.6);
            assert_eq!(entry.profile.preferred_backends, ["wia", "sane", "escl"]);
            assert_eq!(entry.profile.notes, expected.7);
        }
    }

    #[test]
    fn resolution_preserves_vid_name_short_token_and_unknown_results() {
        let vid = resolve_manufacturer("  04B8  ");
        assert_eq!(vid.status, "ok");
        assert_eq!(vid.query, "04B8");
        assert_eq!(vid.matched_by.as_deref(), Some("vid"));
        assert_eq!(vid.message, "matched USB VID 04b8");
        assert_eq!(
            vid.manufacturer.as_ref().map(|entry| entry.id.as_str()),
            Some("epson")
        );

        let name = resolve_manufacturer("Epson Perfection V600");
        assert_eq!(name.status, "ok");
        assert_eq!(name.matched_by.as_deref(), Some("name_token"));
        assert_eq!(name.message, "matched name token for Epson");
        assert_eq!(
            name.manufacturer.as_ref().map(|entry| entry.id.as_str()),
            Some("epson")
        );

        let short_token = resolve_manufacturer("HP LaserJet");
        assert_eq!(short_token.status, "ok");
        assert_eq!(short_token.matched_by.as_deref(), Some("name_token"));
        assert_eq!(short_token.message, "matched name token for HP");
        assert_eq!(
            short_token
                .manufacturer
                .as_ref()
                .map(|entry| entry.id.as_str()),
            Some("hp")
        );

        let unknown = resolve_manufacturer("XXHPXX");
        assert_eq!(unknown.status, "unknown");
        assert_eq!(unknown.query, "XXHPXX");
        assert_eq!(unknown.matched_by, None);
        assert_eq!(unknown.message, "no catalog manufacturer matched");
        assert!(unknown.manufacturer.is_none());
    }

    #[test]
    fn text_catalog_shape_is_stable() {
        assert_eq!(
            format_manufacturers_text(&list_manufacturers()),
            concat!(
                "# manufacturer-family hints only; verify capabilities on the selected device\n",
                "id\tname\tvids\tadf\tir\n",
                "epson\tEpson\t04b8\ttrue\ttrue\n",
                "canon\tCanon\t04a9\ttrue\tfalse\n",
                "hp\tHP\t03f0\ttrue\tfalse\n",
                "brother\tBrother\t04f9\ttrue\tfalse\n",
                "fujitsu\tFujitsu\t04c5\ttrue\tfalse\n",
                "plustek\tPlustek\t07b3\tfalse\ttrue\n",
                "nikon\tNikon\t04b0\tfalse\ttrue\n",
                "kodak\tKodak\t040a\ttrue\tfalse\n",
                "mustek\tMustek\t055f\tfalse\tfalse\n",
                "samsung\tSamsung\t04e8\ttrue\tfalse\n",
            ),
        );
    }
}
