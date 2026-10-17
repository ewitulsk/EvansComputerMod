//! Telephony for the mod's phones and carriers (docs/TELEPHONY.md).
//!
//! | Module | What |
//! |---|---|
//! | [`number`] | E.164 / NANP numbers, dial strings, short codes, ENUM names |
//! | [`sip`] | SIP messages (RFC 3261, trimmed): parse, build, headers |
//! | [`sdp`] | SDP offer/answer for ADPCM-WB / ADPCM-NB and telephone-event |
//! | [`sms`] | 3GPP TS 23.040 TPDUs (SUBMIT, DELIVER, STATUS-REPORT), GSM 7-bit, UCS-2, concatenation; TS 24.011 RP messages |
//! | [`mms`] | OMA MMS encapsulation PDUs, multipart bodies, WAP push |
//! | [`vcard`] | vCard 3.0 contacts |
//! | [`txn`], [`dialog`] | SIP transactions over UDP, dialogs and request builders |
//! | [`ua`] | `telephonyd`: the phone's IMS client (registration, calls with media, SMS) |
//! | [`smsc`] | `smscd`: SMSC / IP-SM-GW, store and forward, reports, short codes, WAP push |
//! | [`ims`] | `imsd`: P/S/I-CSCF, registrar with IMS AKA, TAS, border and media relay |

pub mod dialog;
pub mod ims;
pub mod mms;
pub mod number;
pub mod sdp;
pub mod sip;
pub mod smsc;
pub mod sms;
pub mod txn;
pub mod ua;
pub mod util;
pub mod vcard;
