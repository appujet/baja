use serde_json::Value;

pub fn is_invalid_track(resp: &Value) -> bool {
    let methods = match resp["methods"].as_array() {
        Some(m) if !m.is_empty() => m,
        _ => return true,
    };

    let has_error_note = methods.iter().any(|m| {
        m["interface"]
            .as_str()
            .map(|i| i.contains("ShowNotificationMethod"))
            .unwrap_or(false)
            && m["notification"]["message"]["text"]
                .as_str()
                .map(|t| t.contains("no longer available"))
                .unwrap_or(false)
    });

    let is_homepage = methods[0]["template"]["interface"]
        .as_str()
        .map(|i| i.contains("GalleryTemplate"))
        .unwrap_or(false)
        && methods[0]["template"]["widgets"]
            .as_array()
            .map(|w| w.is_empty())
            .unwrap_or(false);

    has_error_note || is_homepage
}

pub fn is_invalid_album(resp: &Value) -> bool {
    let methods = match resp["methods"].as_array() {
        Some(m) if !m.is_empty() => m,
        _ => return true,
    };

    let template = &methods[0]["template"];
    template["interface"]
        .as_str()
        .map(|i| i.contains("DialogTemplate"))
        .unwrap_or(false)
        && template["header"]
            .as_str()
            .map(|h| h == "Service error")
            .unwrap_or(false)
}

pub fn is_invalid_artist(resp: &Value) -> bool {
    let methods = match resp["methods"].as_array() {
        Some(m) if !m.is_empty() => m,
        _ => return true,
    };

    let template = &methods[0]["template"];
    template["interface"]
        .as_str()
        .map(|i| i.contains("MessageTemplate"))
        .unwrap_or(false)
        && template["header"]
            .as_str()
            .map(|h| h == "We're Sorry")
            .unwrap_or(false)
        && template["message"]
            .as_str()
            .map(|m| m.contains("unable to complete your action"))
            .unwrap_or(false)
}

pub fn is_invalid_playlist(resp: &Value) -> bool {
    let methods = match resp["methods"].as_array() {
        Some(m) if !m.is_empty() => m,
        _ => return true,
    };

    let first = &methods[0];

    if first["template"]["widgets"]
        .as_array()
        .map(|w| w.is_empty())
        .unwrap_or(false)
    {
        return true;
    }

    if let Some(second) = methods.get(1) {
        let msg = second["notification"]["message"]["text"]
            .as_str()
            .or_else(|| second["notification"]["message"]["innerHTML"].as_str())
            .unwrap_or("");
        if msg.to_lowercase().contains("playlist is no longer available") {
            return true;
        }
    }

    first["template"]["templateData"]["deeplink"]
        .as_str()
        .map(|d| d == "/")
        .unwrap_or(false)
}

pub fn is_invalid_community_playlist(resp: &Value) -> bool {
    let template = match resp["methods"].as_array().and_then(|m| m.first()) {
        Some(m) => &m["template"],
        None => return false,
    };

    let is_dialog = template["interface"]
        .as_str()
        .map(|i| i == "Web.TemplatesInterface.v1_0.Touch.DialogTemplateInterface.DialogTemplate")
        .unwrap_or(false);

    let is_service_error = template["header"]
        .as_str()
        .map(|h| h.trim().to_lowercase() == "service error")
        .unwrap_or(false);

    let has_error_body = template["body"]["text"]
        .as_str()
        .map(|t| t.to_lowercase().contains("sorry something went wrong"))
        .unwrap_or(false);

    is_dialog && is_service_error && has_error_body
}
