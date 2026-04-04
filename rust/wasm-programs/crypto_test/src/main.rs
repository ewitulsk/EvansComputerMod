extern crate ecm_ssh_crypto;
extern crate getrandom;

fn main() {
    println!("Running crypto tests...");

    // Test 1: Ed25519 sign/verify
    let (pub_key, priv_key) = ecm_ssh_crypto::ed25519_generate_keypair();
    let message = b"hello from minecraft";
    let sig = ecm_ssh_crypto::ed25519_sign(&priv_key, message);
    let valid = ecm_ssh_crypto::ed25519_verify(&pub_key, message, &sig);
    if !valid {
        println!("  FAIL: Ed25519 sign/verify");
        return;
    }
    println!("  PASS: Ed25519 sign/verify");

    // Test 2: Ed25519 wrong message fails
    let invalid = ecm_ssh_crypto::ed25519_verify(&pub_key, b"wrong message", &sig);
    if invalid {
        println!("  FAIL: Ed25519 wrong message should fail");
        return;
    }
    println!("  PASS: Ed25519 wrong message rejected");

    // Test 3: Key serialization round-trip
    let pub_bytes = ecm_ssh_crypto::ed25519_public_key_bytes(&pub_key);
    let pub_key2 = ecm_ssh_crypto::ed25519_public_key_from_bytes(&pub_bytes).unwrap();
    let valid2 = ecm_ssh_crypto::ed25519_verify(&pub_key2, message, &sig);
    if !valid2 {
        println!("  FAIL: Key serialization round-trip");
        return;
    }
    println!("  PASS: Key serialization round-trip");

    // Test 4: X25519 DH
    let (pub_a, sec_a) = ecm_ssh_crypto::x25519_generate_keypair();
    let (pub_b, sec_b) = ecm_ssh_crypto::x25519_generate_keypair();
    let shared_a = ecm_ssh_crypto::x25519_diffie_hellman(&sec_a, &pub_b);
    let shared_b = ecm_ssh_crypto::x25519_diffie_hellman(&sec_b, &pub_a);
    if shared_a != shared_b {
        println!("  FAIL: X25519 DH shared secret mismatch");
        return;
    }
    println!("  PASS: X25519 DH key agreement");

    // Test 5: SHA-256
    let hash = ecm_ssh_crypto::sha256(b"test");
    if hash.len() != 32 {
        println!("  FAIL: SHA-256 output length");
        return;
    }
    println!("  PASS: SHA-256");

    // Test 6: HMAC-SHA-256
    let mac = ecm_ssh_crypto::hmac_sha256(b"key", b"data");
    if mac.len() != 32 {
        println!("  FAIL: HMAC-SHA-256 output length");
        return;
    }
    println!("  PASS: HMAC-SHA-256");

    // Test 7: ChaCha20-Poly1305 encrypt/decrypt
    let key = ecm_ssh_crypto::sha256(b"encryption key");
    let nonce = [0u8; 12];
    let plaintext = b"secret message";
    match ecm_ssh_crypto::chacha20_poly1305_encrypt(&key, &nonce, plaintext) {
        Ok(ciphertext) => {
            match ecm_ssh_crypto::chacha20_poly1305_decrypt(&key, &nonce, &ciphertext) {
                Ok(decrypted) => {
                    if decrypted != plaintext {
                        println!("  FAIL: ChaCha20-Poly1305 decrypt mismatch");
                        return;
                    }
                    println!("  PASS: ChaCha20-Poly1305 encrypt/decrypt");
                }
                Err(e) => { println!("  FAIL: ChaCha20-Poly1305 decrypt: {}", e); return; }
            }
        }
        Err(e) => { println!("  FAIL: ChaCha20-Poly1305 encrypt: {}", e); return; }
    }

    println!("Crypto test: PASS");
}
