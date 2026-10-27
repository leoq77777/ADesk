use crate::types::RpcMessage;

const MAX_LINE: usize = 1024 * 1024;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CodecError {
    LineTooLong,
}

#[derive(Debug, Clone, PartialEq)]
pub enum DecodeItem {
    Message(RpcMessage),
    Invalid { raw: String, error: String },
}

#[derive(Debug, Default)]
pub struct NdjsonCodec {
    buf: Vec<u8>,
}

impl NdjsonCodec {
    pub fn new() -> Self {
        Self { buf: Vec::new() }
    }

    pub fn push(&mut self, data: &[u8]) -> Result<Vec<DecodeItem>, CodecError> {
        if self.buf.len().saturating_add(data.len()) > MAX_LINE * 2 {
            if !self.buf.contains(&b'\n') && !data.contains(&b'\n') {
                return Err(CodecError::LineTooLong);
            }
        }
        self.buf.extend_from_slice(data);
        self.drain_lines()
    }

    pub fn finish(&mut self) -> Result<Vec<DecodeItem>, CodecError> {
        if self.buf.is_empty() {
            return Ok(Vec::new());
        }
        if self.buf.len() > MAX_LINE {
            self.buf.clear();
            return Err(CodecError::LineTooLong);
        }
        let leftover = std::mem::take(&mut self.buf);
        Ok(vec![parse_line(&leftover)])
    }

    fn drain_lines(&mut self) -> Result<Vec<DecodeItem>, CodecError> {
        let mut out = Vec::new();
        while let Some(pos) = self.buf.iter().position(|&b| b == b'\n') {
            if pos > MAX_LINE {
                self.buf.drain(..=pos);
                return Err(CodecError::LineTooLong);
            }
            let mut line: Vec<u8> = self.buf.drain(..=pos).collect();
            line.pop();
            if line.ends_with(&[b'\r']) {
                line.pop();
            }
            if line.is_empty() {
                continue;
            }
            out.push(parse_line(&line));
        }
        if self.buf.len() > MAX_LINE {
            self.buf.clear();
            return Err(CodecError::LineTooLong);
        }
        Ok(out)
    }
}

fn parse_line(line: &[u8]) -> DecodeItem {
    match serde_json::from_slice::<RpcMessage>(line) {
        Ok(message) => DecodeItem::Message(message),
        Err(error) => DecodeItem::Invalid {
            raw: String::from_utf8_lossy(line).into_owned(),
            error: error.to_string(),
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::RpcMessage;
    use serde_json::json;

    fn req() -> RpcMessage {
        RpcMessage::request(1, "initialize", json!({"protocolVersion": 1}))
    }

    #[test]
    fn u03_whole_chunk_matches_bytewise() {
        let line = req().encode_line().unwrap();
        let mut whole = NdjsonCodec::new();
        let mut bytewise = NdjsonCodec::new();
        let a = whole.push(&line).unwrap();
        let mut b = Vec::new();
        for byte in &line {
            b.extend(bytewise.push(&[*byte]).unwrap());
        }
        assert_eq!(a, b);
        assert_eq!(a.len(), 1);
        match &a[0] {
            DecodeItem::Message(msg) => assert_eq!(msg.method.as_deref(), Some("initialize")),
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn u03_utf8_split_across_chunks() {
        let msg = RpcMessage::request(2, "session/prompt", json!({"text": "你好"}));
        let line = msg.encode_line().unwrap();
        let split = line.iter().position(|&b| b >= 0x80).expect("multibyte");
        let mut codec = NdjsonCodec::new();
        assert!(codec.push(&line[..split]).unwrap().is_empty());
        let items = codec.push(&line[split..]).unwrap();
        assert_eq!(items.len(), 1);
        let DecodeItem::Message(got) = &items[0] else {
            panic!("expected message");
        };
        assert_eq!(got.params.as_ref().unwrap()["text"], "你好");
    }

    #[test]
    fn u03_two_messages_in_one_chunk() {
        let mut raw = req().encode_line().unwrap();
        raw.extend(
            RpcMessage::notification("session/update", json!({"sessionUpdate": "agent_message_chunk"}))
                .encode_line()
                .unwrap(),
        );
        let items = NdjsonCodec::new().push(&raw).unwrap();
        assert_eq!(items.len(), 2);
    }

    #[test]
    fn u04_empty_line_ignored_malformed_diagnosed() {
        let mut codec = NdjsonCodec::new();
        let items = codec.push(b"\n{not-json}\n").unwrap();
        assert_eq!(items.len(), 1);
        assert!(matches!(items[0], DecodeItem::Invalid { .. }));
    }

    #[test]
    fn u04_eof_without_newline() {
        let mut codec = NdjsonCodec::new();
        let raw = serde_json::to_vec(&req()).unwrap();
        assert!(codec.push(&raw).unwrap().is_empty());
        let items = codec.finish().unwrap();
        assert!(matches!(items[0], DecodeItem::Message(_)));
    }

    #[test]
    fn u04_unknown_method_still_a_message() {
        let msg = RpcMessage::notification("future/unknown", json!({}));
        let items = NdjsonCodec::new().push(&msg.encode_line().unwrap()).unwrap();
        let DecodeItem::Message(got) = &items[0] else {
            panic!("expected message");
        };
        assert_eq!(got.method.as_deref(), Some("future/unknown"));
    }

    #[test]
    fn u04_overlong_line_errors() {
        let mut codec = NdjsonCodec::new();
        let err = codec.push(&vec![b'x'; MAX_LINE + 8]).unwrap_err();
        assert_eq!(err, CodecError::LineTooLong);
    }
}
