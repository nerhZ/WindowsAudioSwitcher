use std::io::Cursor;
use std::path::PathBuf;

/// Verifies that the published update manifest signature validates against
/// the app's public key — the exact check tauri-plugin-updater performs on
/// every update check. If this fails, auto-update is broken for everyone.
#[test]
fn published_manifest_verifies_against_app_pubkey() {
    let fixture_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures");
    let manifest_path = fixture_dir.join("latest.json");
    let signature_path = fixture_dir.join("latest.json.sig");
    if !manifest_path.exists() {
        eprintln!("skipping: fixtures not present");
        return;
    }

    let manifest = std::fs::read(&manifest_path).unwrap();
    let signature_b64 = std::fs::read_to_string(&signature_path).unwrap();
    let signature_box = minisign::SignatureBox::from_string(signature_b64.trim()).unwrap();

    // Bare minisign public key (base64), same key that is embedded in
    // tauri.conf.json / baked into installed copies.
    let pubkey =
        minisign::PublicKey::from_base64(APP_PUBKEY_B64).unwrap();

    let result = minisign::verify(&pubkey, &signature_box, Cursor::new(&manifest), true, false, true);

    assert!(result.is_ok(), "manifest signature must verify: {result:?}");
}

const APP_PUBKEY_B64: &str =
    "RWQZD6IN2jZs5xGEucTohwkRgQd2fpyV+V0ccaqho4/Aj4BejwkkNRR+";
