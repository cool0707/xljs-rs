use super::*;
use crate::testutil::tempdir;

fn payload(name: &str, code: &str) -> Payload {
    Payload { name: name.into(), code: code.into(), meta: None, assets: None }
}

fn count(hay: &[u8], needle: &[u8]) -> usize {
    hay.windows(needle.len()).filter(|w| *w == needle).count()
}

#[test]
fn write_read() {
    let dir = tempdir();
    let base = dir.join("base.exe");
    std::fs::write(&base, "MZ fake executable").unwrap();
    assert!(read(&base).unwrap().is_none());
    let out = dir.join("out.exe");
    write_from(&base, &out, &payload("script.js", "console.log(1)")).unwrap();
    let p = read(&out).unwrap().unwrap();
    assert!(p.name == "script.js" && p.code == b"console.log(1)" && p.meta.is_none() && p.assets.is_none());
    // Re-packing a packed exe replaces the payload instead of stacking.
    let out2 = dir.join("out2.exe");
    write_from(&out, &out2, &payload("b.js", "2")).unwrap();
    let data = std::fs::read(&out2).unwrap();
    assert!(data.starts_with(b"MZ fake executable") && count(&data, b"XLJSPAK2") == 1);
    let p = read(&out2).unwrap().unwrap();
    assert!(p.name == "b.js" && p.code == b"2");
}

#[test]
fn assets() {
    let dir = tempdir();
    for f in ["app.js", "ui/index.html", "ui/img/logo.png", "secret.txt"] {
        let p = dir.join(f);
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(&p, format!("<{f}>")).unwrap();
    }
    let base = dir.join("base.exe");
    std::fs::write(&base, "MZ").unwrap();
    let m = Meta {
        mode: "app".into(),
        html: "ui/index.html".into(),
        assets: vec!["ui".into()],
        title: "売上".into(),
        width: 800,
        height: 600,
        ..Default::default()
    };
    let out = dir.join("out.exe");
    let p = Payload { name: "app.js".into(), code: b"x".to_vec(), meta: Some(m.clone()), assets: Some(Box::new(m.asset_fs(&dir))) };
    write_from(&base, &out, &p).unwrap();
    let p = read(&out).unwrap().unwrap();
    assert_eq!(p.meta.as_ref(), Some(&m));
    let a = p.assets.unwrap();
    for f in ["ui/index.html", "ui/img/logo.png"] {
        assert_eq!(a.read(f).unwrap(), format!("<{f}>").into_bytes());
    }
    for f in ["secret.txt", "app.js", "../manifest.json"] {
        assert!(a.read(f).is_err(), "{f} should not be embedded");
    }
    // A missing @html file is an error.
    let m2 = Meta { html: "missing.html".into(), ..Default::default() };
    let p = Payload { name: "app.js".into(), code: b"x".to_vec(), meta: Some(m2.clone()), assets: Some(Box::new(m2.asset_fs(&dir))) };
    assert!(write_from(&base, &out, &p).is_err());
}

#[test]
fn read_v1() {
    let dir = tempdir();
    let exe = dir.join("old.exe");
    let (name, code) = ("old.js", "console.log('v1')");
    let mut payload_bytes = (name.len() as u32).to_le_bytes().to_vec();
    payload_bytes.extend_from_slice(name.as_bytes());
    payload_bytes.extend_from_slice(code.as_bytes());
    let mut data = b"MZ old".to_vec();
    data.extend_from_slice(&payload_bytes);
    data.extend_from_slice(&(payload_bytes.len() as u64).to_le_bytes());
    data.extend_from_slice(b"XLJSPAK1");
    std::fs::write(&exe, data).unwrap();
    let p = read(&exe).unwrap().unwrap();
    assert!(p.name == name && p.code == code.as_bytes());
    // Packing over a v1 exe strips the old payload.
    let out = dir.join("new.exe");
    write_from(&exe, &out, &payload("n.js", "1")).unwrap();
    let got = std::fs::read(&out).unwrap();
    assert!(got.starts_with(b"MZ old") && count(&got, b"XLJSPAK1") == 0);
}
