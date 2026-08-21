use crate::cli::args::InfoModule;
use crate::device::{list_all_devices, list_backends};
use crate::features::feature_matrix;
use crate::manufacturers::manufacturer_support_summary;
use crate::ml::ml_module_info;
use crate::ocr::ocr_module_info;
use crate::platform::platform_summary;
use crate::twain::twain_shim_info;
use crate::{APP_NAME, VERSION};
use serde_json::json;

pub(super) fn run(module: InfoModule) -> i32 {
    let mut payload = json!({
        "app": APP_NAME,
        "version": VERSION,
    });
    let obj = payload.as_object_mut().unwrap();
    match module {
        InfoModule::All | InfoModule::Platform => {
            obj.insert("platform".into(), platform_summary());
        }
        _ => {}
    }
    if matches!(module, InfoModule::All | InfoModule::Ocr) {
        obj.insert("ocr".into(), ocr_module_info());
    }
    if matches!(module, InfoModule::All | InfoModule::Ml) {
        obj.insert("ml".into(), ml_module_info());
    }
    if matches!(module, InfoModule::All | InfoModule::Twain) {
        obj.insert("twain".into(), twain_shim_info());
    }
    if matches!(module, InfoModule::All | InfoModule::Backends) {
        insert_backends(obj);
    }
    if matches!(module, InfoModule::All | InfoModule::Features) {
        obj.insert("features".into(), feature_matrix());
    }
    if matches!(module, InfoModule::All | InfoModule::Manufacturers) {
        obj.insert("manufacturers".into(), manufacturer_support_summary());
    }
    replace_single_module_payload(obj, module);
    println!(
        "{}",
        serde_json::to_string_pretty(&payload).unwrap_or_else(|_| "{}".into())
    );
    0
}

fn insert_backends(obj: &mut serde_json::Map<String, serde_json::Value>) {
    let backends: Vec<_> = list_backends()
        .into_iter()
        .map(|b| {
            json!({
                "id": b.id,
                "name": b.name,
                "available": b.available,
            })
        })
        .collect();
    let devices: Vec<_> = list_all_devices()
        .into_iter()
        .map(|d| {
            json!({
                "id": d.id,
                "name": d.name,
                "kind": d.kind,
                "manufacturer_id": d.manufacturer_id,
                "manufacturer_name": d.manufacturer_name,
            })
        })
        .collect();
    obj.insert("backends".into(), json!(backends));
    obj.insert("devices".into(), json!(devices));
}

fn replace_single_module_payload(
    obj: &mut serde_json::Map<String, serde_json::Value>,
    module: InfoModule,
) {
    match module {
        InfoModule::Platform => replace_payload(obj, "platform", platform_summary()),
        InfoModule::Ocr => replace_payload(obj, "ocr", ocr_module_info()),
        InfoModule::Ml => replace_payload(obj, "ml", ml_module_info()),
        InfoModule::Twain => replace_payload(obj, "twain", twain_shim_info()),
        InfoModule::Features => replace_payload(obj, "features", feature_matrix()),
        InfoModule::Manufacturers => {
            replace_payload(obj, "manufacturers", manufacturer_support_summary())
        }
        InfoModule::Backends | InfoModule::All => {}
    }
}

fn replace_payload(
    obj: &mut serde_json::Map<String, serde_json::Value>,
    key: &str,
    value: serde_json::Value,
) {
    *obj = serde_json::Map::from_iter([
        ("app".into(), json!(APP_NAME)),
        ("version".into(), json!(VERSION)),
        (key.into(), value),
    ]);
}
