//! The library index `musicd` keeps: one entry per MP3 in the library folder,
//! grouped into albums and artists, with the genres list, the New and A-Z
//! browse orders and the bit rate warnings `musicd status` shows.
//!
//! A scan is two steps so the daemon only reads files that changed:
//! [`Library::changed`] says which of the files it listed need reading
//! (new, or a different size or mtime than last time), the daemon reads those
//! and turns each into a [`Song`] with [`Song::from_file`] (making the cover
//! art thumbnails from the tags while it has them), and [`Library::apply`]
//! merges them with the unchanged entries. [`Library::save`] / [`Library::load`]
//! keep the index across restarts.
//!
//! Song ids come from the file's path relative to the library folder, so they
//! stay the same across rescans and restarts (playlists and likes refer to
//! them) as long as the file isn't moved.

use crate::search::{self, SongDoc};
use crate::text::{self, fnv64};
use crate::{id3, mp3};
use std::collections::HashMap;

/// Files above this bit rate get a warning (128 kbps is about 16 KB/s per listener).
pub const DEFAULT_MAX_KBPS: u32 = 192;

/// A file in the library folder as the daemon lists it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FileStat {
    /// Path relative to the library folder, with `/` separators.
    pub path: String,
    pub size: u64,
    /// Modification time (any monotonic unit the daemon likes; seconds).
    pub mtime: u64,
}

/// Whether a path looks like an MP3 (by extension).
pub fn is_mp3_path(path: &str) -> bool {
    path.rsplit('.').next().is_some_and(|e| e.eq_ignore_ascii_case("mp3")) && path.contains('.')
}

/// The stable id of the song at `path`.
pub fn song_id(path: &str) -> String {
    format!("s{:016x}", fnv64(path))
}

/// One song.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Song {
    pub id: String,
    pub path: String,
    pub size: u64,
    pub mtime: u64,
    pub title: String,
    /// Empty when the tags don't say.
    pub artist: String,
    pub album: String,
    pub album_artist: String,
    pub genre: String,
    pub track: Option<u32>,
    pub year: Option<u32>,
    pub duration_ms: u64,
    pub bitrate_kbps: u32,
    pub vbr: bool,
    pub sample_rate: u32,
    pub channels: u8,
    /// MIME type of the embedded cover picture, if there is one.
    pub art: Option<String>,
    pub artist_id: String,
    /// Empty when the song has no album.
    pub album_id: String,
}

impl Song {
    /// Read a song from the file's bytes. None when there is no MP3 audio.
    /// The tags come back too, for the cover picture.
    pub fn from_file(stat: &FileStat, bytes: &[u8]) -> Option<(Song, id3::Tags)> {
        let info = mp3::probe(bytes)?;
        let tags = id3::read(bytes);
        let stem = stat.path.rsplit('/').next().unwrap_or(&stat.path);
        let stem = stem.rsplit_once('.').map_or(stem, |(s, _)| s);
        let t = |v: &Option<String>| v.as_deref().map(str::trim).unwrap_or("").to_string();
        let mut song = Song {
            id: song_id(&stat.path),
            path: stat.path.clone(),
            size: stat.size,
            mtime: stat.mtime,
            title: tags.title.as_deref().map(str::trim).filter(|s| !s.is_empty()).map(str::to_string).unwrap_or_else(|| stem.replace('_', " ").trim().to_string()),
            artist: t(&tags.artist),
            album: t(&tags.album),
            album_artist: t(&tags.album_artist),
            genre: t(&tags.genre),
            track: tags.track,
            year: tags.year,
            duration_ms: info.duration_ms,
            bitrate_kbps: info.bitrate_kbps,
            vbr: info.vbr,
            sample_rate: info.sample_rate,
            channels: info.channels,
            art: tags.picture.as_ref().map(|p| p.mime.clone()),
            ..Song::default()
        };
        song.fill_ids();
        Some((song, tags))
    }

    /// The album artist if there is one, else the artist.
    pub fn album_artist_or_artist(&self) -> &str {
        if self.album_artist.is_empty() {
            &self.artist
        } else {
            &self.album_artist
        }
    }

    fn fill_ids(&mut self) {
        self.id = song_id(&self.path);
        self.artist_id = if self.artist.is_empty() { String::new() } else { search::artist_id(&self.artist) };
        self.album_id = if self.album.is_empty() { String::new() } else { search::album_id(self.album_artist_or_artist(), &self.album) };
    }
}

/// An album: songs sharing an album name and album artist.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Album {
    pub id: String,
    pub name: String,
    pub artist: String,
    pub artist_id: String,
    pub year: Option<u32>,
    /// The most common genre of its songs.
    pub genre: String,
    /// Song ids in track order.
    pub songs: Vec<String>,
    /// The song whose picture is the album's cover.
    pub art_song: Option<String>,
    pub duration_ms: u64,
    /// Newest file mtime (for the New order).
    pub added: u64,
}

/// An artist with their albums and songs.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Artist {
    pub id: String,
    pub name: String,
    pub albums: Vec<String>,
    pub songs: Vec<String>,
}

/// A file above the bit rate limit.
#[derive(Clone, Debug, PartialEq)]
pub struct Warning {
    pub song_id: String,
    pub path: String,
    pub kbps: u32,
}

/// Browse order.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum Sort {
    /// Newest files first.
    New,
    /// By title (or name), ignoring case, accents and a leading "the"/"a".
    Az,
}

impl Sort {
    pub fn parse(s: &str) -> Option<Sort> {
        match s {
            "new" => Some(Sort::New),
            "az" | "a-z" => Some(Sort::Az),
            _ => None,
        }
    }
}

/// What a scan changed.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ScanReport {
    pub added: usize,
    pub changed: usize,
    pub removed: usize,
    pub unchanged: usize,
    /// Listed files that were neither unchanged nor read successfully.
    pub skipped: usize,
}

/// The library index.
#[derive(Clone, Debug, Default)]
pub struct Library {
    pub max_kbps: u32,
    songs: Vec<Song>,
    by_id: HashMap<String, usize>,
    by_path: HashMap<String, usize>,
    albums: Vec<Album>,
    album_by_id: HashMap<String, usize>,
    artists: Vec<Artist>,
    artist_by_id: HashMap<String, usize>,
    genres: Vec<(String, usize)>,
    warnings: Vec<Warning>,
    order_new: Vec<usize>,
    order_az: Vec<usize>,
}

fn sort_key(s: &str) -> String {
    let t = search::name_tokens(s);
    if t.is_empty() {
        s.to_lowercase()
    } else {
        t.join(" ")
    }
}

impl Library {
    pub fn new(max_kbps: u32) -> Library {
        Library { max_kbps, ..Library::default() }
    }

    /// The files of `files` that need reading: new ones and ones whose size
    /// or mtime changed.
    pub fn changed<'a>(&self, files: &'a [FileStat]) -> Vec<&'a FileStat> {
        files
            .iter()
            .filter(|f| match self.by_path.get(&f.path) {
                Some(&i) => self.songs[i].size != f.size || self.songs[i].mtime != f.mtime,
                None => true,
            })
            .collect()
    }

    /// Merge a scan: `files` is everything now in the library folder and
    /// `scanned` the songs read for the files [`Library::changed`] returned.
    /// Entries for files no longer listed are dropped.
    pub fn apply(&mut self, files: &[FileStat], scanned: Vec<Song>) -> ScanReport {
        let mut report = ScanReport::default();
        let mut fresh: HashMap<String, Song> = scanned.into_iter().map(|s| (s.path.clone(), s)).collect();
        let mut old: HashMap<String, Song> = std::mem::take(&mut self.songs).into_iter().map(|s| (s.path.clone(), s)).collect();
        let mut songs = Vec::with_capacity(files.len());
        for f in files {
            if let Some(mut s) = fresh.remove(&f.path) {
                if old.remove(&f.path).is_some() {
                    report.changed += 1;
                } else {
                    report.added += 1;
                }
                s.fill_ids();
                songs.push(s);
            } else if let Some(s) = old.remove(&f.path).filter(|s| s.size == f.size && s.mtime == f.mtime) {
                report.unchanged += 1;
                songs.push(s);
            } else {
                report.skipped += 1;
            }
        }
        report.removed = old.len();
        self.songs = songs;
        self.regroup();
        report
    }

    fn regroup(&mut self) {
        self.songs.sort_by(|a, b| a.path.cmp(&b.path));
        self.songs.dedup_by(|a, b| a.path == b.path);
        self.by_id = self.songs.iter().enumerate().map(|(i, s)| (s.id.clone(), i)).collect();
        self.by_path = self.songs.iter().enumerate().map(|(i, s)| (s.path.clone(), i)).collect();
        self.albums.clear();
        self.album_by_id.clear();
        self.artists.clear();
        self.artist_by_id.clear();
        let mut genres: HashMap<String, (String, usize)> = HashMap::new();
        for s in self.songs.iter() {
            if !s.album_id.is_empty() {
                let a = *self.album_by_id.entry(s.album_id.clone()).or_insert_with(|| {
                    let by = s.album_artist_or_artist().to_string();
                    self.albums.push(Album {
                        id: s.album_id.clone(),
                        name: s.album.clone(),
                        artist_id: if by.is_empty() { String::new() } else { search::artist_id(&by) },
                        artist: by,
                        ..Album::default()
                    });
                    self.albums.len() - 1
                });
                let al = &mut self.albums[a];
                al.songs.push(s.id.clone());
                al.duration_ms += s.duration_ms;
                al.added = al.added.max(s.mtime);
                if al.year.is_none() {
                    al.year = s.year;
                }
            }
            if !s.artist_id.is_empty() {
                let a = *self.artist_by_id.entry(s.artist_id.clone()).or_insert_with(|| {
                    self.artists.push(Artist { id: s.artist_id.clone(), name: s.artist.clone(), ..Artist::default() });
                    self.artists.len() - 1
                });
                let ar = &mut self.artists[a];
                ar.songs.push(s.id.clone());
                if !s.album_id.is_empty() && !ar.albums.contains(&s.album_id) {
                    ar.albums.push(s.album_id.clone());
                }
            }
            if !s.genre.is_empty() {
                let e = genres.entry(sort_key(&s.genre)).or_insert_with(|| (s.genre.clone(), 0));
                e.1 += 1;
            }
        }
        let songs = &self.songs;
        let by_id = &self.by_id;
        let track_order = |ids: &mut Vec<String>| {
            ids.sort_by_key(|id| {
                let s = &songs[by_id[id]];
                (s.track.unwrap_or(u32::MAX), sort_key(&s.title), s.path.clone())
            })
        };
        for al in &mut self.albums {
            track_order(&mut al.songs);
            al.art_song = al.songs.iter().find(|id| songs[by_id[*id]].art.is_some()).cloned();
            let mut count: HashMap<&str, usize> = HashMap::new();
            for id in &al.songs {
                let g = songs[by_id[id]].genre.as_str();
                if !g.is_empty() {
                    *count.entry(g).or_default() += 1;
                }
            }
            al.genre = count.into_iter().max_by(|a, b| a.1.cmp(&b.1).then(b.0.cmp(a.0))).map(|g| g.0.to_string()).unwrap_or_default();
        }
        let album_key: HashMap<String, (Option<u32>, String)> = self.albums.iter().map(|a| (a.id.clone(), (a.year, sort_key(&a.name)))).collect();
        for ar in &mut self.artists {
            ar.songs.sort_by_key(|id| sort_key(&songs[by_id[id]].title));
            ar.albums.sort_by_key(|id| album_key.get(id).cloned().unwrap_or_default());
        }
        self.albums.sort_by_key(|a| (sort_key(&a.name), sort_key(&a.artist), a.id.clone()));
        self.album_by_id = self.albums.iter().enumerate().map(|(i, a)| (a.id.clone(), i)).collect();
        self.artists.sort_by_key(|a| (sort_key(&a.name), a.id.clone()));
        self.artist_by_id = self.artists.iter().enumerate().map(|(i, a)| (a.id.clone(), i)).collect();
        let mut g: Vec<(String, usize)> = genres.into_values().collect();
        g.sort_by_key(|(name, _)| sort_key(name));
        self.genres = g;
        self.warnings = self
            .songs
            .iter()
            .filter(|s| s.bitrate_kbps > self.max_kbps)
            .map(|s| Warning { song_id: s.id.clone(), path: s.path.clone(), kbps: s.bitrate_kbps })
            .collect();
        let keys: Vec<(String, String)> = self.songs.iter().map(|s| (sort_key(&s.title), sort_key(&s.artist))).collect();
        let mut az: Vec<usize> = (0..self.songs.len()).collect();
        az.sort_by(|&a, &b| keys[a].cmp(&keys[b]).then(a.cmp(&b)));
        let mut new: Vec<usize> = (0..self.songs.len()).collect();
        new.sort_by(|&a, &b| self.songs[b].mtime.cmp(&self.songs[a].mtime).then(keys[a].cmp(&keys[b])).then(a.cmp(&b)));
        self.order_az = az;
        self.order_new = new;
    }

    /// All songs, by path.
    pub fn songs(&self) -> &[Song] {
        &self.songs
    }

    pub fn len(&self) -> usize {
        self.songs.len()
    }

    pub fn is_empty(&self) -> bool {
        self.songs.is_empty()
    }

    pub fn song(&self, id: &str) -> Option<&Song> {
        self.by_id.get(id).map(|&i| &self.songs[i])
    }

    pub fn song_by_path(&self, path: &str) -> Option<&Song> {
        self.by_path.get(path).map(|&i| &self.songs[i])
    }

    /// Albums, A-Z.
    pub fn albums(&self) -> &[Album] {
        &self.albums
    }

    pub fn album(&self, id: &str) -> Option<&Album> {
        self.album_by_id.get(id).map(|&i| &self.albums[i])
    }

    /// Artists, A-Z.
    pub fn artists(&self) -> &[Artist] {
        &self.artists
    }

    pub fn artist(&self, id: &str) -> Option<&Artist> {
        self.artist_by_id.get(id).map(|&i| &self.artists[i])
    }

    /// Genres with their song counts, A-Z.
    pub fn genres(&self) -> &[(String, usize)] {
        &self.genres
    }

    /// Songs of a genre (case and accents ignored), A-Z.
    pub fn genre_songs(&self, genre: &str) -> Vec<&Song> {
        let key = sort_key(genre);
        self.order_az.iter().map(|&i| &self.songs[i]).filter(|s| sort_key(&s.genre) == key).collect()
    }

    /// Files above [`Library::max_kbps`].
    pub fn warnings(&self) -> &[Warning] {
        &self.warnings
    }

    /// A page of songs in `sort` order.
    pub fn browse(&self, sort: Sort, limit: usize, offset: usize) -> Vec<&Song> {
        let order = if sort == Sort::New { &self.order_new } else { &self.order_az };
        order.iter().skip(offset).take(limit).map(|&i| &self.songs[i]).collect()
    }

    /// A page of albums in `sort` order.
    pub fn browse_albums(&self, sort: Sort, limit: usize, offset: usize) -> Vec<&Album> {
        let mut v: Vec<&Album> = self.albums.iter().collect();
        if sort == Sort::New {
            v.sort_by(|a, b| b.added.cmp(&a.added));
        }
        v.into_iter().skip(offset).take(limit).collect()
    }

    /// The songs of an album in track order.
    pub fn album_songs(&self, id: &str) -> Vec<&Song> {
        self.album(id).map(|a| a.songs.iter().filter_map(|s| self.song(s)).collect()).unwrap_or_default()
    }

    /// The songs for [`search::Index::build`], with play counts from `plays`.
    pub fn search_docs(&self, plays: impl Fn(&str) -> u64) -> Vec<SongDoc> {
        self.songs
            .iter()
            .map(|s| SongDoc {
                id: s.id.clone(),
                title: s.title.clone(),
                artist: s.artist.clone(),
                album: s.album.clone(),
                album_artist: s.album_artist.clone(),
                genre: s.genre.clone(),
                year: s.year,
                plays: plays(&s.id),
            })
            .collect()
    }

    /// The index as text (one line per song).
    pub fn save(&self) -> String {
        let mut out = format!("ecm-music-library\t1\t{}\n", self.max_kbps);
        let opt = |v: Option<u32>| v.map(|n| n.to_string()).unwrap_or_default();
        for s in &self.songs {
            out.push_str(&text::line(&[
                "song",
                &s.path,
                &s.size.to_string(),
                &s.mtime.to_string(),
                &s.title,
                &s.artist,
                &s.album,
                &s.album_artist,
                &s.genre,
                &opt(s.track),
                &opt(s.year),
                &s.duration_ms.to_string(),
                &s.bitrate_kbps.to_string(),
                if s.vbr { "1" } else { "0" },
                &s.sample_rate.to_string(),
                &s.channels.to_string(),
                s.art.as_deref().unwrap_or(""),
            ]));
        }
        out
    }

    /// Read a saved index. `max_kbps` replaces the saved limit when given
    /// (the daemon's `--max-kbps` wins).
    pub fn load(text: &str, max_kbps: Option<u32>) -> Result<Library, String> {
        let mut lines = text.lines();
        let head = lines.next().ok_or("empty library index")?;
        let h = text::fields(head);
        if h.first().map(String::as_str) != Some("ecm-music-library") || h.get(1).map(String::as_str) != Some("1") {
            return Err("not an ecm-music library index".into());
        }
        let saved_max = h.get(2).and_then(|v| v.parse().ok()).unwrap_or(DEFAULT_MAX_KBPS);
        let mut lib = Library::new(max_kbps.unwrap_or(saved_max));
        for (n, l) in lines.enumerate() {
            if l.is_empty() {
                continue;
            }
            let f = text::fields(l);
            if f.len() < 17 || f[0] != "song" {
                return Err(format!("library index line {}: bad record", n + 2));
            }
            let num = |i: usize| f[i].parse::<u64>().map_err(|_| format!("library index line {}: bad number", n + 2));
            let opt = |i: usize| f[i].parse::<u32>().ok();
            let mut s = Song {
                path: f[1].clone(),
                size: num(2)?,
                mtime: num(3)?,
                title: f[4].clone(),
                artist: f[5].clone(),
                album: f[6].clone(),
                album_artist: f[7].clone(),
                genre: f[8].clone(),
                track: opt(9),
                year: opt(10),
                duration_ms: num(11)?,
                bitrate_kbps: num(12)? as u32,
                vbr: f[13] == "1",
                sample_rate: num(14)? as u32,
                channels: num(15)? as u8,
                art: (!f[16].is_empty()).then(|| f[16].clone()),
                ..Song::default()
            };
            s.fill_ids();
            lib.songs.push(s);
        }
        lib.regroup();
        Ok(lib)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn song(path: &str, title: &str, artist: &str, album: &str, track: u32, mtime: u64, kbps: u32) -> Song {
        let mut s = Song {
            path: path.into(),
            size: 1000,
            mtime,
            title: title.into(),
            artist: artist.into(),
            album: album.into(),
            genre: "Rock".into(),
            track: Some(track),
            year: Some(2000),
            duration_ms: 1000,
            bitrate_kbps: kbps,
            ..Song::default()
        };
        s.fill_ids();
        s
    }

    fn stat(s: &Song) -> FileStat {
        FileStat { path: s.path.clone(), size: s.size, mtime: s.mtime }
    }

    #[test]
    fn reads_the_fixtures() {
        let dir = format!("{}/tests/data", env!("CARGO_MANIFEST_DIR"));
        let read = |name: &str| {
            let bytes = std::fs::read(format!("{dir}/{name}")).unwrap();
            let st = FileStat { path: format!("test/{name}"), size: bytes.len() as u64, mtime: 5 };
            Song::from_file(&st, &bytes).unwrap()
        };
        let (cbr, tags) = read("tone_cbr.mp3");
        assert_eq!((cbr.title.as_str(), cbr.artist.as_str(), cbr.album.as_str(), cbr.album_artist.as_str()), ("Sine Tone", "Test Artist", "Test Album", "Various"));
        assert_eq!((cbr.track, cbr.year, cbr.genre.as_str()), (Some(3), Some(2001), "Electronic"));
        assert_eq!(cbr.art.as_deref(), Some("image/jpeg"));
        assert_eq!(tags.picture.unwrap().kind, id3::FRONT_COVER);
        assert_eq!(cbr.album_id, search::album_id("Various", "Test Album"));
        let (vbr, _) = read("tone_vbr.mp3");
        assert_eq!((vbr.title.as_str(), vbr.artist.as_str(), vbr.year, vbr.track), ("Café Ünïcode ß", "Björk", Some(1997), Some(7)));
        assert!(vbr.vbr);
        let (m2, tags) = read("tone_mpeg2.mp3");
        assert!(tags.v1 && tags.v2 == 0);
        assert_eq!((m2.title.as_str(), m2.artist.as_str(), m2.year, m2.track, m2.genre.as_str()), ("Old Tag", "V1 Artist", Some(1985), Some(5), "Rock"));
        assert_eq!((m2.sample_rate, m2.channels, m2.bitrate_kbps), (22050, 1, 32));
        assert!(Song::from_file(&FileStat { path: "x.mp3".into(), size: 3, mtime: 0 }, b"abc").is_none());
        let untitled = FileStat { path: "a/My_Song.mp3".into(), size: 0, mtime: 0 };
        let mut bytes = std::fs::read(format!("{dir}/tone_mpeg2.mp3")).unwrap();
        bytes.truncate(bytes.len() - 128);
        assert_eq!(Song::from_file(&untitled, &bytes).unwrap().0.title, "My Song", "title from the file name");
    }

    #[test]
    fn grouping_browse_warnings_and_incremental_scan() {
        let a = song("x/1.mp3", "Second", "The Band", "First Album", 2, 10, 128);
        let b = song("x/2.mp3", "First", "Band", "First Album", 1, 30, 320);
        let c = song("y/3.mp3", "A Zebra", "Other", "", 1, 20, 128);
        let files: Vec<FileStat> = [&a, &b, &c].iter().map(|s| stat(s)).collect();
        let mut lib = Library::new(DEFAULT_MAX_KBPS);
        assert_eq!(lib.changed(&files).len(), 3);
        let r = lib.apply(&files, vec![a.clone(), b.clone(), c.clone()]);
        assert_eq!(r, ScanReport { added: 3, ..ScanReport::default() });
        assert_eq!(lib.albums().len(), 1, "'The Band' and 'Band' are the same artist");
        let al = &lib.albums()[0];
        assert_eq!(al.songs, vec![b.id.clone(), a.id.clone()], "track order");
        assert_eq!(al.duration_ms, 2000);
        assert_eq!(lib.artists().len(), 2);
        assert_eq!(lib.artist(&a.artist_id).unwrap().songs.len(), 2);
        assert_eq!(lib.genres(), &[("Rock".to_string(), 3)]);
        assert_eq!(lib.warnings().len(), 1);
        assert_eq!(lib.warnings()[0].kbps, 320);
        let az: Vec<&str> = lib.browse(Sort::Az, 10, 0).iter().map(|s| s.title.as_str()).collect();
        assert_eq!(az, vec!["First", "Second", "A Zebra"], "a leading article is ignored");
        let new: Vec<&str> = lib.browse(Sort::New, 2, 0).iter().map(|s| s.title.as_str()).collect();
        assert_eq!(new, vec!["First", "A Zebra"]);
        assert_eq!(lib.browse(Sort::New, 2, 2).len(), 1);
        // Rescan: one unchanged, one changed, one removed, one new.
        let mut a2 = a.clone();
        a2.mtime = 40;
        a2.title = "Second (remaster)".into();
        let d = song("z/4.mp3", "New", "Other", "", 1, 50, 128);
        let files2 = vec![stat(&a2), stat(&b), stat(&d)];
        let todo: Vec<String> = lib.changed(&files2).iter().map(|f| f.path.clone()).collect();
        assert_eq!(todo, vec!["x/1.mp3", "z/4.mp3"]);
        let r = lib.apply(&files2, vec![a2.clone(), d.clone()]);
        assert_eq!(r, ScanReport { added: 1, changed: 1, removed: 1, unchanged: 1, skipped: 0 });
        assert_eq!(lib.song(&a.id).unwrap().title, "Second (remaster)", "same id after a rescan");
        assert!(lib.song(&c.id).is_none());
        assert_eq!(lib.search_docs(|_| 3).len(), 3);
    }

    #[test]
    fn save_and_load_round_trip() {
        let mut s = song("dir/tab\tname.mp3", "Tïtle\nwith newline", "Artist", "Album", 1, 1, 128);
        s.art = Some("image/png".into());
        s.vbr = true;
        let mut lib = Library::new(160);
        lib.apply(&[stat(&s)], vec![s.clone()]);
        let text = lib.save();
        let back = Library::load(&text, None).unwrap();
        assert_eq!(back.songs(), lib.songs());
        assert_eq!(back.max_kbps, 160);
        assert_eq!(back.albums(), lib.albums());
        assert!(back.changed(&[stat(&s)]).is_empty(), "nothing to rescan");
        assert_eq!(Library::load(&text, Some(256)).unwrap().max_kbps, 256);
        assert!(Library::load("garbage", None).is_err());
        assert!(Library::load("ecm-music-library\t1\t192\nsong\tx", None).is_err());
        assert!(is_mp3_path("a/B.MP3") && !is_mp3_path("mp3") && !is_mp3_path("a.ogg"));
    }
}
