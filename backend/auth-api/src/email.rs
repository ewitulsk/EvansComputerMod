//! Outgoing mail: `log` (stdout, for local use), `resend` and `postmark`.

use async_trait::async_trait;
use std::sync::{Arc, Mutex};

#[derive(Debug, Clone, PartialEq)]
pub struct Mail {
    pub to: String,
    pub subject: String,
    pub text: String,
}

#[async_trait]
pub trait Mailer: Send + Sync {
    async fn send(&self, from: &str, mail: &Mail) -> Result<(), String>;
}

/// Mails sent through the `log` provider, newest last (tests read the links from here).
pub type Outbox = Arc<Mutex<Vec<Mail>>>;

/// Prints each mail to stdout and keeps the last hundred.
pub struct LogMailer {
    pub outbox: Outbox,
}

#[async_trait]
impl Mailer for LogMailer {
    async fn send(&self, from: &str, mail: &Mail) -> Result<(), String> {
        println!("--- mail (EMAIL_PROVIDER=log) ---\nFrom: {from}\nTo: {}\nSubject: {}\n\n{}\n---", mail.to, mail.subject, mail.text);
        let mut o = self.outbox.lock().unwrap();
        if o.len() >= 100 {
            o.remove(0);
        }
        o.push(mail.clone());
        Ok(())
    }
}

fn client() -> reqwest::Client {
    reqwest::Client::builder().timeout(std::time::Duration::from_secs(15)).build().expect("http client")
}

async fn delivered(provider: &str, r: Result<reqwest::Response, reqwest::Error>) -> Result<(), String> {
    match r {
        Ok(resp) if resp.status().is_success() => Ok(()),
        Ok(resp) => {
            let status = resp.status();
            let body: String = resp.text().await.unwrap_or_default().chars().take(300).collect();
            Err(format!("{provider} answered {status}: {body}"))
        }
        Err(e) => Err(format!("{provider}: {}", e.without_url())),
    }
}

/// https://resend.com/docs/api-reference/emails/send-email
pub struct Resend {
    pub api_key: String,
    http: reqwest::Client,
}

#[async_trait]
impl Mailer for Resend {
    async fn send(&self, from: &str, mail: &Mail) -> Result<(), String> {
        let body = serde_json::json!({ "from": from, "to": [mail.to], "subject": mail.subject, "text": mail.text });
        delivered("resend", self.http.post("https://api.resend.com/emails").bearer_auth(&self.api_key).json(&body).send().await).await
    }
}

/// https://postmarkapp.com/developer/api/email-api
pub struct Postmark {
    pub api_key: String,
    http: reqwest::Client,
}

#[async_trait]
impl Mailer for Postmark {
    async fn send(&self, from: &str, mail: &Mail) -> Result<(), String> {
        let body = serde_json::json!({
            "From": from, "To": mail.to, "Subject": mail.subject, "TextBody": mail.text, "MessageStream": "outbound",
        });
        let req = self
            .http
            .post("https://api.postmarkapp.com/email")
            .header("Accept", "application/json")
            .header("X-Postmark-Server-Token", &self.api_key)
            .json(&body);
        delivered("postmark", req.send().await).await
    }
}

/// The mailer for `EMAIL_PROVIDER`, and the outbox the `log` provider fills.
pub fn mailer(provider: &str, api_key: Option<&str>) -> Result<(Arc<dyn Mailer>, Outbox), String> {
    let outbox: Outbox = Arc::default();
    let key = || api_key.map(str::to_string).ok_or_else(|| format!("EMAIL_PROVIDER={provider} needs EMAIL_API_KEY"));
    let m: Arc<dyn Mailer> = match provider {
        "log" => Arc::new(LogMailer { outbox: outbox.clone() }),
        "resend" => Arc::new(Resend { api_key: key()?, http: client() }),
        "postmark" => Arc::new(Postmark { api_key: key()?, http: client() }),
        other => return Err(format!("EMAIL_PROVIDER must be log, resend or postmark, not '{other}'")),
    };
    Ok((m, outbox))
}
