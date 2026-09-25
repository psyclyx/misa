//! Host-owned local filesystem destination for downloaded attachments.

pub fn write_new(destination: &str, bytes: &[u8]) -> Result<(), String> {
    use std::io::Write as _;
    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(destination)
        .map_err(|error| format!("Could not create {destination}: {error}"))?;
    file.write_all(bytes)
        .and_then(|_| file.sync_all())
        .map_err(|error| format!("Could not finish {destination}: {error}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn saving_creates_an_exact_file_and_refuses_to_overwrite() {
        let destination = std::env::temp_dir().join(format!(
            "misa-save-{}-{}.bin",
            std::process::id(),
            crate::test_unique_id()
        ));
        let path = destination.to_str().unwrap();
        write_new(path, b"received bytes").unwrap();
        assert_eq!(std::fs::read(&destination).unwrap(), b"received bytes");
        assert!(write_new(path, b"replacement").is_err());
        assert_eq!(std::fs::read(&destination).unwrap(), b"received bytes");
        std::fs::remove_file(destination).unwrap();
    }
}
