use std::fs;
use std::path::Path;

use smartcomp::archive::{self, CreateOptions};
use smartcomp::codecs::Level;

fn sample_tree(dir: &Path) {
    fs::create_dir_all(dir.join("sub")).unwrap();
    fs::write(dir.join("a.txt"), b"hello hello hello").unwrap();
    fs::write(dir.join("empty.bin"), b"").unwrap();
    fs::write(dir.join("sub/big.bin"), vec![7u8; 3 * (1 << 20) + 5]).unwrap();
}

fn create(arc: &Path, input: &Path, password: Option<&str>) {
    let opts = CreateOptions { level: Level::Store, password, verbose: false, force_codec: None, progress: false };
    archive::create(arc, &[input.to_path_buf()], &opts).unwrap();
}

#[test]
fn roundtrip_plain_and_encrypted() {
    for password in [None, Some("secret")] {
        let tmp = tempfile::tempdir().unwrap();
        let src = tmp.path().join("src");
        sample_tree(&src);
        let arc = tmp.path().join("t.sca");
        create(&arc, &src, password);

        let out = tmp.path().join("out");
        let n = archive::extract(&arc, &out, password, false, false).unwrap();
        assert_eq!(n, 3);
        for f in ["a.txt", "empty.bin", "sub/big.bin"] {
            assert_eq!(fs::read(src.join(f)).unwrap(), fs::read(out.join("src").join(f)).unwrap(), "{f}");
        }
    }
}

#[test]
fn wrong_or_missing_password_fails() {
    let tmp = tempfile::tempdir().unwrap();
    let src = tmp.path().join("src");
    sample_tree(&src);
    let arc = tmp.path().join("t.sca");
    create(&arc, &src, Some("secret"));
    let out = tmp.path().join("out");
    assert!(archive::extract(&arc, &out, Some("wrong"), false, false).is_err());
    assert!(archive::extract(&arc, &out, None, false, false).is_err());
}

#[test]
fn tampering_is_detected() {
    for password in [None, Some("secret")] {
        let tmp = tempfile::tempdir().unwrap();
        let src = tmp.path().join("src");
        sample_tree(&src);
        let arc = tmp.path().join("t.sca");
        create(&arc, &src, password);

        let mut bytes = fs::read(&arc).unwrap();
        bytes[100] ^= 0xFF; // inside the first payload
        fs::write(&arc, &bytes).unwrap();
        assert!(archive::extract(&arc, &tmp.path().join("out"), password, false, false).is_err());
    }
}
