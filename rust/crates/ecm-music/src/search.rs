//! Search: the in-memory inverted index `musicd` rebuilds on every scan.
//!
//! Every song, artist, album and playlist is a document with weighted fields
//! (a song's title counts more than its artist, its artist more than its
//! album, its album more than its genre). Text is cut into normalised tokens
//! (lower case, accents folded, punctuation dropped, a leading "the"/"a" of a
//! name ignored, `&` read as "and"), and each token points at the documents
//! and fields it occurs in.
//!
//! A query word matches a token exactly, then as a prefix (search-as-you-type:
//! `daft pu` finds Daft Punk), then with a typo (edit distance 1 for words of
//! 4+ letters, 2 for 8+) when nothing matched exactly. Results are ranked by
//! how many query words matched, then by score (field weight times match
//! quality, plus a bonus for a field matched completely), with the play count
//! as a light tie-breaker. `artist:`, `album:`, `genre:` and `year:` filters
//! narrow the songs (`year:1990-1999`, `artist:"daft punk"`).
//!
//! Private playlists are only found by their owner.

use crate::text::fnv64;
use std::collections::HashMap;

/// A song as search sees it.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct SongDoc {
    pub id: String,
    pub title: String,
    pub artist: String,
    pub album: String,
    pub album_artist: String,
    pub genre: String,
    pub year: Option<u32>,
    pub plays: u64,
}

/// A playlist as search sees it.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct PlaylistDoc {
    pub id: String,
    pub name: String,
    pub owner: String,
    pub public: bool,
}

/// What kind of thing a hit is.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum HitKind {
    Song,
    Artist,
    Album,
    Playlist,
}

impl HitKind {
    pub fn as_str(&self) -> &'static str {
        match self {
            HitKind::Song => "song",
            HitKind::Artist => "artist",
            HitKind::Album => "album",
            HitKind::Playlist => "playlist",
        }
    }
}

/// One search result.
#[derive(Clone, Debug, PartialEq)]
pub struct Hit {
    pub kind: HitKind,
    pub id: String,
    /// Title, artist name, album name or playlist name.
    pub name: String,
    /// The artist of a song or album, the owner of a playlist.
    pub subtitle: String,
    /// For songs: the position in the list given to [`Index::build`].
    pub index: usize,
    /// How many query words matched.
    pub matched: u8,
    pub score: f32,
}

/// A grouped answer: the best hit of any kind, then each kind in rank order
/// (paged by `limit`/`offset`), with the totals before paging.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Results {
    pub top: Option<Hit>,
    pub songs: Vec<Hit>,
    pub artists: Vec<Hit>,
    pub albums: Vec<Hit>,
    pub playlists: Vec<Hit>,
    pub total_songs: usize,
    pub total_artists: usize,
    pub total_albums: usize,
    pub total_playlists: usize,
}

// ---------------------------------------------------------------------------
// Normalisation

/// Latin-1 Supplement and Latin Extended-A letters folded to ASCII.
fn fold(c: char) -> Option<&'static str> {
    let s = match c as u32 {
        0xc0..=0xc5 | 0xe0..=0xe5 => "a",
        0xc6 | 0xe6 => "ae",
        0xc7 | 0xe7 => "c",
        0xc8..=0xcb | 0xe8..=0xeb => "e",
        0xcc..=0xcf | 0xec..=0xef => "i",
        0xd0 | 0xf0 => "d",
        0xd1 | 0xf1 => "n",
        0xd2..=0xd6 | 0xf2..=0xf6 | 0xd8 | 0xf8 => "o",
        0xd9..=0xdc | 0xf9..=0xfc => "u",
        0xdd | 0xfd | 0xff => "y",
        0xde | 0xfe => "th",
        0xdf => "ss",
        0x100..=0x105 => "a",
        0x106..=0x10d => "c",
        0x10e..=0x111 => "d",
        0x112..=0x11b => "e",
        0x11c..=0x123 => "g",
        0x124..=0x127 => "h",
        0x128..=0x131 => "i",
        0x132..=0x133 => "ij",
        0x134..=0x135 => "j",
        0x136..=0x138 => "k",
        0x139..=0x142 => "l",
        0x143..=0x14b => "n",
        0x14c..=0x151 => "o",
        0x152..=0x153 => "oe",
        0x154..=0x159 => "r",
        0x15a..=0x161 | 0x17f => "s",
        0x162..=0x167 => "t",
        0x168..=0x173 => "u",
        0x174..=0x175 => "w",
        0x176..=0x178 => "y",
        0x179..=0x17e => "z",
        _ => return None,
    };
    Some(s)
}

/// Cut `s` into normalised tokens. Apostrophes and dots join (`don't` is
/// `dont`, `R.E.M.` is `rem`); other punctuation separates; `&` is `and`;
/// combining accents are dropped.
pub fn normalize(s: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut cur = String::new();
    let flush = |cur: &mut String, out: &mut Vec<String>| {
        if !cur.is_empty() {
            out.push(std::mem::take(cur));
        }
    };
    for c in s.chars() {
        if c.is_ascii_alphanumeric() {
            cur.push(c.to_ascii_lowercase());
        } else if let Some(f) = fold(c) {
            cur.push_str(f);
        } else if matches!(c, '\'' | '\u{2019}' | '\u{2018}' | '.' | '\u{b4}' | '`') || ('\u{300}'..='\u{36f}').contains(&c) {
            // joins: nothing to add
        } else if c == '&' {
            flush(&mut cur, &mut out);
            out.push("and".into());
        } else if c.is_alphanumeric() {
            cur.extend(c.to_lowercase());
        } else {
            flush(&mut cur, &mut out);
        }
    }
    flush(&mut cur, &mut out);
    out
}

/// Tokens of a name (title, artist, album, playlist): a leading "the" or "a"
/// is ignored when more words follow, so "The Beatles" is found as "beatles".
pub fn name_tokens(s: &str) -> Vec<String> {
    let mut t = normalize(s);
    if t.len() > 1 && (t[0] == "the" || t[0] == "a") {
        t.remove(0);
    }
    t
}

fn id_of(prefix: &str, key: &str) -> String {
    format!("{prefix}{:016x}", fnv64(key))
}

/// The id of an artist: the same for every spelling that normalises alike
/// ("The Beatles", "beatles").
pub fn artist_id(name: &str) -> String {
    let t = name_tokens(name);
    id_of("ar", &if t.is_empty() { name.trim().to_lowercase() } else { t.join(" ") })
}

/// The id of an album by `artist` (the album artist when there is one).
pub fn album_id(artist: &str, album: &str) -> String {
    let a = name_tokens(artist).join(" ");
    let b = name_tokens(album);
    let b = if b.is_empty() { album.trim().to_lowercase() } else { b.join(" ") };
    id_of("al", &format!("{a}\u{1}{b}"))
}

/// A bit per letter/digit in `chars` (others share one bit). One edit
/// changes this set by at most two members, which rules most of the
/// vocabulary out of typo matching before the distance is computed.
fn char_mask(chars: &[char]) -> u64 {
    chars.iter().fold(0u64, |m, &c| {
        m | 1 << match c {
            'a'..='z' => c as u32 - 'a' as u32,
            '0'..='9' => 26 + c as u32 - '0' as u32,
            _ => 63,
        }
    })
}

/// Optimal-string-alignment edit distance (a swap of two neighbours counts
/// once), or None when it is above `max`.
pub fn edit_distance(a: &[char], b: &[char], max: usize) -> Option<usize> {
    if a.len().abs_diff(b.len()) > max {
        return None;
    }
    // Three rows on the stack: words longer than 63 letters aren't typo-matched.
    let n = b.len();
    if n >= 64 || a.len() >= 64 {
        return (a == b).then_some(0);
    }
    let mut prev2 = [0usize; 64];
    let mut prev = [0usize; 64];
    let mut cur = [0usize; 64];
    for (j, p) in prev.iter_mut().enumerate().take(n + 1) {
        *p = j;
    }
    for i in 1..=a.len() {
        cur[0] = i;
        let mut row_min = cur[0];
        for j in 1..=n {
            let cost = (a[i - 1] != b[j - 1]) as usize;
            let mut v = (prev[j] + 1).min(cur[j - 1] + 1).min(prev[j - 1] + cost);
            if i > 1 && j > 1 && a[i - 1] == b[j - 2] && a[i - 2] == b[j - 1] {
                v = v.min(prev2[j - 2] + 1);
            }
            cur[j] = v;
            row_min = row_min.min(v);
        }
        if row_min > max {
            return None;
        }
        std::mem::swap(&mut prev2, &mut prev);
        std::mem::swap(&mut prev, &mut cur);
    }
    (prev[n] <= max).then_some(prev[n])
}

// ---------------------------------------------------------------------------
// The index

const FIELDS: usize = 5;
const MAX_WORDS: usize = 8;

/// Field weights by kind: song title, artist, album artist, album, genre;
/// artist name; album name, album artist; playlist name.
fn weight(kind: HitKind, field: usize) -> f32 {
    match (kind, field) {
        (HitKind::Song, 0) => 10.0,
        (HitKind::Song, 1) => 6.0,
        (HitKind::Song, 2) => 5.0,
        (HitKind::Song, 3) => 4.0,
        (HitKind::Song, _) => 2.0,
        (HitKind::Album, 1) => 5.0,
        _ => 9.0,
    }
}

const EXACT: f32 = 1.0;
const FUZZY1: f32 = 0.45;
const FUZZY2: f32 = 0.3;
const PLAYS_WEIGHT: f32 = 0.15;

struct Entity {
    kind: HitKind,
    /// Index into songs/artists/albums/playlists.
    idx: u32,
    plays: u64,
    /// Distinct tokens per field (for the "whole field matched" bonus).
    field_len: [u8; FIELDS],
}

struct SongEntry {
    doc: SongDoc,
    entity: u32,
    artist: Option<u32>,
    album: Option<u32>,
    artist_toks: Vec<String>,
    album_toks: Vec<String>,
    genre_toks: Vec<String>,
}

struct Group {
    id: String,
    name: String,
    subtitle: String,
    entity: u32,
    songs: Vec<u32>,
}

/// The search index over one library and its playlists.
pub struct Index {
    entities: Vec<Entity>,
    songs: Vec<SongEntry>,
    artists: Vec<Group>,
    albums: Vec<Group>,
    playlists: Vec<(PlaylistDoc, u32)>,
    /// Sorted vocabulary, with its postings and characters.
    vocab: Vec<String>,
    chars: Vec<Vec<char>>,
    masks: Vec<u64>,
    postings: Vec<Vec<(u32, u8)>>,
    lookup: HashMap<String, u32>,
    /// Token ids by character count (for typo matching).
    by_len: Vec<Vec<u32>>,
    song_by_id: HashMap<String, u32>,
}

#[derive(Clone, Copy, Default)]
struct Acc {
    best: [f32; MAX_WORDS],
    mask: u8,
    cover: [u8; FIELDS],
}

#[derive(Default)]
struct Filters {
    artist: Vec<String>,
    album: Vec<String>,
    genre: Vec<String>,
    year: Option<(u32, u32)>,
}

impl Filters {
    fn any(&self) -> bool {
        !self.artist.is_empty() || !self.album.is_empty() || !self.genre.is_empty() || self.year.is_some()
    }
}

/// Split a query into free words and filters.
fn parse_query(q: &str) -> (Vec<String>, Filters) {
    let mut free = String::new();
    let mut f = Filters::default();
    let chars: Vec<char> = q.chars().collect();
    let mut i = 0;
    while i < chars.len() {
        if chars[i].is_whitespace() {
            i += 1;
            continue;
        }
        let start = i;
        while i < chars.len() && !chars[i].is_whitespace() && chars[i] != ':' {
            i += 1;
        }
        let key: String = chars[start..i].iter().collect::<String>().to_lowercase();
        let is_filter = i < chars.len() && chars[i] == ':' && matches!(key.as_str(), "artist" | "album" | "genre" | "year");
        if !is_filter {
            while i < chars.len() && !chars[i].is_whitespace() {
                i += 1;
            }
            free.push_str(&chars[start..i].iter().collect::<String>());
            free.push(' ');
            continue;
        }
        i += 1; // ':'
        let value: String = if i < chars.len() && chars[i] == '"' {
            i += 1;
            let s = i;
            while i < chars.len() && chars[i] != '"' {
                i += 1;
            }
            let v = chars[s..i].iter().collect();
            i += 1;
            v
        } else {
            let s = i;
            while i < chars.len() && !chars[i].is_whitespace() {
                i += 1;
            }
            chars[s..i].iter().collect()
        };
        match key.as_str() {
            "artist" => f.artist.extend(name_tokens(&value)),
            "album" => f.album.extend(name_tokens(&value)),
            "genre" => f.genre.extend(normalize(&value)),
            _ => {
                let mut parts = value.splitn(2, '-').map(|p| p.trim().parse::<u32>().ok());
                let a = parts.next().flatten();
                let b = parts.next();
                f.year = match (a, b) {
                    (Some(a), None) => Some((a, a)),
                    (Some(a), Some(Some(b))) => Some((a.min(b), a.max(b))),
                    (Some(a), Some(None)) => Some((a, u32::MAX)),
                    (None, Some(Some(b))) => Some((0, b)),
                    _ => Some((1, 0)), // unreadable: matches nothing
                };
            }
        }
    }
    let mut words = name_tokens(&free);
    words.truncate(MAX_WORDS);
    (words, f)
}

/// Every filter token matches a token of `field` (the last one may be a prefix).
fn tokens_match(want: &[String], field: &[String]) -> bool {
    want.iter().enumerate().all(|(i, w)| field.iter().any(|t| t == w || (i + 1 == want.len() && t.starts_with(w.as_str()))))
}

impl Index {
    /// Build the index over `songs` and `playlists`. Artists and albums are
    /// grouped from the songs with [`artist_id`] and [`album_id`].
    pub fn build(songs: &[SongDoc], playlists: &[PlaylistDoc]) -> Index {
        let mut ix = Index {
            entities: Vec::new(),
            songs: Vec::with_capacity(songs.len()),
            artists: Vec::new(),
            albums: Vec::new(),
            playlists: Vec::new(),
            vocab: Vec::new(),
            chars: Vec::new(),
            masks: Vec::new(),
            postings: Vec::new(),
            lookup: HashMap::new(),
            by_len: Vec::new(),
            song_by_id: HashMap::new(),
        };
        let mut terms: HashMap<String, Vec<(u32, u8)>> = HashMap::new();
        let mut add = |entities: &mut Vec<Entity>, kind: HitKind, idx: u32, plays: u64, fields: [Vec<String>; FIELDS]| -> u32 {
            let e = entities.len() as u32;
            let mut field_len = [0u8; FIELDS];
            for (fi, toks) in fields.iter().enumerate() {
                let mut seen: Vec<&str> = Vec::new();
                for t in toks {
                    if seen.contains(&t.as_str()) {
                        continue;
                    }
                    seen.push(t);
                    terms.entry(t.clone()).or_default().push((e, fi as u8));
                }
                field_len[fi] = seen.len().min(255) as u8;
            }
            entities.push(Entity { kind, idx, plays, field_len });
            e
        };
        let mut artist_ix: HashMap<String, u32> = HashMap::new();
        let mut album_ix: HashMap<String, u32> = HashMap::new();
        for (i, s) in songs.iter().enumerate() {
            let artist_toks = name_tokens(&s.artist);
            let album_artist_toks = name_tokens(&s.album_artist);
            let album_toks = name_tokens(&s.album);
            let genre_toks = normalize(&s.genre);
            let e = add(
                &mut ix.entities,
                HitKind::Song,
                i as u32,
                s.plays,
                [
                    name_tokens(&s.title),
                    artist_toks.clone(),
                    if album_artist_toks == artist_toks { Vec::new() } else { album_artist_toks.clone() },
                    album_toks.clone(),
                    genre_toks.clone(),
                ],
            );
            let artist = (!s.artist.trim().is_empty()).then(|| {
                let id = artist_id(&s.artist);
                *artist_ix.entry(id.clone()).or_insert_with(|| {
                    ix.artists.push(Group { id, name: s.artist.trim().to_string(), subtitle: String::new(), entity: u32::MAX, songs: Vec::new() });
                    (ix.artists.len() - 1) as u32
                })
            });
            let album = (!s.album.trim().is_empty()).then(|| {
                let by = if s.album_artist.trim().is_empty() { &s.artist } else { &s.album_artist };
                let id = album_id(by, &s.album);
                *album_ix.entry(id.clone()).or_insert_with(|| {
                    ix.albums.push(Group { id, name: s.album.trim().to_string(), subtitle: by.trim().to_string(), entity: u32::MAX, songs: Vec::new() });
                    (ix.albums.len() - 1) as u32
                })
            });
            if let Some(a) = artist {
                ix.artists[a as usize].songs.push(i as u32);
            }
            if let Some(a) = album {
                ix.albums[a as usize].songs.push(i as u32);
            }
            ix.song_by_id.insert(s.id.clone(), i as u32);
            ix.songs.push(SongEntry {
                doc: s.clone(),
                entity: e,
                artist,
                album,
                artist_toks: [artist_toks, album_artist_toks].concat(),
                album_toks,
                genre_toks,
            });
        }
        for a in 0..ix.artists.len() {
            let plays = ix.artists[a].songs.iter().map(|&s| songs[s as usize].plays).sum();
            let name = name_tokens(&ix.artists[a].name);
            ix.artists[a].entity = add(&mut ix.entities, HitKind::Artist, a as u32, plays, [name, vec![], vec![], vec![], vec![]]);
        }
        for a in 0..ix.albums.len() {
            let plays = ix.albums[a].songs.iter().map(|&s| songs[s as usize].plays).sum();
            let name = name_tokens(&ix.albums[a].name);
            let by = name_tokens(&ix.albums[a].subtitle);
            ix.albums[a].entity = add(&mut ix.entities, HitKind::Album, a as u32, plays, [name, by, vec![], vec![], vec![]]);
        }
        for (i, p) in playlists.iter().enumerate() {
            let e = add(&mut ix.entities, HitKind::Playlist, i as u32, 0, [name_tokens(&p.name), vec![], vec![], vec![], vec![]]);
            ix.playlists.push((p.clone(), e));
        }
        let mut vocab: Vec<(String, Vec<(u32, u8)>)> = terms.into_iter().collect();
        vocab.sort_by(|a, b| a.0.cmp(&b.0));
        for (i, (t, p)) in vocab.into_iter().enumerate() {
            let ch: Vec<char> = t.chars().collect();
            if ix.by_len.len() <= ch.len() {
                ix.by_len.resize(ch.len() + 1, Vec::new());
            }
            ix.by_len[ch.len()].push(i as u32);
            ix.lookup.insert(t.clone(), i as u32);
            ix.vocab.push(t);
            ix.masks.push(char_mask(&ch));
            ix.chars.push(ch);
            ix.postings.push(p);
        }
        ix
    }

    /// Number of songs indexed.
    pub fn len(&self) -> usize {
        self.songs.len()
    }

    pub fn is_empty(&self) -> bool {
        self.songs.is_empty()
    }

    /// Update a song's play count (and its artist's and album's totals)
    /// without rebuilding.
    pub fn set_plays(&mut self, song_id: &str, plays: u64) {
        let Some(&i) = self.song_by_id.get(song_id) else { return };
        let s = &mut self.songs[i as usize];
        let old = s.doc.plays;
        s.doc.plays = plays;
        let (e, artist, album) = (s.entity, s.artist, s.album);
        self.entities[e as usize].plays = plays;
        let mut adjust = |e: u32| {
            let p = &mut self.entities[e as usize].plays;
            *p = (*p + plays).saturating_sub(old);
        };
        if let Some(a) = artist {
            adjust(self.artists[a as usize].entity);
        }
        if let Some(a) = album {
            adjust(self.albums[a as usize].entity);
        }
    }

    fn song_passes(&self, s: &SongEntry, f: &Filters) -> bool {
        (f.artist.is_empty() || tokens_match(&f.artist, &s.artist_toks))
            && (f.album.is_empty() || tokens_match(&f.album, &s.album_toks))
            && (f.genre.is_empty() || tokens_match(&f.genre, &s.genre_toks))
            && f.year.map_or(true, |(a, b)| s.doc.year.is_some_and(|y| y >= a && y <= b))
    }

    /// Search for `query` as `user` (None when signed out: only public
    /// playlists). Each group is paged with `limit` and `offset`.
    pub fn search(&self, query: &str, user: Option<&str>, limit: usize, offset: usize) -> Results {
        let (words, filters) = parse_query(query);
        let filtered = filters.any();
        let passes: Vec<bool> = if filtered { self.songs.iter().map(|s| self.song_passes(s, &filters)).collect() } else { Vec::new() };
        let mut acc: Vec<Acc> = Vec::new();
        let mut touched: Vec<u32> = Vec::new();
        if words.is_empty() {
            if !filtered {
                return Results::default();
            }
            // Only filters: everything that passes, by play count.
            touched = self.songs.iter().enumerate().filter(|(i, _)| passes[*i]).map(|(_, s)| s.entity).collect();
            touched.extend(self.artists.iter().filter(|g| g.songs.iter().any(|&s| passes[s as usize])).map(|g| g.entity));
            touched.extend(self.albums.iter().filter(|g| g.songs.iter().any(|&s| passes[s as usize])).map(|g| g.entity));
            acc = vec![Acc::default(); self.entities.len()];
        } else {
            acc.resize(self.entities.len(), Acc::default());
            // Per (entity, field): the last word counted towards the field's
            // coverage, so a word matching two tokens of a field counts once.
            let mut stamp: Vec<u8> = vec![0; self.entities.len() * FIELDS];
            for (wi, w) in words.iter().enumerate() {
                let last = wi + 1 == words.len();
                let mut visit = |tok: u32, quality: f32, covers: bool, acc: &mut Vec<Acc>, touched: &mut Vec<u32>| {
                    for &(e, field) in &self.postings[tok as usize] {
                        let a = &mut acc[e as usize];
                        if a.mask == 0 {
                            touched.push(e);
                        }
                        let s = weight(self.entities[e as usize].kind, field as usize) * quality;
                        if s > a.best[wi] {
                            a.best[wi] = s;
                        }
                        a.mask |= 1 << wi;
                        let st = &mut stamp[e as usize * FIELDS + field as usize];
                        if covers && *st != wi as u8 + 1 {
                            *st = wi as u8 + 1;
                            a.cover[field as usize] += 1;
                        }
                    }
                };
                let exact = self.lookup.get(w.as_str()).copied();
                if let Some(t) = exact {
                    visit(t, EXACT, true, &mut acc, &mut touched);
                }
                if w.len() >= 2 || last {
                    let from = self.vocab.partition_point(|t| t.as_str() < w.as_str());
                    for t in from..self.vocab.len() {
                        let tok = &self.vocab[t];
                        if !tok.starts_with(w.as_str()) {
                            break;
                        }
                        if tok.len() == w.len() {
                            continue;
                        }
                        // A longer completion is a weaker match.
                        let q = 0.5 + 0.2 * w.len() as f32 / tok.len() as f32;
                        visit(t as u32, q, last, &mut acc, &mut touched);
                    }
                }
                let wc: Vec<char> = w.chars().collect();
                if exact.is_none() && wc.len() >= 4 {
                    let max = if wc.len() >= 8 { 2 } else { 1 };
                    let wm = char_mask(&wc);
                    for len in wc.len().saturating_sub(max)..=wc.len() + max {
                        for &t in self.by_len.get(len).map(|v| v.as_slice()).unwrap_or(&[]) {
                            if (wm ^ self.masks[t as usize]).count_ones() as usize > 2 * max {
                                continue;
                            }
                            if let Some(d) = edit_distance(&wc, &self.chars[t as usize], max) {
                                if d > 0 {
                                    visit(t, if d == 1 { FUZZY1 } else { FUZZY2 }, false, &mut acc, &mut touched);
                                }
                            }
                        }
                    }
                }
            }
        }
        // Rank cheap (matched, score, entity) triples; only the page becomes Hits.
        let mut ranked: [Vec<(u8, f32, u32)>; 4] = Default::default();
        for &e in &touched {
            let ent = &self.entities[e as usize];
            let a = &acc[e as usize];
            let idx = ent.idx as usize;
            let slot = match ent.kind {
                HitKind::Song if filtered && !passes[idx] => continue,
                HitKind::Song => 0,
                HitKind::Artist | HitKind::Album => {
                    let (g, slot) = if ent.kind == HitKind::Artist { (&self.artists[idx], 1) } else { (&self.albums[idx], 2) };
                    if filtered && !g.songs.iter().any(|&s| passes[s as usize]) {
                        continue;
                    }
                    slot
                }
                HitKind::Playlist => {
                    let p = &self.playlists[idx].0;
                    if filtered || !(p.public || user == Some(p.owner.as_str())) {
                        continue;
                    }
                    3
                }
            };
            let mut score: f32 = a.best.iter().sum();
            for f in 0..FIELDS {
                if ent.field_len[f] > 0 && a.cover[f] >= ent.field_len[f] {
                    score += weight(ent.kind, f) * 0.5;
                }
            }
            score += PLAYS_WEIGHT * (1.0 + ent.plays as f32).ln();
            ranked[slot].push((a.mask.count_ones() as u8, score, e));
        }
        let rank = |a: &(u8, f32, u32), b: &(u8, f32, u32)| {
            b.0.cmp(&a.0).then(b.1.total_cmp(&a.1)).then_with(|| self.name_of(a.2).cmp(self.name_of(b.2))).then(a.2.cmp(&b.2))
        };
        for r in ranked.iter_mut() {
            r.sort_unstable_by(rank);
        }
        let top = ranked.iter().filter_map(|r| r.first()).min_by(|a, b| rank(a, b)).map(|r| self.hit(r));
        let page = |r: &[(u8, f32, u32)]| r.iter().skip(offset).take(limit).map(|r| self.hit(r)).collect::<Vec<_>>();
        Results {
            top,
            songs: page(&ranked[0]),
            artists: page(&ranked[1]),
            albums: page(&ranked[2]),
            playlists: page(&ranked[3]),
            total_songs: ranked[0].len(),
            total_artists: ranked[1].len(),
            total_albums: ranked[2].len(),
            total_playlists: ranked[3].len(),
        }
    }

    fn name_of(&self, e: u32) -> &str {
        let ent = &self.entities[e as usize];
        let i = ent.idx as usize;
        match ent.kind {
            HitKind::Song => &self.songs[i].doc.title,
            HitKind::Artist => &self.artists[i].name,
            HitKind::Album => &self.albums[i].name,
            HitKind::Playlist => &self.playlists[i].0.name,
        }
    }

    fn hit(&self, &(matched, score, e): &(u8, f32, u32)) -> Hit {
        let ent = &self.entities[e as usize];
        let index = ent.idx as usize;
        let (id, name, subtitle) = match ent.kind {
            HitKind::Song => {
                let d = &self.songs[index].doc;
                (&d.id, &d.title, &d.artist)
            }
            HitKind::Artist | HitKind::Album => {
                let g = if ent.kind == HitKind::Artist { &self.artists[index] } else { &self.albums[index] };
                (&g.id, &g.name, &g.subtitle)
            }
            HitKind::Playlist => {
                let p = &self.playlists[index].0;
                (&p.id, &p.name, &p.owner)
            }
        };
        Hit { kind: ent.kind, id: id.clone(), name: name.clone(), subtitle: subtitle.clone(), index, matched, score }
    }
}

#[cfg(test)]
pub(crate) mod fixture {
    use super::SongDoc;

    /// A generated library of `n` songs (deterministic).
    pub fn songs(n: usize) -> Vec<SongDoc> {
        const SYL: [&str; 24] = ["ka", "lo", "mi", "ra", "ven", "tor", "sa", "bel", "dri", "mo", "nex", "qua", "zi", "fen", "gor", "hul", "jas", "pri", "sto", "wex", "yul", "bro", "cla", "dum"];
        const WORDS: [&str; 40] = [
            "love", "night", "dream", "fire", "heart", "rain", "summer", "light", "dance", "river", "golden", "shadow", "stars", "city",
            "blue", "wild", "electric", "ocean", "falling", "forever", "midnight", "storm", "silver", "highway", "paradise", "echo",
            "thunder", "winter", "sunrise", "garden", "machine", "velvet", "crystal", "frozen", "neon", "secret", "broken", "morning",
            "lonely", "rocket",
        ];
        const GENRES: [&str; 8] = ["Rock", "Pop", "Electronic", "Jazz", "Hip-Hop", "Classical", "Folk", "Metal"];
        let mut seed = 0x9e37_79b9_7f4a_7c15u64;
        let mut r = move || {
            seed ^= seed << 13;
            seed ^= seed >> 7;
            seed ^= seed << 17;
            seed
        };
        let artists: Vec<String> = (0..800)
            .map(|_| {
                let w = 1 + r() % 2;
                (0..w)
                    .map(|_| {
                        let s: String = (0..2 + r() % 2).map(|_| SYL[(r() % 24) as usize]).collect();
                        let mut c = s.chars();
                        c.next().unwrap().to_uppercase().chain(c).collect::<String>()
                    })
                    .collect::<Vec<_>>()
                    .join(" ")
            })
            .collect();
        (0..n)
            .map(|i| {
                let a = (r() % 800) as usize;
                let words = 1 + r() % 3;
                // Half common words, half made-up ones, so the vocabulary is
                // as big as a real library's (about one new token a song).
                let title: Vec<String> = (0..words)
                    .map(|_| if r() % 2 == 0 { WORDS[(r() % 40) as usize].to_string() } else { (0..2 + r() % 3).map(|_| SYL[(r() % 24) as usize]).collect() })
                    .collect();
                SongDoc {
                    id: format!("s{i}"),
                    title: title.join(" "),
                    artist: artists[a].clone(),
                    album: format!("{} {}", WORDS[(a * 7 + i % 3) % 40], WORDS[(a * 3 + 1) % 40]),
                    album_artist: String::new(),
                    genre: GENRES[a % 8].into(),
                    year: Some(1960 + (r() % 60) as u32),
                    plays: r() % 100,
                }
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn song(id: &str, title: &str, artist: &str, album: &str, genre: &str, year: u32, plays: u64) -> SongDoc {
        SongDoc {
            id: id.into(),
            title: title.into(),
            artist: artist.into(),
            album: album.into(),
            album_artist: String::new(),
            genre: genre.into(),
            year: Some(year),
            plays,
        }
    }

    fn library() -> (Vec<SongDoc>, Vec<PlaylistDoc>) {
        let songs = vec![
            song("1", "One More Time", "Daft Punk", "Discovery", "Electronic", 2001, 50),
            song("2", "Digital Love", "Daft Punk", "Discovery", "Electronic", 2001, 10),
            song("3", "Around the World", "Daft Punk", "Homework", "Electronic", 1997, 30),
            song("4", "Punk Rock Song", "Bad Religion", "The Gray Race", "Punk", 1996, 5),
            song("5", "Hey Jude", "The Beatles", "Hey Jude", "Rock", 1968, 99),
            song("6", "Jóga", "Björk", "Homogenic", "Electronic", 1997, 7),
            song("7", "Rock & Roll", "Led Zeppelin", "Led Zeppelin IV", "Rock", 1971, 20),
            song("8", "Straße", "Die Ärzte", "Geräusch", "Punk", 2003, 1),
            song("9", "Smells Like Teen Spirit", "Nirvana", "Nevermind", "Grunge", 1991, 80),
            song("10", "Come As You Are", "Nirvana", "Nevermind", "Grunge", 1991, 60),
            song("11", "Wonderwall", "Oasis", "(What's the Story) Morning Glory?", "Britpop", 1995, 40),
        ];
        let playlists = vec![
            PlaylistDoc { id: "p1".into(), name: "Road Trip".into(), owner: "alice".into(), public: true },
            PlaylistDoc { id: "p2".into(), name: "Secret Road Mix".into(), owner: "bob".into(), public: false },
        ];
        (songs, playlists)
    }

    #[test]
    fn normalisation() {
        assert_eq!(normalize("Björk & Æsir — Straße, Don't R.E.M.!"), vec!["bjork", "and", "aesir", "strasse", "dont", "rem"]);
        assert_eq!(normalize("Øresund Łódź Ĳssel œuvre ſ"), vec!["oresund", "lodz", "ijssel", "oeuvre", "s"]);
        assert_eq!(normalize("AC/DC"), vec!["ac", "dc"]);
        assert_eq!(name_tokens("The Beatles"), vec!["beatles"]);
        assert_eq!(name_tokens("A Day in the Life"), vec!["day", "in", "the", "life"]);
        assert_eq!(name_tokens("The"), vec!["the"]);
        assert_eq!(artist_id("The Beatles"), artist_id("beatles"));
        assert_ne!(album_id("A", "X"), album_id("B", "X"));
        let c = |s: &str| s.chars().collect::<Vec<_>>();
        assert_eq!(edit_distance(&c("beatles"), &c("beatels"), 1), Some(1), "a swap is one edit");
        assert_eq!(edit_distance(&c("nirvana"), &c("nirvanna"), 1), Some(1));
        assert_eq!(edit_distance(&c("abcd"), &c("wxyz"), 2), None);
    }

    #[test]
    fn prefix_search_as_you_type_finds_the_artist_first() {
        let (s, p) = library();
        let ix = Index::build(&s, &p);
        let r = ix.search("daft pu", None, 10, 0);
        let top = r.top.unwrap();
        assert_eq!((top.kind, top.name.as_str()), (HitKind::Artist, "Daft Punk"));
        let both: Vec<&Hit> = r.songs.iter().filter(|h| h.matched == 2).collect();
        assert_eq!(both.len(), 3);
        assert_eq!(r.songs[0].id, "1", "most played Daft Punk song first");
        assert!(both.iter().all(|h| h.subtitle == "Daft Punk"), "{:?}", r.songs);
        assert!(r.songs[3..].iter().all(|h| h.matched == 1), "songs matching only `pu` come after");
        let typing = ix.search("d", None, 10, 0);
        assert!(typing.artists.iter().any(|h| h.name == "Daft Punk" || h.name == "Die Ärzte"));
    }

    #[test]
    fn title_beats_artist_and_all_words_beat_some() {
        let (s, p) = library();
        let ix = Index::build(&s, &p);
        let r = ix.search("punk", None, 10, 0);
        assert_eq!(r.songs[0].id, "4", "title match before the artist match: {:?}", r.songs);
        let r = ix.search("one more time", None, 10, 0);
        assert_eq!(r.top.unwrap().id, "1");
        let r = ix.search("hey jude", None, 10, 0);
        assert_eq!(r.songs[0].id, "5");
        assert_eq!(r.albums[0].name, "Hey Jude");
        let r = ix.search("nevermind spirit", None, 10, 0);
        assert_eq!(r.songs[0].id, "9", "both words beat one: {:?}", r.songs);
    }

    #[test]
    fn accents_articles_ampersand_and_typos() {
        let (s, p) = library();
        let ix = Index::build(&s, &p);
        assert_eq!(ix.search("joga bjork", None, 5, 0).songs[0].id, "6");
        assert_eq!(ix.search("strasse", None, 5, 0).songs[0].id, "8");
        assert_eq!(ix.search("arzte", None, 5, 0).top.unwrap().name, "Die Ärzte");
        assert_eq!(ix.search("the beatles", None, 5, 0).top.unwrap().name, "The Beatles");
        assert_eq!(ix.search("beatles", None, 5, 0).top.unwrap().name, "The Beatles");
        assert_eq!(ix.search("rock and roll", None, 5, 0).songs[0].id, "7");
        assert_eq!(ix.search("rock & roll", None, 5, 0).songs[0].id, "7");
        assert_eq!(ix.search("nirvanna", None, 5, 0).top.unwrap().name, "Nirvana", "one typo");
        assert_eq!(ix.search("wonderwal", None, 5, 0).songs[0].id, "11", "prefix");
        assert_eq!(ix.search("wondrewall", None, 5, 0).songs[0].id, "11", "a swap");
        assert_eq!(ix.search("discoverry", None, 5, 0).albums[0].name, "Discovery");
        assert_eq!(ix.search("homewrk", None, 5, 0).albums[0].name, "Homework");
        assert!(ix.search("nrvn", None, 5, 0).top.is_none(), "too far");
        assert!(ix.search("jde", None, 5, 0).top.is_none(), "short words need an exact or prefix match");
        assert_eq!(ix.search("whats the story", None, 5, 0).songs[0].id, "11");
    }

    #[test]
    fn filters() {
        let (s, p) = library();
        let ix = Index::build(&s, &p);
        let r = ix.search("artist:\"daft punk\" year:1990-1999", None, 10, 0);
        assert_eq!(r.songs.iter().map(|h| h.id.as_str()).collect::<Vec<_>>(), vec!["3"]);
        assert!(r.playlists.is_empty());
        let r = ix.search("genre:grunge", None, 10, 0);
        assert_eq!(r.total_songs, 2);
        assert_eq!(r.songs[0].id, "9", "most played first");
        assert_eq!(r.artists[0].name, "Nirvana");
        let r = ix.search("love album:discovery", None, 10, 0);
        assert_eq!(r.songs[0].id, "2");
        assert_eq!(ix.search("year:2001", None, 10, 0).total_songs, 2);
        assert_eq!(ix.search("Year:1971", None, 10, 0).songs[0].id, "7");
        assert_eq!(ix.search("year:abc", None, 10, 0).total_songs, 0);
        assert_eq!(ix.search("artist:nirv", None, 10, 0).total_songs, 2, "prefix in a filter");
    }

    #[test]
    fn private_playlists_only_for_their_owner_and_paging() {
        let (s, p) = library();
        let ix = Index::build(&s, &p);
        assert_eq!(ix.search("road", None, 10, 0).playlists.len(), 1);
        assert_eq!(ix.search("road", Some("alice"), 10, 0).playlists.len(), 1);
        let bob = ix.search("road", Some("bob"), 10, 0);
        assert_eq!(bob.total_playlists, 2);
        assert!(ix.search("secret", None, 10, 0).top.is_none());
        let all = ix.search("nirvana", None, 10, 0);
        let page = ix.search("nirvana", None, 1, 1);
        assert_eq!(page.songs, all.songs[1..2].to_vec());
        assert_eq!(page.total_songs, all.total_songs);
        assert_eq!(ix.search("", None, 10, 0), Results::default());
    }

    #[test]
    fn play_counts_break_ties() {
        let (mut s, p) = library();
        s.push(song("12", "Come As You Are", "Cover Band", "", "", 2010, 0));
        let mut ix = Index::build(&s, &p);
        assert_eq!(ix.search("come as you are", None, 5, 0).songs[0].id, "10");
        ix.set_plays("12", 100_000);
        assert_eq!(ix.search("come as you are", None, 5, 0).songs[0].id, "12");
    }

    #[test]
    fn ten_thousand_songs_benchmark() {
        let songs = fixture::songs(10_000);
        let t0 = std::time::Instant::now();
        let ix = Index::build(&songs, &[]);
        let build = t0.elapsed();
        let queries = ["love", "midnight sto", "electrik ocean", "golden river dance", "sh", "artist:ka genre:rock", "paradyse", "neon year:1990-1999", "s", "crystal frozen morning"];
        let mut worst = std::time::Duration::ZERO;
        let t1 = std::time::Instant::now();
        let rounds = 5;
        for _ in 0..rounds {
            for q in queries {
                let t = std::time::Instant::now();
                let r = ix.search(q, None, 20, 0);
                let e = t.elapsed();
                worst = worst.max(e);
                if std::env::var("ECM_MUSIC_BENCH").is_ok() {
                    println!("  {q:?}: {e:?}, {} songs", r.total_songs);
                }
                assert!(r.top.is_some(), "{q}");
            }
        }
        let avg = t1.elapsed() / (rounds * queries.len() as u32);
        println!("search over 10,000 songs: build {build:?}, average query {avg:?}, worst {worst:?} ({} tokens)", ix.vocab.len());
        // Debug builds are several times slower than the release WASM daemon;
        // `cargo test --release` gives the real number (well under 2 ms).
        let limit = if cfg!(debug_assertions) { 60 } else { 2 };
        assert!(avg.as_millis() < limit, "average {avg:?}");
    }
}
