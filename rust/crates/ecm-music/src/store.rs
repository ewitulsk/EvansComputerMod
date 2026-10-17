//! Per-user data `musicd` keeps in `/var/music`: playlists, Liked Songs,
//! recently played and play counts.
//!
//! The permission rules live here rather than in the daemon's handlers so
//! they are the same for every request: only a playlist's owner changes it,
//! and anyone else sees it only while it is public. A song id that later
//! leaves the library stays in playlists (the app shows it greyed out) until
//! [`Store::forget_songs`] is called.

use crate::search::PlaylistDoc;
use crate::text;
use std::collections::{BTreeMap, HashMap};

/// How many recently played songs are kept per user.
pub const RECENT_MAX: usize = 50;
/// Longest playlist name, in characters.
pub const NAME_MAX: usize = 100;
/// Most songs in one playlist.
pub const PLAYLIST_MAX: usize = 5000;

/// Why a change was refused.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum StoreError {
    /// No such playlist, or one the user may not see.
    NotFound,
    /// The user may see the playlist but doesn't own it.
    NotOwner,
    BadName,
    BadIndex,
    TooMany,
}

impl StoreError {
    /// The HTTP status the daemon answers with.
    pub fn status(&self) -> u16 {
        match self {
            StoreError::NotFound => 404,
            StoreError::NotOwner => 403,
            _ => 400,
        }
    }

    pub fn message(&self) -> &'static str {
        match self {
            StoreError::NotFound => "no such playlist",
            StoreError::NotOwner => "only the owner can change this playlist",
            StoreError::BadName => "a playlist name must be 1 to 100 characters",
            StoreError::BadIndex => "no song at that position",
            StoreError::TooMany => "a playlist holds at most 5000 songs",
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Playlist {
    pub id: String,
    pub name: String,
    pub owner: String,
    pub public: bool,
    /// Song ids in play order (the same song may appear twice).
    pub songs: Vec<String>,
    pub created: u64,
    pub updated: u64,
}

#[derive(Clone, Debug, Default, PartialEq)]
struct User {
    /// Most recently liked first.
    liked: Vec<String>,
    /// Most recently played first, no repeats.
    recent: Vec<String>,
    plays: BTreeMap<String, u64>,
}

/// All users' data.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Store {
    playlists: BTreeMap<String, Playlist>,
    users: BTreeMap<String, User>,
    /// Play counts summed over users.
    totals: HashMap<String, u64>,
    next_id: u64,
}

fn check_name(name: &str) -> Result<String, StoreError> {
    let n = name.trim();
    if n.is_empty() || n.chars().count() > NAME_MAX || n.chars().any(|c| c.is_control()) {
        return Err(StoreError::BadName);
    }
    Ok(n.to_string())
}

impl Store {
    pub fn new() -> Store {
        Store::default()
    }

    /// Create a playlist owned by `owner`; returns its id.
    pub fn create_playlist(&mut self, owner: &str, name: &str, public: bool, now: u64) -> Result<String, StoreError> {
        let name = check_name(name)?;
        self.next_id += 1;
        let id = format!("p{}", self.next_id);
        self.playlists.insert(
            id.clone(),
            Playlist { id: id.clone(), name, owner: owner.to_string(), public, songs: Vec::new(), created: now, updated: now },
        );
        Ok(id)
    }

    /// A playlist as `viewer` (None: signed out) may see it.
    pub fn playlist(&self, viewer: Option<&str>, id: &str) -> Result<&Playlist, StoreError> {
        match self.playlists.get(id) {
            Some(p) if p.public || viewer == Some(p.owner.as_str()) => Ok(p),
            _ => Err(StoreError::NotFound),
        }
    }

    fn owned(&mut self, user: &str, id: &str) -> Result<&mut Playlist, StoreError> {
        let p = self.playlists.get_mut(id).ok_or(StoreError::NotFound)?;
        if p.owner == user {
            Ok(p)
        } else if p.public {
            Err(StoreError::NotOwner)
        } else {
            Err(StoreError::NotFound) // a private playlist doesn't exist for others
        }
    }

    pub fn rename(&mut self, user: &str, id: &str, name: &str, now: u64) -> Result<(), StoreError> {
        let name = check_name(name)?;
        let p = self.owned(user, id)?;
        p.name = name;
        p.updated = now;
        Ok(())
    }

    pub fn set_public(&mut self, user: &str, id: &str, public: bool, now: u64) -> Result<(), StoreError> {
        let p = self.owned(user, id)?;
        p.public = public;
        p.updated = now;
        Ok(())
    }

    pub fn delete_playlist(&mut self, user: &str, id: &str) -> Result<(), StoreError> {
        self.owned(user, id)?;
        self.playlists.remove(id);
        Ok(())
    }

    /// Insert songs at `at` (None: at the end).
    pub fn add_songs(&mut self, user: &str, id: &str, songs: &[String], at: Option<usize>, now: u64) -> Result<(), StoreError> {
        let p = self.owned(user, id)?;
        if p.songs.len() + songs.len() > PLAYLIST_MAX {
            return Err(StoreError::TooMany);
        }
        let at = at.unwrap_or(p.songs.len());
        if at > p.songs.len() {
            return Err(StoreError::BadIndex);
        }
        p.songs.splice(at..at, songs.iter().cloned());
        p.updated = now;
        Ok(())
    }

    /// Remove the song at position `index`.
    pub fn remove_at(&mut self, user: &str, id: &str, index: usize, now: u64) -> Result<String, StoreError> {
        let p = self.owned(user, id)?;
        if index >= p.songs.len() {
            return Err(StoreError::BadIndex);
        }
        p.updated = now;
        Ok(p.songs.remove(index))
    }

    /// Remove every occurrence of `song`; returns how many were removed.
    pub fn remove_song(&mut self, user: &str, id: &str, song: &str, now: u64) -> Result<usize, StoreError> {
        let p = self.owned(user, id)?;
        let before = p.songs.len();
        p.songs.retain(|s| s != song);
        p.updated = now;
        Ok(before - p.songs.len())
    }

    /// Move the song at `from` so it ends up at `to`.
    pub fn move_song(&mut self, user: &str, id: &str, from: usize, to: usize, now: u64) -> Result<(), StoreError> {
        let p = self.owned(user, id)?;
        if from >= p.songs.len() || to >= p.songs.len() {
            return Err(StoreError::BadIndex);
        }
        let s = p.songs.remove(from);
        p.songs.insert(to, s);
        p.updated = now;
        Ok(())
    }

    /// Replace the order: `order` must hold exactly the playlist's songs.
    pub fn reorder(&mut self, user: &str, id: &str, order: &[String], now: u64) -> Result<(), StoreError> {
        let p = self.owned(user, id)?;
        let mut a = p.songs.clone();
        let mut b = order.to_vec();
        a.sort();
        b.sort();
        if a != b {
            return Err(StoreError::BadIndex);
        }
        p.songs = order.to_vec();
        p.updated = now;
        Ok(())
    }

    /// `owner`'s own playlists, newest change first.
    pub fn playlists_of(&self, owner: &str) -> Vec<&Playlist> {
        let mut v: Vec<&Playlist> = self.playlists.values().filter(|p| p.owner == owner).collect();
        v.sort_by(|a, b| b.updated.cmp(&a.updated).then(a.id.cmp(&b.id)));
        v
    }

    /// Every playlist `viewer` may see.
    pub fn visible_playlists(&self, viewer: Option<&str>) -> Vec<&Playlist> {
        self.playlists.values().filter(|p| p.public || viewer == Some(p.owner.as_str())).collect()
    }

    /// All playlists for [`crate::search::Index::build`] (the index itself
    /// hides private ones from other users).
    pub fn search_docs(&self) -> Vec<PlaylistDoc> {
        self.playlists.values().map(|p| PlaylistDoc { id: p.id.clone(), name: p.name.clone(), owner: p.owner.clone(), public: p.public }).collect()
    }

    /// Like `song` (moving it to the top if already liked).
    pub fn like(&mut self, user: &str, song: &str) {
        let u = self.users.entry(user.to_string()).or_default();
        u.liked.retain(|s| s != song);
        u.liked.insert(0, song.to_string());
    }

    pub fn unlike(&mut self, user: &str, song: &str) -> bool {
        self.users.get_mut(user).is_some_and(|u| {
            let n = u.liked.len();
            u.liked.retain(|s| s != song);
            n != u.liked.len()
        })
    }

    pub fn is_liked(&self, user: &str, song: &str) -> bool {
        self.users.get(user).is_some_and(|u| u.liked.iter().any(|s| s == song))
    }

    /// Liked Songs, most recent first.
    pub fn liked(&self, user: &str) -> &[String] {
        self.users.get(user).map(|u| u.liked.as_slice()).unwrap_or(&[])
    }

    /// Record a play: counts it and puts the song at the top of recently played.
    pub fn played(&mut self, user: &str, song: &str) {
        let u = self.users.entry(user.to_string()).or_default();
        u.recent.retain(|s| s != song);
        u.recent.insert(0, song.to_string());
        u.recent.truncate(RECENT_MAX);
        *u.plays.entry(song.to_string()).or_default() += 1;
        *self.totals.entry(song.to_string()).or_default() += 1;
    }

    /// Recently played, most recent first.
    pub fn recent(&self, user: &str) -> &[String] {
        self.users.get(user).map(|u| u.recent.as_slice()).unwrap_or(&[])
    }

    /// Plays of `song` by everyone (for search ranking).
    pub fn play_count(&self, song: &str) -> u64 {
        self.totals.get(song).copied().unwrap_or(0)
    }

    /// Plays of `song` by `user`.
    pub fn user_play_count(&self, user: &str, song: &str) -> u64 {
        self.users.get(user).and_then(|u| u.plays.get(song)).copied().unwrap_or(0)
    }

    /// Drop songs that left the library from likes, recents, counts and
    /// playlists.
    pub fn forget_songs(&mut self, exists: impl Fn(&str) -> bool) {
        for p in self.playlists.values_mut() {
            p.songs.retain(|s| exists(s));
        }
        for u in self.users.values_mut() {
            u.liked.retain(|s| exists(s));
            u.recent.retain(|s| exists(s));
            u.plays.retain(|s, _| exists(s));
        }
        self.totals.retain(|s, _| exists(s));
    }

    /// Everything as text.
    pub fn save(&self) -> String {
        let mut out = format!("ecm-music-store\t1\t{}\n", self.next_id);
        for p in self.playlists.values() {
            let created = p.created.to_string();
            let updated = p.updated.to_string();
            let mut f: Vec<&str> = vec!["playlist", &p.id, &p.owner, if p.public { "1" } else { "0" }, &created, &updated, &p.name];
            f.extend(p.songs.iter().map(String::as_str));
            out.push_str(&text::line(&f));
        }
        for (name, u) in &self.users {
            for (kind, list) in [("liked", &u.liked), ("recent", &u.recent)] {
                if !list.is_empty() {
                    let mut f: Vec<&str> = vec![kind, name];
                    f.extend(list.iter().map(String::as_str));
                    out.push_str(&text::line(&f));
                }
            }
            if !u.plays.is_empty() {
                let pairs: Vec<String> = u.plays.iter().map(|(s, n)| format!("{s}={n}")).collect();
                let mut f: Vec<&str> = vec!["plays", name];
                f.extend(pairs.iter().map(String::as_str));
                out.push_str(&text::line(&f));
            }
        }
        out
    }

    /// Read what [`Store::save`] wrote.
    pub fn load(text: &str) -> Result<Store, String> {
        let mut lines = text.lines();
        let h = text::fields(lines.next().ok_or("empty store")?);
        if h.first().map(String::as_str) != Some("ecm-music-store") || h.get(1).map(String::as_str) != Some("1") {
            return Err("not an ecm-music store".into());
        }
        let mut st = Store { next_id: h.get(2).and_then(|n| n.parse().ok()).unwrap_or(0), ..Store::default() };
        for (n, l) in lines.enumerate() {
            if l.is_empty() {
                continue;
            }
            let f = text::fields(l);
            let bad = || format!("store line {}: bad record", n + 2);
            match f[0].as_str() {
                "playlist" if f.len() >= 7 => {
                    let p = Playlist {
                        id: f[1].clone(),
                        owner: f[2].clone(),
                        public: f[3] == "1",
                        created: f[4].parse().map_err(|_| bad())?,
                        updated: f[5].parse().map_err(|_| bad())?,
                        name: f[6].clone(),
                        songs: f[7..].to_vec(),
                    };
                    if let Some(k) = p.id.strip_prefix('p').and_then(|k| k.parse::<u64>().ok()) {
                        st.next_id = st.next_id.max(k);
                    }
                    st.playlists.insert(p.id.clone(), p);
                }
                "liked" | "recent" if f.len() >= 2 => {
                    let u = st.users.entry(f[1].clone()).or_default();
                    let list = f[2..].to_vec();
                    if f[0] == "liked" {
                        u.liked = list;
                    } else {
                        u.recent = list;
                        u.recent.truncate(RECENT_MAX);
                    }
                }
                "plays" if f.len() >= 2 => {
                    let u = st.users.entry(f[1].clone()).or_default();
                    for pair in &f[2..] {
                        let (s, c) = pair.rsplit_once('=').ok_or_else(bad)?;
                        let c: u64 = c.parse().map_err(|_| bad())?;
                        u.plays.insert(s.to_string(), c);
                        *st.totals.entry(s.to_string()).or_default() += c;
                    }
                }
                _ => return Err(bad()),
            }
        }
        Ok(st)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ids(v: &[&str]) -> Vec<String> {
        v.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn playlist_editing_and_permissions() {
        let mut st = Store::new();
        let id = st.create_playlist("alice", "  Road Trip ", false, 1).unwrap();
        assert_eq!(st.playlist(Some("alice"), &id).unwrap().name, "Road Trip");
        assert_eq!(st.playlist(Some("bob"), &id), Err(StoreError::NotFound), "private: invisible to others");
        assert_eq!(st.playlist(None, &id), Err(StoreError::NotFound));
        assert_eq!(st.add_songs("bob", &id, &ids(&["s1"]), None, 2), Err(StoreError::NotFound));
        st.add_songs("alice", &id, &ids(&["s1", "s2", "s3"]), None, 2).unwrap();
        st.add_songs("alice", &id, &ids(&["s0"]), Some(0), 3).unwrap();
        assert_eq!(st.playlist(Some("alice"), &id).unwrap().songs, ids(&["s0", "s1", "s2", "s3"]));
        st.move_song("alice", &id, 0, 3, 4).unwrap();
        assert_eq!(st.playlist(Some("alice"), &id).unwrap().songs, ids(&["s1", "s2", "s3", "s0"]));
        st.reorder("alice", &id, &ids(&["s3", "s2", "s1", "s0"]), 5).unwrap();
        assert_eq!(st.reorder("alice", &id, &ids(&["s3", "s2"]), 5), Err(StoreError::BadIndex));
        assert_eq!(st.remove_at("alice", &id, 1, 6).unwrap(), "s2");
        assert_eq!(st.remove_at("alice", &id, 9, 6), Err(StoreError::BadIndex));
        assert_eq!(st.remove_song("alice", &id, "s0", 6).unwrap(), 1);
        st.set_public("alice", &id, true, 7).unwrap();
        assert_eq!(st.playlist(Some("bob"), &id).unwrap().songs, ids(&["s3", "s1"]), "public: visible");
        assert_eq!(st.rename("bob", &id, "Mine now", 8), Err(StoreError::NotOwner));
        assert_eq!(st.delete_playlist("bob", &id), Err(StoreError::NotOwner));
        assert_eq!(st.rename("alice", &id, "", 8), Err(StoreError::BadName));
        assert_eq!(st.rename("alice", &id, &"x".repeat(101), 8), Err(StoreError::BadName));
        st.rename("alice", &id, "Summer", 8).unwrap();
        let other = st.create_playlist("alice", "Later", true, 9).unwrap();
        assert_eq!(st.playlists_of("alice").iter().map(|p| p.id.as_str()).collect::<Vec<_>>(), vec![other.as_str(), id.as_str()]);
        assert_eq!(st.visible_playlists(Some("bob")).len(), 2);
        st.delete_playlist("alice", &id).unwrap();
        assert_eq!(st.playlist(Some("alice"), &id), Err(StoreError::NotFound));
        assert_eq!(StoreError::NotOwner.status(), 403);
    }

    #[test]
    fn likes_recent_and_play_counts() {
        let mut st = Store::new();
        st.like("alice", "s1");
        st.like("alice", "s2");
        st.like("alice", "s1");
        assert_eq!(st.liked("alice"), ids(&["s1", "s2"]).as_slice());
        assert!(st.unlike("alice", "s2") && !st.unlike("alice", "s2"));
        assert!(st.is_liked("alice", "s1") && !st.is_liked("bob", "s1"));
        for i in 0..60 {
            st.played("alice", &format!("t{i}"));
        }
        st.played("alice", "t10");
        st.played("bob", "t10");
        let r = st.recent("alice");
        assert_eq!(r.len(), RECENT_MAX);
        assert_eq!(r[0], "t10");
        assert_eq!(r.iter().filter(|s| *s == "t10").count(), 1, "no repeats");
        assert_eq!(st.play_count("t10"), 3);
        assert_eq!(st.user_play_count("alice", "t10"), 2);
        st.forget_songs(|s| s != "t10");
        assert_eq!(st.play_count("t10"), 0);
        assert!(st.recent("bob").is_empty());
    }

    #[test]
    fn save_and_load_round_trip() {
        let mut st = Store::new();
        let id = st.create_playlist("al\tice", "Name = with \\ backslash", true, 5).unwrap();
        st.add_songs("al\tice", &id, &ids(&["s1", "s1", "s=2"]), None, 6).unwrap();
        st.create_playlist("bob", "Empty", false, 7).unwrap();
        st.like("bob", "s1");
        st.played("bob", "s=2");
        st.played("bob", "s=2");
        let text = st.save();
        let back = Store::load(&text).unwrap();
        assert_eq!(back, st);
        assert_eq!(back.play_count("s=2"), 2);
        let mut back = back;
        let next = back.create_playlist("bob", "Third", false, 8).unwrap();
        assert_eq!(next, "p3", "ids keep counting after a load");
        assert!(Store::load("nope").is_err());
        assert!(Store::load("ecm-music-store\t1\t0\nwhat\tever").is_err());
    }
}
