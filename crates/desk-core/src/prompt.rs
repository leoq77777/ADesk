use serde_json::{json, Value};

use crate::service::Provider;
use crate::store::Attachment;

pub fn encode_prompt(
    provider: &Provider,
    text: &str,
    attachments: &[Attachment],
) -> Result<Vec<Value>, String> {
    let mut parts = vec![json!({ "type": "text", "text": text })];
    for att in attachments {
        parts.push(encode_attachment(provider, att)?);
    }
    Ok(parts)
}

fn encode_attachment(provider: &Provider, att: &Attachment) -> Result<Value, String> {
    match provider {
        Provider::Fake { .. } | Provider::Acp { .. } => encode_acp_block(att),
    }
}

fn encode_acp_block(att: &Attachment) -> Result<Value, String> {
    let uri = format!("file://{}", att.stored_path);
    if att.mime.starts_with("image/") {
        let bytes = std::fs::read(&att.stored_path).map_err(|e| e.to_string())?;
        return Ok(json!({
            "type": "image",
            "data": base64::Engine::encode(&base64::engine::general_purpose::STANDARD, bytes),
            "mimeType": att.mime
        }));
    }
    if att.mime.starts_with("text/") || att.mime == "application/json" {
        let text = std::fs::read_to_string(&att.stored_path).map_err(|e| e.to_string())?;
        return Ok(json!({
            "type": "resource",
            "resource": {
                "uri": uri,
                "mimeType": att.mime,
                "text": text
            }
        }));
    }
    Ok(json!({
        "type": "resource_link",
        "name": att.original_name,
        "uri": uri,
        "mimeType": att.mime,
        "size": att.size
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn fake() -> Provider {
        Provider::Fake {
            bin: PathBuf::from("desk-fake-cli"),
            scenario: "reply".into(),
        }
    }

    fn att(name: &str, mime: &str, path: PathBuf) -> Attachment {
        let size = std::fs::metadata(&path).unwrap().len() as i64;
        Attachment {
            id: "a1".into(),
            original_name: name.into(),
            mime: mime.into(),
            size,
            stored_path: path.display().to_string(),
            message_id: None,
        }
    }

    fn tmp(name: &str, bytes: &[u8]) -> PathBuf {
        let path = std::env::temp_dir().join(format!(
            "desk-prompt-{}-{}-{name}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::write(&path, bytes).unwrap();
        path
    }

    #[test]
    fn text_file_is_nested_resource() {
        let path = tmp("note.txt", b"secret-xyz");
        let parts = encode_prompt(&fake(), "read", &[att("note.txt", "text/plain", path)]).unwrap();
        assert_eq!(parts[0]["type"], "text");
        assert_eq!(parts[1]["type"], "resource");
        assert_eq!(parts[1]["resource"]["text"], "secret-xyz");
        assert!(parts[1].get("uri").is_none());
    }

    #[test]
    fn image_is_base64_block() {
        let path = tmp("dot.png", &[0x89, 0x50, 0x4e, 0x47]);
        let parts = encode_prompt(&fake(), "see", &[att("dot.png", "image/png", path)]).unwrap();
        assert_eq!(parts[1]["type"], "image");
        assert_eq!(parts[1]["mimeType"], "image/png");
        assert!(parts[1]["data"].as_str().unwrap().starts_with("iVBOR"));
    }

    #[test]
    fn binary_is_resource_link() {
        let path = tmp("blob.bin", &[1, 2, 3]);
        let parts =
            encode_prompt(&fake(), "open", &[att("blob.bin", "application/octet-stream", path)])
                .unwrap();
        assert_eq!(parts[1]["type"], "resource_link");
        assert_eq!(parts[1]["name"], "blob.bin");
    }
}
