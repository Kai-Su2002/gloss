use serde_json::Value;

pub fn provider_error(code: Option<&str>) -> String {
    match code {
        Some("subscription_sharing_usage_limit_exceeded") => "ChatGPT plan sharing limit reached. Review usage in ChatGPT Settings and try again later.",
        Some("subscription_sharing_usage_unavailable") => "ChatGPT plan sharing is unavailable. Review this app's access in ChatGPT Settings.",
        _ => "ChatGPT request failed. Try again or reconnect your account.",
    }.into()
}

/// Buffer bytes, not strings: UTF-8 code points may straddle network chunks.
#[derive(Default)]
pub struct Stream {
    pending: Vec<u8>,
    lines: Vec<String>,
    pub completed: bool,
}
impl Stream {
    pub fn push(
        &mut self,
        bytes: &[u8],
        mut delta: impl FnMut(String) -> Result<(), String>,
    ) -> Result<(), String> {
        self.pending.extend_from_slice(bytes);
        if self.pending.len() > 4 * 1024 * 1024 {
            return Err("ChatGPT event too large".into());
        }
        while let Some(end) = self.pending.iter().position(|b| *b == b'\n') {
            let line: Vec<_> = self.pending.drain(..=end).collect();
            let line = std::str::from_utf8(&line[..line.len() - 1])
                .map_err(|_| "Invalid ChatGPT stream encoding")?
                .trim_end_matches('\r');
            if line.is_empty() {
                let data = self.lines.join("\n");
                self.lines.clear();
                if data.is_empty() {
                    continue;
                }
                if data == "[DONE]" {
                    if !self.completed {
                        return Err("ChatGPT stream ended without completion".into());
                    }
                    continue;
                }
                let value: Value =
                    serde_json::from_str(&data).map_err(|_| "Invalid ChatGPT event")?;
                match value["type"].as_str() {
                    Some("response.output_text.delta") => {
                        if self.completed {
                            return Err("Unexpected delta after completion".into());
                        }
                        delta(
                            value["delta"]
                                .as_str()
                                .ok_or("Invalid ChatGPT delta")?
                                .into(),
                        )?;
                    }
                    Some("response.completed") => self.completed = true,
                    Some("response.failed" | "response.incomplete" | "error") => {
                        return Err(provider_error(
                            value
                                .pointer("/response/error/code")
                                .or_else(|| value.get("code"))
                                .and_then(Value::as_str),
                        ))
                    }
                    _ => {}
                }
            } else if let Some(data) = line.strip_prefix("data:") {
                self.lines
                    .push(data.strip_prefix(' ').unwrap_or(data).into());
                if self.lines.iter().map(String::len).sum::<usize>() > 4 * 1024 * 1024 {
                    return Err("ChatGPT event too large".into());
                }
            }
        }
        Ok(())
    }
    pub fn finish(&self) -> Result<(), String> {
        if self.completed {
            Ok(())
        } else {
            Err("ChatGPT stream interrupted before completion".into())
        }
    }
}
