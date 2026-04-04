extern crate ecm_host_abi;
extern crate ecm_ssh_crypto;
extern crate getrandom;

fn main() {
    println!("Generating new SSH host key...");

    let (pub_key, priv_key) = ecm_ssh_crypto::ed25519_generate_keypair();
    let pub_bytes = ecm_ssh_crypto::ed25519_public_key_bytes(&pub_key);
    let priv_bytes = ecm_ssh_crypto::ed25519_private_key_bytes(&priv_key);

    // Save private key
    ecm_host_abi::fs::mkdir("/etc/ssh");
    ecm_host_abi::fs::write_file_bytes("/etc/ssh/ssh_host_ed25519_key", &priv_bytes);

    // Save public key
    ecm_host_abi::fs::write_file_bytes("/etc/ssh/ssh_host_ed25519_key.pub", &pub_bytes);

    // Print fingerprint
    let fingerprint = ecm_ssh_crypto::sha256(&pub_bytes);
    let hex: String = fingerprint[..16].iter().map(|b| format!("{:02x}", b)).collect();
    println!("Host key fingerprint: SHA256:{}", hex);
    println!("Key saved to /etc/ssh/ssh_host_ed25519_key");
}
