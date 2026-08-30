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
