use serde_json::{json, Map, Value};
use std::sync::Mutex;

const IMAGE_MANIFEST: &str = include_str!("../../release/fpga-image-identity-v1.json");
static LAST_LOGGED_IDENTITY: Mutex<Option<String>> = Mutex::new(None);

fn read_build_id(fpga: &Map<String, Value>) -> Result<Option<u32>, &'static str> {
    let mut readings = Vec::new();
    for key in ["build_id_raw", "date_code_raw"] {
        if let Some(value) = fpga.get(key).filter(|value| !value.is_null()) {
            let value = value.as_u64().ok_or("invalid")?;
            readings.push(u32::try_from(value).map_err(|_| "invalid")?);
        }
    }
    if let Some(value) = fpga.get("date_code_hex").filter(|value| !value.is_null()) {
        let text = value.as_str().ok_or("invalid")?.trim_start_matches("0x");
        if text.len() != 8 || !text.bytes().all(|byte| byte.is_ascii_hexdigit()) {
            return Err("invalid");
        }
        readings.push(u32::from_str_radix(text, 16).map_err(|_| "invalid")?);
    }
    let Some(first) = readings.first().copied() else {
        return Ok(None);
    };
    if readings.iter().any(|value| *value != first) {
        return Err("conflicting");
    }
    Ok(Some(first))
}

/// Decorate a live app telemetry snapshot with the exact image identity.
/// Both backends expose the FPGA USR_ACCESS register; the deployed XDMA bridge
/// currently publishes it under the legacy date_code_hex telemetry name.
pub(crate) fn annotate(telemetry: &mut Value) {
    let fresh = telemetry.get("pid_matches_service") == Some(&Value::Bool(true))
        && telemetry
            .get("age_seconds")
            .and_then(Value::as_f64)
            .is_some_and(|age| (0.0..=5.0).contains(&age));
    let Some(fpga) = telemetry
        .get_mut("current")
        .and_then(|current| current.get_mut("fpga"))
        .and_then(Value::as_object_mut)
    else {
        return;
    };
    // An unavailable or malformed reading must never inherit an earlier label.
    for field in [
        "build_id_hex",
        "build_identity_status",
        "firmware_display",
        "firmware_subversion",
        "rx_filter",
    ] {
        fpga.remove(field);
    }
    if fpga.get("available") != Some(&Value::Bool(true)) {
        fpga.insert("build_identity_status".into(), json!("unavailable"));
        return;
    }
    let (Some(major), Some(minor)) = (
        fpga.get("firmware_major_version").and_then(Value::as_u64),
        fpga.get("firmware_version").and_then(Value::as_u64),
    ) else {
        fpga.insert("build_identity_status".into(), json!("unavailable"));
        return;
    };
    let build_id = match read_build_id(fpga) {
        Ok(Some(value)) => value,
        Ok(None) => {
            fpga.insert("build_identity_status".into(), json!("unavailable"));
            return;
        }
        Err(status) => {
            fpga.insert("build_identity_status".into(), json!(status));
            return;
        }
    };
    let build_id_hex = format!("0x{build_id:08X}");
    fpga.insert("build_id_raw".into(), json!(build_id));
    fpga.insert("build_id_hex".into(), json!(build_id_hex));
    if !fresh {
        fpga.insert("build_identity_status".into(), json!("stale"));
        return;
    }
    let manifest: Value = match serde_json::from_str::<Value>(IMAGE_MANIFEST) {
        Ok(value)
            if value.get("format").and_then(Value::as_str)
                == Some("saturn-fpga-image-identity-v1")
                && value.get("build_id_register").and_then(Value::as_str) == Some("0x4004")
                && value.get("images").and_then(Value::as_array).is_some() =>
        {
            value
        }
        _ => {
            fpga.insert("build_identity_status".into(), json!("manifest_invalid"));
            return;
        }
    };
    let match_entry = manifest
        .get("images")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .find(|entry| {
            entry.get("build_id").and_then(Value::as_str) == Some(build_id_hex.as_str())
                && entry.get("firmware_major").and_then(Value::as_u64) == Some(major)
                && entry.get("firmware_minor").and_then(Value::as_u64) == Some(minor)
        });
    let (status, version, rx_filter, subversion) = if let Some(entry) = match_entry {
        match (
            entry.get("firmware_subversion").and_then(Value::as_u64),
            entry.get("rx_filter").and_then(Value::as_str),
        ) {
            (Some(subversion), Some(filter)) => (
                "identified",
                format!("{major}.{minor:02}.{subversion:03}"),
                Some(filter.to_string()),
                Some(subversion),
            ),
            _ => (
                "manifest_invalid",
                format!("{major}.{minor:02} — build unidentified"),
                None,
                None,
            ),
        }
    } else {
        (
            "unidentified",
            format!("{major}.{minor:02} — build unidentified"),
            None,
            None,
        )
    };
    fpga.insert("build_identity_status".into(), json!(status));
    fpga.insert("firmware_display".into(), json!(version));
    fpga.insert("firmware_subversion".into(), json!(subversion));
    fpga.insert("rx_filter".into(), json!(rx_filter));
    let log_key = format!("{status}:{major}:{minor}:{build_id_hex}");
    if let Ok(mut last_logged) = LAST_LOGGED_IDENTITY.lock() {
        if last_logged.as_deref() != Some(&log_key) {
            eprintln!(
                "FPGA firmware: {version}; RX filter: {}; Build ID: {build_id_hex}; identity: {status}",
                rx_filter.as_deref().unwrap_or("unidentified")
            );
            *last_logged = Some(log_key);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn identifies_only_exact_manifest_match() {
        let mut known = json!({"pid_matches_service":true,"age_seconds":0.1,
            "current":{"fpga":{"available":true,"firmware_major_version":1,
            "firmware_version":30,"build_id_raw":0x53460002_u32}}});
        annotate(&mut known);
        let fpga = &known["current"]["fpga"];
        assert_eq!(fpga["firmware_display"], "1.30.002");
        assert_eq!(fpga["rx_filter"], "22/Q24");
        assert_eq!(fpga["build_id_hex"], "0x53460002");

        known["current"]["fpga"]["build_id_raw"] = json!(0x53460003_u32);
        annotate(&mut known);
        let fpga = &known["current"]["fpga"];
        assert_eq!(fpga["firmware_display"], "1.30 — build unidentified");
        assert!(fpga["rx_filter"].is_null());
        assert_eq!(fpga["build_id_hex"], "0x53460003");

        // The separately built 1.31 image is identified only with the exact
        // USR_ACCESS and firmware major/minor pair in the verified manifest.
        known["current"]["fpga"]["firmware_version"] = json!(31);
        annotate(&mut known);
        let fpga = &known["current"]["fpga"];
        assert_eq!(fpga["build_identity_status"], "identified");
        assert_eq!(fpga["firmware_display"], "1.31.001");
        assert_eq!(fpga["rx_filter"], "22/Q24 saturated");
        assert_eq!(fpga["build_id_hex"], "0x53460003");

        known["current"]["fpga"]["build_id_raw"] = json!(0x53460004_u32);
        annotate(&mut known);
        assert_eq!(
            known["current"]["fpga"]["build_identity_status"],
            "unidentified"
        );
        assert!(known["current"]["fpga"]["rx_filter"].is_null());

        known["current"]["fpga"]["build_id_raw"] = json!(0x53460002_u32);
        annotate(&mut known);
        assert_eq!(
            known["current"]["fpga"]["build_identity_status"],
            "unidentified"
        );

        known["current"]["fpga"]["firmware_version"] = json!(29);
        annotate(&mut known);
        assert_eq!(
            known["current"]["fpga"]["build_identity_status"],
            "unidentified"
        );

        known["current"]["fpga"]["build_id_raw"] = Value::Null;
        annotate(&mut known);
        let fpga = &known["current"]["fpga"];
        assert_eq!(fpga["build_identity_status"], "unavailable");
        assert!(fpga.get("firmware_display").is_none());
        assert!(fpga.get("rx_filter").is_none());

        known["current"]["fpga"]["build_id_raw"] = json!(0x53460002_u32);
        known["pid_matches_service"] = json!(false);
        annotate(&mut known);
        let fpga = &known["current"]["fpga"];
        assert_eq!(fpga["build_identity_status"], "stale");
        assert_eq!(fpga["build_id_hex"], "0x53460002");
        assert!(fpga.get("firmware_display").is_none());

        let mut baseline = json!({"pid_matches_service":true,"age_seconds":0.1,
            "current":{"fpga":{"available":true,"firmware_major_version":1,
            "firmware_version":30,"date_code_hex":"53460001"}}});
        annotate(&mut baseline);
        let fpga = &baseline["current"]["fpga"];
        assert_eq!(fpga["firmware_display"], "1.30.001");
        assert_eq!(fpga["rx_filter"], "18/Q20");
        assert_eq!(fpga["build_id_raw"], 0x53460001_u32);

        baseline["current"]["fpga"]["date_code_raw"] = json!(0x53460002_u32);
        annotate(&mut baseline);
        assert_eq!(
            baseline["current"]["fpga"]["build_identity_status"],
            "conflicting"
        );
        assert!(baseline["current"]["fpga"].get("rx_filter").is_none());
    }
}
