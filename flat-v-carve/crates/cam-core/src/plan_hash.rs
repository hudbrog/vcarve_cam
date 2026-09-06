//! Hash the canonical artifact serialization without a second full JSON buffer.
use serde::Serialize;
use sha2::{Digest, Sha256};
use std::io::{self, BufWriter, Write};

pub(crate) fn hash(value: &impl Serialize) -> serde_json::Result<String> {
    struct Writer(Sha256);
    impl Write for Writer {
        fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
            self.0.update(bytes);
            Ok(bytes.len())
        }
        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }
    let mut writer = Writer(Sha256::new());
    // Serde writes many tiny tokens. Batch digest updates without retaining the
    // full artifact or changing a single canonical serialization byte.
    {
        let mut buffer = BufWriter::with_capacity(64 * 1024, &mut writer);
        serde_json::to_writer(&mut buffer, value)?;
        buffer.flush().map_err(serde_json::Error::io)?;
    }
    Ok(format!("{:x}", writer.0.finalize()))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn streaming_preserves_existing_fingerprints() {
        let value = serde_json::json!({"text":"quotes\" and \\ and λ".repeat(10_000), "coordinates":[0.0, -1.25, 1e-12]});
        assert_eq!(
            hash(&value).unwrap(),
            format!("{:x}", Sha256::digest(serde_json::to_vec(&value).unwrap()))
        );
    }
}
