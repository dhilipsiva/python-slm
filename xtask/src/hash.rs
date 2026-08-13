use crate::error::{IoContext, Result, XtaskError};
use sha2::{Digest, Sha256};
use std::fs::File;
use std::io::Read;
use std::path::Path;

pub fn bytes(bytes: &[u8]) -> String {
    hex::encode(Sha256::digest(bytes))
}

pub fn file(path: &Path) -> Result<String> {
    let mut input = File::open(path).io_context(
        "FILE_OPEN_FAILED",
        format!("could not open {}", path.display()),
    )?;
    let mut hasher = Sha256::new();
    let mut buffer = [0_u8; 64 * 1024];
    loop {
        let read = input.read(&mut buffer).io_context(
            "FILE_READ_FAILED",
            format!("could not read {}", path.display()),
        )?;
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
    }
    Ok(hex::encode(hasher.finalize()))
}

pub fn require_file(path: &Path, expected: &str, code: &'static str) -> Result<()> {
    let actual = file(path)?;
    if actual != expected {
        return Err(XtaskError::integrity(
            code,
            format!(
                "SHA-256 mismatch for {}: expected {expected}, observed {actual}",
                path.display()
            ),
        ));
    }
    Ok(())
}

pub fn is_lower_sha256(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hashes_known_bytes() {
        assert_eq!(
            bytes(b"abc"),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
    }
}
