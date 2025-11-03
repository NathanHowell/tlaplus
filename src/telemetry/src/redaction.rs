use std::io::{self, Write};

use serde_json::Value;
use tracing::Metadata;
use tracing_subscriber::fmt::MakeWriter;

const REDACTED_PLACEHOLDER: &str = "[REDACTED]";

/// Wraps a [`MakeWriter`] to redact sensitive TLC fields before flushing output.
#[derive(Clone)]
pub(crate) struct RedactingMakeWriter<M> {
    inner: M,
}

impl<M> RedactingMakeWriter<M> {
    pub(crate) fn new(inner: M) -> Self {
        Self { inner }
    }
}

impl<'a, M> MakeWriter<'a> for RedactingMakeWriter<M>
where
    M: MakeWriter<'a>,
    M::Writer: Write,
{
    type Writer = RedactingWriter<M::Writer>;

    fn make_writer(&'a self) -> Self::Writer {
        RedactingWriter::new(self.inner.make_writer())
    }

    fn make_writer_for(&'a self, meta: &Metadata<'_>) -> Self::Writer {
        RedactingWriter::new(self.inner.make_writer_for(meta))
    }
}

/// Writer that buffers JSON log lines and redacts sensitive keys before forwarding them.
pub(crate) struct RedactingWriter<W> {
    inner: W,
    buffer: Vec<u8>,
}

impl<W> RedactingWriter<W>
where
    W: Write,
{
    fn new(inner: W) -> Self {
        Self {
            inner,
            buffer: Vec::new(),
        }
    }

    fn flush_buffer(&mut self) -> io::Result<()> {
        if self.buffer.is_empty() {
            return Ok(());
        }

        let line: Vec<u8> = self.buffer.drain(..).collect();
        let sanitized = redact_json_line(&line)?;
        self.inner.write_all(&sanitized)?;
        Ok(())
    }
}

impl<W: Write> Write for RedactingWriter<W> {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.buffer.extend_from_slice(buf);
        while let Some(position) = self.buffer.iter().position(|byte| *byte == b'\n') {
            let mut line: Vec<u8> = self.buffer.drain(..=position).collect();
            let newline = line.pop().is_some();
            if line.last() == Some(&b'\r') {
                line.pop();
            }

            let mut sanitized = redact_json_line(&line)?;
            if newline {
                sanitized.push(b'\n');
            }
            self.inner.write_all(&sanitized)?;
        }

        Ok(buf.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        self.flush_buffer()?;
        self.inner.flush()
    }
}

fn redact_json_line(line: &[u8]) -> io::Result<Vec<u8>> {
    let mut value: Value = match serde_json::from_slice(line) {
        Ok(value) => value,
        Err(_) => {
            // If parsing fails, forward the original line unchanged.
            return Ok(line.to_vec());
        }
    };

    redact_value(&mut value);

    serde_json::to_vec(&value).map_err(|err| io::Error::new(io::ErrorKind::Other, err))
}

fn redact_value(value: &mut Value) {
    match value {
        Value::Object(map) => {
            let keys: Vec<String> = map.keys().cloned().collect();
            for key in keys {
                if let Some(entry) = map.get_mut(&key) {
                    if should_redact(&key) {
                        *entry = Value::String(REDACTED_PLACEHOLDER.to_string());
                    } else {
                        redact_value(entry);
                    }
                }
            }
        }
        Value::Array(items) => {
            for item in items {
                redact_value(item);
            }
        }
        _ => {}
    }
}

fn should_redact(field_name: &str) -> bool {
    let lower = field_name.to_ascii_lowercase();
    let tokens: Vec<&str> = lower
        .split(|c| matches!(c, '.' | '_' | '-'))
        .filter(|token| !token.is_empty())
        .collect();

    if tokens.is_empty() {
        return false;
    }

    if tokens.iter().any(|token| {
        matches!(
            *token,
            "hash" | "id" | "ids" | "fingerprint" | "count" | "total"
        )
    }) {
        return false;
    }

    let has_spec = tokens.iter().any(|token| {
        matches!(
            *token,
            "spec" | "specification" | "specname" | "specificationname"
        )
    });
    let has_module = tokens
        .iter()
        .any(|token| matches!(*token, "module" | "modules" | "mod"));
    let identifier_hint = tokens.iter().any(|token| {
        matches!(
            *token,
            "name" | "path" | "file" | "label" | "identifier" | "ident" | "primary" | "fullname"
        )
    });

    if (has_spec || has_module) && identifier_hint {
        return true;
    }

    if has_spec && has_module {
        return true;
    }

    if tokens.len() == 1 && (has_spec || has_module) {
        return true;
    }

    if has_module
        && tokens
            .iter()
            .any(|token| matches!(*token, "current" | "root" | "active" | "primary"))
    {
        return true;
    }

    false
}

#[cfg(test)]
mod tests {
    use super::{redact_json_line, should_redact, REDACTED_PLACEHOLDER};

    #[test]
    fn redact_known_sensitive_fields() {
        assert!(should_redact("spec_name"));
        assert!(should_redact("primary_module"));
        assert!(should_redact("spec.module"));
        assert!(should_redact("modules"));
        assert!(should_redact("module-path"));
    }

    #[test]
    fn avoid_redacting_hashes_and_counts() {
        assert!(!should_redact("spec_hash"));
        assert!(!should_redact("module_count"));
        assert!(!should_redact("checkpoint_id"));
    }

    #[test]
    fn redact_json_output_lines() {
        let line = br#"{"fields":{"spec_name":"Foo","message":"ok"}}"#;
        let sanitized = redact_json_line(line).expect("redaction should succeed");
        let json: serde_json::Value =
            serde_json::from_slice(&sanitized).expect("sanitized JSON should parse");
        assert_eq!(
            json["fields"]["spec_name"].as_str(),
            Some(REDACTED_PLACEHOLDER)
        );
    }
}
