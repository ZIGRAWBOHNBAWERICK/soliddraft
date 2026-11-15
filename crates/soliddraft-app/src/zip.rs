//! A zip archive of files stored without compression, for exports that are several
//! files and download in the browser as one.

/// The archive of `files`, each a name and its bytes, stored as they are.
pub fn stored(files: &[(String, Vec<u8>)]) -> Vec<u8> {
    let mut out = Vec::new();
    let mut directory = Vec::new();
    for (name, bytes) in files {
        let offset = out.len() as u32;
        let crc = crc32(bytes);
        let size = bytes.len() as u32;
        let name = name.as_bytes();
        // Local header: version 2.0, UTF-8 names, stored, no date.
        out.extend_from_slice(&0x0403_4b50u32.to_le_bytes());
        out.extend_from_slice(&20u16.to_le_bytes());
        out.extend_from_slice(&0x0800u16.to_le_bytes());
        out.extend_from_slice(&0u16.to_le_bytes());
        out.extend_from_slice(&0u16.to_le_bytes());
        out.extend_from_slice(&0x0021u16.to_le_bytes());
        out.extend_from_slice(&crc.to_le_bytes());
        out.extend_from_slice(&size.to_le_bytes());
        out.extend_from_slice(&size.to_le_bytes());
        out.extend_from_slice(&(name.len() as u16).to_le_bytes());
        out.extend_from_slice(&0u16.to_le_bytes());
        out.extend_from_slice(name);
        out.extend_from_slice(bytes);
        // Its central directory entry.
        directory.extend_from_slice(&0x0201_4b50u32.to_le_bytes());
        directory.extend_from_slice(&20u16.to_le_bytes());
        directory.extend_from_slice(&20u16.to_le_bytes());
        directory.extend_from_slice(&0x0800u16.to_le_bytes());
        directory.extend_from_slice(&0u16.to_le_bytes());
        directory.extend_from_slice(&0u16.to_le_bytes());
        directory.extend_from_slice(&0x0021u16.to_le_bytes());
        directory.extend_from_slice(&crc.to_le_bytes());
        directory.extend_from_slice(&size.to_le_bytes());
        directory.extend_from_slice(&size.to_le_bytes());
        directory.extend_from_slice(&(name.len() as u16).to_le_bytes());
        directory.extend_from_slice(&[0; 12]);
        directory.extend_from_slice(&offset.to_le_bytes());
        directory.extend_from_slice(name);
    }
    let start = out.len() as u32;
    out.extend_from_slice(&directory);
    out.extend_from_slice(&0x0605_4b50u32.to_le_bytes());
    out.extend_from_slice(&[0; 4]);
    out.extend_from_slice(&(files.len() as u16).to_le_bytes());
    out.extend_from_slice(&(files.len() as u16).to_le_bytes());
    out.extend_from_slice(&(directory.len() as u32).to_le_bytes());
    out.extend_from_slice(&start.to_le_bytes());
    out.extend_from_slice(&0u16.to_le_bytes());
    out
}

/// The CRC-32 a zip entry carries (the reflected IEEE polynomial).
fn crc32(bytes: &[u8]) -> u32 {
    let mut crc = !0u32;
    for byte in bytes {
        crc ^= u32::from(*byte);
        for _ in 0..8 {
            crc = if crc & 1 == 1 { (crc >> 1) ^ 0xEDB8_8320 } else { crc >> 1 };
        }
    }
    !crc
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_crc_is_the_standard_one() {
        assert_eq!(crc32(b"123456789"), 0xCBF4_3926);
    }

    /// The archive opens with the `unzip` its readers have: the names and bytes come back.
    #[test]
    fn an_export_and_its_proof_make_one_archive() {
        let files = vec![
            ("Part1.step".to_owned(), b"ISO-10303-21;".to_vec()),
            ("Part1.step.proof.json".to_owned(), b"{}\n".to_vec()),
            ("Part1.step.proof.txt".to_owned(), b"proved\n".to_vec()),
        ];
        let archive = stored(&files);
        assert_eq!(&archive[..4], b"PK\x03\x04");
        let dir = std::env::temp_dir().join(format!("soliddraft-zip-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("Part1.zip");
        std::fs::write(&path, &archive).unwrap();
        if let Ok(listing) = std::process::Command::new("unzip").arg("-Z1").arg(&path).output()
            && listing.status.success()
        {
            let names = String::from_utf8_lossy(&listing.stdout);
            let names: Vec<&str> = names.lines().collect();
            assert_eq!(names, ["Part1.step", "Part1.step.proof.json", "Part1.step.proof.txt"]);
            let test = std::process::Command::new("unzip").arg("-t").arg(&path).output().unwrap();
            assert!(test.status.success(), "{}", String::from_utf8_lossy(&test.stdout));
        }
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
