//! Known-answer tests: IEEE 802.11 Annex J (PBKDF2, PRF, CCMP), RFC 3394 (AES key wrap)
//! and an independently computed PTK / EAPOL-MIC vector. The vectors live in
//! `tests/vectors/*.json` so the Java 802.11 package can read the same files.

mod common;
use common::json::{self, hex};

use ecm_wifi::crypto;
use ecm_wifi::eapol::{self, KeyFrame};
use ecm_wifi::frame;

#[test]
fn pbkdf2_psk_mapping_annex_j4() {
    let v = json::load("pbkdf2.json");
    let vs = v.get("vectors").arr();
    assert_eq!(vs.len(), 3);
    for t in vs {
        let pmk = crypto::pmk_from_passphrase(t.get("passphrase").str().as_bytes(), t.get("ssid").str().as_bytes());
        assert_eq!(pmk.to_vec(), t.get("pmk").hex(), "{}", t.get("passphrase").str());
    }
    // The two headline vectors, pinned literally as well.
    assert_eq!(
        crypto::pmk_from_passphrase(b"password", b"IEEE").to_vec(),
        hex("f42c6fc52df0ebef9ebb4b90b38a5f902e83fe1b135a70e23aed762e9710a12e")
    );
    assert_eq!(
        crypto::pmk_from_passphrase(b"ThisIsAPassword", b"ThisIsASSID").to_vec(),
        hex("0dc0d6eb90555ed6419756b9a15ec3e3209b63df707dd508d14581f8982721af")
    );
}

#[test]
fn prf_annex_j3() {
    let v = json::load("prf.json");
    for t in v.get("vectors").arr() {
        let out = crypto::prf(
            &t.get("key").hex(),
            t.get("label").str().as_bytes(),
            t.get("data").str().as_bytes(),
            t.get("len").num() as usize,
        );
        assert_eq!(out, t.get("output").hex(), "label {}", t.get("label").str());
    }
}

#[test]
fn ccmp_annex_j64_encrypt_and_decrypt() {
    let v = json::load("ccmp.json");
    let t = &v.get("vectors").arr()[0];
    let tk: [u8; 16] = t.get("tk").hex().try_into().unwrap();
    let pn_b = t.get("pn").hex();
    let pn = pn_b.iter().fold(0u64, |a, &b| a << 8 | b as u64);
    assert_eq!(pn, 0xB503_9776_E70C);
    let hdr = t.get("header").hex();
    assert_eq!(crypto::ccmp_aad(&hdr), t.get("aad").hex());
    assert_eq!(crypto::ccmp_nonce(&hdr, pn).to_vec(), t.get("nonce").hex());
    assert_eq!(crypto::ccmp_header(pn, 0).to_vec(), t.get("ccmp_header").hex());

    let mut plain_mpdu = hdr.clone();
    plain_mpdu[1] &= !frame::FL_PROTECTED;
    plain_mpdu.extend_from_slice(&t.get("plaintext").hex());
    let mut enc = crypto::ccmp_encrypt(&tk, &plain_mpdu, pn, 0).unwrap();
    frame::append_fcs(&mut enc);
    assert_eq!(enc, t.get("encrypted_mpdu_with_fcs").hex());

    let wire = t.get("encrypted_mpdu_with_fcs").hex();
    let body = frame::strip_fcs(&wire).expect("FCS valid");
    let d = crypto::ccmp_decrypt(&tk, body).unwrap();
    assert_eq!(d.pn, pn);
    assert_eq!(d.key_id, 0);
    assert_eq!(&d.mpdu[24..], &t.get("plaintext").hex()[..]);
    assert_eq!(&body[body.len() - 8..], &t.get("mic").hex()[..]);

    // Any flipped bit in header (outside masked fields), body or MIC fails.
    for idx in [4usize, 30, 40, body.len() - 1] {
        let mut bad = body.to_vec();
        bad[idx] ^= 0x01;
        assert!(crypto::ccmp_decrypt(&tk, &bad).is_none(), "tamper at {idx}");
    }
    // The Retry bit is masked out of the AAD: a retransmission still decrypts.
    let mut retry = body.to_vec();
    retry[1] ^= frame::FL_RETRY;
    assert!(crypto::ccmp_decrypt(&tk, &retry).is_some());
    // Wrong key fails.
    let mut wrong = tk;
    wrong[0] ^= 1;
    assert!(crypto::ccmp_decrypt(&wrong, body).is_none());
}

#[test]
fn aes_key_wrap_rfc3394() {
    let v = json::load("aes_kw.json");
    for t in v.get("vectors").arr() {
        let kek = t.get("kek").hex();
        let key = t.get("key").hex();
        let wrapped = t.get("wrapped").hex();
        match kek.len() {
            16 => {
                let k: [u8; 16] = kek.try_into().unwrap();
                assert_eq!(crypto::aes_wrap(&k, &key).unwrap(), wrapped);
                assert_eq!(crypto::aes_unwrap(&k, &wrapped).unwrap(), key);
                let mut bad = wrapped.clone();
                bad[3] ^= 0x80;
                assert!(crypto::aes_unwrap(&k, &bad).is_none(), "integrity check");
            }
            32 => {
                let k: [u8; 32] = kek.try_into().unwrap();
                assert_eq!(crypto::aes_wrap_256(&k, &key).unwrap(), wrapped);
                assert_eq!(crypto::aes_unwrap_256(&k, &wrapped).unwrap(), key);
            }
            n => panic!("kek len {n}"),
        }
    }
}

#[test]
fn ptk_and_eapol_mic_cross_vector() {
    let v = json::load("ptk_mic.json");
    let t = &v.get("vectors").arr()[0];
    let pmk = crypto::pmk_from_passphrase(t.get("passphrase").str().as_bytes(), t.get("ssid").str().as_bytes());
    assert_eq!(pmk.to_vec(), t.get("pmk").hex());
    let aa: [u8; 6] = t.get("aa").hex().try_into().unwrap();
    let spa: [u8; 6] = t.get("spa").hex().try_into().unwrap();
    let an: [u8; 32] = t.get("anonce").hex().try_into().unwrap();
    let sn: [u8; 32] = t.get("snonce").hex().try_into().unwrap();
    let ptk = crypto::derive_ptk(&pmk, &aa, &spa, &an, &sn);
    assert_eq!(ptk.kck.to_vec(), t.get("kck").hex());
    assert_eq!(ptk.kek.to_vec(), t.get("kek").hex());
    assert_eq!(ptk.tk.to_vec(), t.get("tk").hex());
    // Swapping the roles must give the same PTK (min/max ordering).
    assert_eq!(crypto::derive_ptk(&pmk, &spa, &aa, &sn, &an), ptk);

    let unsigned = t.get("m2_unsigned").hex();
    assert_eq!(crypto::eapol_mic(&ptk.kck, &unsigned).to_vec(), t.get("m2_mic").hex());
    let signed = t.get("m2_signed").hex();
    assert!(eapol::verify_mic_raw(&ptk.kck, &signed));
    // Our EAPOL-Key codec builds the byte-identical frame.
    let mut kf = KeyFrame::parse(&signed).unwrap();
    assert_eq!(kf.to_bytes(), signed);
    assert!(kf.verify_mic(&ptk.kck));
    assert_eq!(kf.to_signed_bytes(&ptk.kck), signed);
    let mut bad = signed.clone();
    bad[20] ^= 1;
    assert!(!eapol::verify_mic_raw(&ptk.kck, &bad));
}
