//! The address book: Contacts' vCard file (`/home/.contacts/contacts.vcf`, vCard 3.0)
//! shared by Phone, Messages and Contacts (and the `phone` module's `contacts()`).

use crate::Files;
use ecm_tel::vcard::{self, Contact};

pub const PATH: &str = "/home/.contacts/contacts.vcf";

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Book {
    pub contacts: Vec<Contact>,
}

fn digits(s: &str) -> String {
    s.chars().filter(|c| c.is_ascii_digit()).collect()
}

/// Do two ways of writing a number name the same phone (`(214) 555-0142` and `+12145550142`)?
pub fn same_number(a: &str, b: &str) -> bool {
    let (x, y) = (digits(a), digits(b));
    !x.is_empty() && !y.is_empty() && (x == y || (x.len() >= 7 && y.ends_with(&x)) || (y.len() >= 7 && x.ends_with(&y)))
}

impl Book {
    pub fn load(files: &mut dyn Files) -> Book {
        let text = files.read(PATH).map(|b| String::from_utf8_lossy(&b).into_owned()).unwrap_or_default();
        let mut contacts = vcard::parse(&text).unwrap_or_default();
        for (i, c) in contacts.iter_mut().enumerate() {
            if c.uid.is_empty() {
                c.uid = format!("c{}", i + 1);
            }
        }
        contacts.sort_by_key(|c| c.name.to_lowercase());
        Book { contacts }
    }

    pub fn save(&self, files: &mut dyn Files) -> Result<(), String> {
        files.write(PATH, vcard::write_all(&self.contacts).as_bytes())
    }

    /// The contact with this number.
    pub fn find(&self, number: &str) -> Option<&Contact> {
        self.contacts.iter().find(|c| c.tels.iter().any(|t| same_number(&t.number, number)))
    }

    /// The name to show for a number (a contact's name, else the number written nicely).
    pub fn name_of(&self, number: &str) -> String {
        if number == "anonymous" || number.is_empty() {
            return "No Caller ID".into();
        }
        self.find(number).map(|c| c.name.clone()).unwrap_or_else(|| crate::pretty(number))
    }

    pub fn by_uid(&self, uid: &str) -> Option<&Contact> {
        self.contacts.iter().find(|c| c.uid == uid)
    }

    /// Add or replace (by uid; a new uid when empty). Returns the uid.
    pub fn put(&mut self, mut c: Contact) -> String {
        if c.uid.is_empty() {
            let mut n = self.contacts.len() + 1;
            while self.contacts.iter().any(|x| x.uid == format!("c{n}")) {
                n += 1;
            }
            c.uid = format!("c{n}");
        }
        let uid = c.uid.clone();
        self.contacts.retain(|x| x.uid != uid);
        self.contacts.push(c);
        self.contacts.sort_by_key(|c| c.name.to_lowercase());
        uid
    }

    pub fn remove(&mut self, uid: &str) -> bool {
        let n = self.contacts.len();
        self.contacts.retain(|c| c.uid != uid);
        n != self.contacts.len()
    }

    /// Contacts whose name or a number matches `q` (case-insensitive; empty: all).
    pub fn search(&self, q: &str) -> Vec<&Contact> {
        let q = q.trim().to_lowercase();
        if q.is_empty() {
            return self.contacts.iter().collect();
        }
        let qd = digits(&q);
        self.contacts.iter().filter(|c| c.name.to_lowercase().contains(&q) || (!qd.is_empty() && c.tels.iter().any(|t| digits(&t.number).contains(&qd)))).collect()
    }

    /// Favourites (the Phone app's first tab).
    pub fn favourites(&self) -> Vec<&Contact> {
        self.contacts.iter().filter(|c| c.favourite).collect()
    }

    /// Import a `.vcf` text: new contacts added, ones with a number already in the book merged.
    pub fn import(&mut self, text: &str) -> Result<usize, String> {
        let cs = vcard::parse(text).map_err(|e| e.to_string())?;
        let n = cs.len();
        for mut c in cs {
            if let Some(existing) = c.primary().and_then(|p| self.find(p)).map(|e| e.uid.clone()) {
                c.uid = existing;
            } else {
                c.uid.clear();
            }
            self.put(c);
        }
        Ok(n)
    }

    /// The letters the A-Z index shows (first letters of names, `#` for the rest).
    pub fn letters(&self) -> Vec<char> {
        let mut v: Vec<char> = self.contacts.iter().map(|c| c.name.chars().next().map(|x| x.to_ascii_uppercase()).filter(|x| x.is_ascii_alphabetic()).unwrap_or('#')).collect();
        v.dedup();
        v
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fake::MemFiles;

    #[test]
    fn book_round_trip_and_lookup() {
        let mut f = MemFiles::default();
        let mut b = Book::load(&mut f);
        assert!(b.contacts.is_empty());
        let a = b.put(Contact::new("Alex Smith", "+12145550142"));
        b.put(Contact::new("bo", "(214) 555-0143"));
        b.save(&mut f).unwrap();
        let b2 = Book::load(&mut f);
        assert_eq!(b2.contacts.len(), 2);
        assert_eq!(b2.name_of("+12145550143"), "bo");
        assert_eq!(b2.name_of("2145550142"), "Alex Smith");
        assert_eq!(b2.name_of("+12145550199"), "(214) 555-0199");
        assert_eq!(b2.name_of("anonymous"), "No Caller ID");
        assert_eq!(b2.search("555-0143").len(), 1);
        assert_eq!(b2.search("ALEX").len(), 1);
        assert_eq!(b2.letters(), vec!['A', 'B']);
        let mut b3 = b2.clone();
        assert!(b3.remove(&a));
        assert_eq!(b3.import("BEGIN:VCARD\nVERSION:3.0\nFN:Cy\nTEL:+12145550144\nEND:VCARD\nBEGIN:VCARD\nVERSION:3.0\nFN:Bo Jones\nTEL:+12145550143\nEND:VCARD\n").unwrap(), 2);
        assert_eq!(b3.contacts.len(), 2, "bo merged, cy added");
        assert!(b3.find("+12145550143").is_some_and(|c| c.name == "Bo Jones"));
    }
}
