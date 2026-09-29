//! RFC 1071 internet checksum, used by IPv4, ICMP, TCP, and UDP.

use super::types::Ipv4Addr;

fn sum_words(mut sum: u32, data: &[u8]) -> u32 {
    let (words, rest) = data.as_chunks::<2>();
    for c in words {
        sum += u16::from_be_bytes(*c) as u32;
        // Fold eagerly so arbitrarily long input can't overflow.
        if sum > 0xFFFF_0000 {
            sum = (sum & 0xffff) + (sum >> 16);
        }
    }
    if let [b] = rest {
        sum += (*b as u32) << 8;
    }
    sum
}

fn fold(mut sum: u32) -> u16 {
    while sum >> 16 != 0 {
        sum = (sum & 0xffff) + (sum >> 16);
    }
    !(sum as u16)
}

/// Compute the one's complement checksum over the given data.
/// Over data that already contains a correct checksum, the result is 0.
pub fn internet_checksum(data: &[u8]) -> u16 {
    fold(sum_words(0, data))
}

/// Compute the TCP/UDP checksum including the IPv4 pseudo-header.
/// Over a segment that already contains a correct checksum, the result is 0.
pub fn pseudo_header_checksum(src: &Ipv4Addr, dst: &Ipv4Addr, protocol: u8, segment: &[u8]) -> u16 {
    let mut sum: u32 = 0;
    sum = sum_words(sum, &src.0);
    sum = sum_words(sum, &dst.0);
    sum += protocol as u32;
    sum += (segment.len() as u32) & 0xffff;
    fold(sum_words(sum, segment))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rfc1071_example() {
        // Example from RFC 1071 section 3: sum = 0xddf2, checksum = !0xddf2
        let data = [0x00, 0x01, 0xf2, 0x03, 0xf4, 0xf5, 0xf6, 0xf7];
        assert_eq!(internet_checksum(&data), !0xddf2u16);
    }

    #[test]
    fn empty_odd_and_verify() {
        assert_eq!(internet_checksum(&[]), 0xffff);
        assert_eq!(internet_checksum(&[0xff]), !0xff00u16);
        let mut e = vec![0x45u8, 0, 0, 20, 0, 0, 0, 0, 64, 17, 0, 0, 10, 0, 0, 1, 10, 0, 0, 2];
        let c = internet_checksum(&e);
        e[10] = (c >> 8) as u8;
        e[11] = c as u8;
        assert_eq!(internet_checksum(&e), 0);
    }

    #[test]
    fn large_input_no_overflow() {
        let d = vec![0xffu8; 200_000];
        let _ = internet_checksum(&d);
        let _ = pseudo_header_checksum(&Ipv4Addr::BROADCAST, &Ipv4Addr::BROADCAST, 6, &d);
    }
}
