//! ICMP echo packet construction helpers shared by every `NetTools` backend.

/// Build an ICMP echo request packet (type 8). Lays out an 8-byte header
/// followed by a 32-byte payload, matching what `ping` has historically sent.
/// The checksum is filled in before returning.
pub fn build_echo_packet(id: u16, seq: u16) -> [u8; 40] {
    let mut pkt = [0u8; 40];
    pkt[0] = 8; // type = echo request
    pkt[1] = 0; // code
    // [2..4] checksum filled below
    pkt[4] = (id >> 8) as u8;
    pkt[5] = id as u8;
    pkt[6] = (seq >> 8) as u8;
    pkt[7] = seq as u8;
    for i in 8..40 {
        pkt[i] = (i - 8) as u8;
    }
    let cksum = checksum(&pkt);
    pkt[2] = (cksum >> 8) as u8;
    pkt[3] = cksum as u8;
    pkt
}

/// Standard 16-bit one's complement Internet checksum.
pub fn checksum(data: &[u8]) -> u16 {
    let mut sum: u32 = 0;
    let mut i = 0;
    while i + 1 < data.len() {
        sum += ((data[i] as u32) << 8) | (data[i + 1] as u32);
        i += 2;
    }
    if i < data.len() {
        sum += (data[i] as u32) << 8;
    }
    while sum > 0xFFFF {
        sum = (sum & 0xFFFF) + (sum >> 16);
    }
    !sum as u16
}
