Golden 802.11 frames, hand-assembled from IEEE 802.11-2016 field layouts
(hex, whitespace ignored, '#' starts a comment). MPDUs exclude the FCS unless
the file says otherwise. Addresses: AP/BSSID 02:00:00:00:00:01,
station 02:00:00:00:00:02, SSID "ECM". Used by FrameCodecTest (Java) and meant
to be parsed/produced byte for byte by the Rust ecm-wifi crate too.
