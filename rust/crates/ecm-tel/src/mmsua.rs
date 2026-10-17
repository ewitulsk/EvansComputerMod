//! The phone's side of MMS (the HTTP part `telephonyd` does): building an
//! `M-Send.req` for one number or a group, reading what the MMSC answers,
//! the `M-NotifyResp.ind` after a fetch, and turning an `M-Retrieve.conf` into
//! what Messages shows (who it's from, the group, the text and the files).

use crate::mms::{self, Body, MessageType, Part, Pdu, ResponseStatus, Sender, Status};

/// What the phone needs to send one MMS.
#[derive(Clone, Debug, PartialEq)]
pub struct Outgoing {
    pub to: Vec<String>,
    pub subject: Option<String>,
    pub text: Option<String>,
    /// (content type, file name, bytes): image/png, audio/wav, text/x-vcard.
    pub files: Vec<(String, String, Vec<u8>)>,
}

/// `M-Send.req` with transaction id `tid` (the MMSC inserts our number).
pub fn send_req(tid: &str, o: &Outgoing) -> Pdu {
    let parts: Vec<Part> = o.files.iter().map(|(ct, name, data)| Part::attachment(ct, name, data.clone())).collect();
    let body = Body::message(o.text.as_deref(), parts, true);
    let mut p = Pdu::send_req(tid, Sender::InsertAddress, o.to.iter().map(|n| mms::plmn_address(n)).collect(), o.subject.as_deref(), body);
    p.delivery_report = Some(true);
    p.read_report = Some(false);
    p
}

/// The MMSC's `M-Send.conf`: the message id, or why not.
pub fn send_conf(bytes: &[u8]) -> Result<String, String> {
    let p = Pdu::decode(bytes).map_err(|e| format!("bad answer from the MMSC: {}", e))?;
    if p.message_type != MessageType::SendConf {
        return Err("unexpected answer from the MMSC".into());
    }
    match p.response_status {
        Some(ResponseStatus::Ok) => Ok(p.message_id.unwrap_or_default()),
        Some(ResponseStatus::ErrorContentNotAccepted) => Err("too big for the network".into()),
        Some(ResponseStatus::ErrorSendingAddressUnresolved) => Err("no such number".into()),
        Some(ResponseStatus::ErrorServiceDenied) => Err("MMS isn't allowed from this phone".into()),
        Some(s) => Err(format!("refused by the MMSC ({:?})", s)),
        None => Err("no status from the MMSC".into()),
    }
}

/// After fetching: tell the MMSC (transaction id from the notification).
pub fn notify_resp(tid: &str, retrieved: bool) -> Pdu {
    Pdu::notifyresp_ind(tid, if retrieved { Status::Retrieved } else { Status::Deferred })
}

/// A received MMS as Messages shows it.
#[derive(Clone, Debug, PartialEq)]
pub struct Received {
    pub message_id: String,
    pub from: String,
    /// Everyone it went to (a group when more than one besides us).
    pub to: Vec<String>,
    pub subject: String,
    pub text: Option<String>,
    pub files: Vec<(String, String, Vec<u8>)>,
    pub date_s: u64,
}

pub fn retrieved(bytes: &[u8]) -> Result<Received, String> {
    let p = Pdu::decode(bytes).map_err(|e| format!("bad MMS: {}", e))?;
    if p.message_type != MessageType::RetrieveConf {
        return Err("not an MMS".into());
    }
    let from = p.from_address().map(|a| mms::parse_address(a).0.to_string()).unwrap_or_default();
    let to = p.to.iter().map(|a| mms::parse_address(a).0.to_string()).collect();
    let body = p.body.clone().unwrap_or_else(|| Body::mixed(Vec::new()));
    let files = body
        .attachments()
        .iter()
        .map(|a| (a.content_type.clone(), a.name.clone().or_else(|| a.content_location.clone()).unwrap_or_else(|| "file".into()), a.data.clone()))
        .collect();
    Ok(Received { message_id: p.message_id.clone().unwrap_or_default(), from, to, subject: p.subject.clone().unwrap_or_default(), text: body.text(), files, date_s: p.date.unwrap_or(0) })
}

/// The thread a message belongs to: the other people in it, sorted (one number, or a group).
pub fn thread_key(me: &str, from: &str, to: &[String]) -> Vec<String> {
    let mut v: Vec<String> = std::iter::once(from.to_string()).chain(to.iter().cloned()).filter(|n| n != me && !n.is_empty()).collect();
    v.sort();
    v.dedup();
    v
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn send_and_read_back() {
        let o = Outgoing { to: vec!["+12145550143".into(), "+12145550144".into()], subject: None, text: Some("look".into()), files: vec![("image/png".into(), "cat.png".into(), vec![0x89, b'P', b'N', b'G'])] };
        let p = send_req("t1", &o);
        let b = p.encode();
        let back = Pdu::decode(&b).unwrap();
        assert_eq!(back.to.len(), 2);
        assert_eq!(back.body.as_ref().unwrap().text().as_deref(), Some("look"));
        let conf = Pdu::send_conf("t1", ResponseStatus::Ok, Some("m1")).encode();
        assert_eq!(send_conf(&conf).unwrap(), "m1");
        assert!(send_conf(&Pdu::send_conf("t1", ResponseStatus::ErrorContentNotAccepted, None).encode()).unwrap_err().contains("too big"));
        let rc = Pdu::retrieve_conf(None, Some("m1"), 5, Some(&mms::plmn_address("+12145550142")), o.to.iter().map(|t| mms::plmn_address(t)).collect(), None, back.body.unwrap()).encode();
        let r = retrieved(&rc).unwrap();
        assert_eq!(r.from, "+12145550142");
        assert_eq!(r.files.len(), 1);
        assert_eq!(r.files[0].1, "cat.png");
        assert_eq!(thread_key("+12145550143", &r.from, &r.to), vec!["+12145550142".to_string(), "+12145550144".to_string()]);
    }
}
