//! Структуры ответов API Яндекс Музыки и их преобразование в общие модели.
//!
//! API непоследовательно отдаёт идентификаторы то числами, то строками,
//! поэтому все id читаются через [`de_id`] / [`de_opt_id`].

use serde::{Deserialize, Deserializer};

use crate::model::{
    Account, AlbumRef, AlbumSummary, Artist, ArtistPage, ArtistSummary, Playlist, PlaylistKey,
    SearchResults, Source, Track, TrackKey,
};

#[derive(Deserialize)]
#[serde(untagged)]
enum IdRepr {
    Str(String),
    Num(u64),
}

impl From<IdRepr> for String {
    fn from(v: IdRepr) -> Self {
        match v {
            IdRepr::Str(s) => s,
            IdRepr::Num(n) => n.to_string(),
        }
    }
}

pub(crate) fn de_id<'de, D: Deserializer<'de>>(d: D) -> Result<String, D::Error> {
    IdRepr::deserialize(d).map(String::from)
}

pub(crate) fn de_opt_id<'de, D: Deserializer<'de>>(d: D) -> Result<Option<String>, D::Error> {
    Option::<IdRepr>::deserialize(d).map(|o| o.map(String::from))
}

/// Обёртка всех ответов: `{"invocationInfo": …, "result": …}` или `{"error": …}`.
#[derive(Deserialize)]
pub(crate) struct Envelope<T> {
    pub result: Option<T>,
    pub error: Option<ApiErrorBody>,
}

#[derive(Deserialize)]
pub(crate) struct ApiErrorBody {
    pub name: Option<String>,
    pub message: Option<String>,
}

// ---- аккаунт ----

#[derive(Deserialize)]
pub(crate) struct AccountStatus {
    pub account: AccountInfo,
    pub plus: Option<PlusInfo>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct AccountInfo {
    #[serde(default, deserialize_with = "de_opt_id")]
    pub uid: Option<String>,
    pub login: Option<String>,
    pub display_name: Option<String>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct PlusInfo {
    #[serde(default)]
    pub has_plus: bool,
}

impl AccountStatus {
    pub fn into_account(self) -> Option<Account> {
        Some(Account {
            uid: self.account.uid?,
            login: self.account.login,
            display_name: self.account.display_name,
            has_plus: self.plus.is_some_and(|p| p.has_plus),
        })
    }
}

// ---- треки ----

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct YTrack {
    #[serde(deserialize_with = "de_id")]
    pub id: String,
    pub title: Option<String>,
    pub version: Option<String>,
    #[serde(default)]
    pub artists: Vec<YArtist>,
    #[serde(default)]
    pub albums: Vec<YAlbum>,
    pub duration_ms: Option<u64>,
    pub available: Option<bool>,
    pub cover_uri: Option<String>,
}

#[derive(Deserialize)]
pub(crate) struct YArtist {
    #[serde(default, deserialize_with = "de_opt_id")]
    pub id: Option<String>,
    pub name: Option<String>,
}

#[derive(Deserialize)]
pub(crate) struct YAlbum {
    #[serde(default, deserialize_with = "de_opt_id")]
    pub id: Option<String>,
    pub title: Option<String>,
    pub year: Option<u32>,
}

/// `avatars.yandex.net/get-music-content/…/%%` → `https://…/400x400`.
pub(crate) fn cover_url(uri: &str, size: &str) -> String {
    format!("https://{}", uri.replace("%%", size))
}

impl From<YTrack> for Track {
    fn from(t: YTrack) -> Self {
        let album = t.albums.into_iter().next().map(|a| AlbumRef {
            id: a.id,
            title: a.title.unwrap_or_default(),
            year: a.year,
        });
        Track {
            key: TrackKey {
                source: Source::Yandex,
                id: t.id,
            },
            title: t.title.unwrap_or_default(),
            version: t.version,
            artists: t
                .artists
                .into_iter()
                .filter_map(|a| {
                    Some(Artist {
                        id: a.id,
                        name: a.name?,
                    })
                })
                .collect(),
            album,
            duration_ms: t.duration_ms,
            available: t.available.unwrap_or(true),
            cover_url: t.cover_uri.as_deref().map(|u| cover_url(u, "400x400")),
            isrc: None,
        }
    }
}

// ---- лайки ----

#[derive(Deserialize)]
pub(crate) struct LikesResult {
    pub library: LikesLibrary,
}

#[derive(Deserialize)]
pub(crate) struct LikesLibrary {
    #[serde(default)]
    pub tracks: Vec<TrackShort>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct TrackShort {
    #[serde(deserialize_with = "de_id")]
    pub id: String,
    #[serde(default, deserialize_with = "de_opt_id")]
    pub album_id: Option<String>,
}

impl TrackShort {
    /// Формат, который принимает `/tracks`: `trackId:albumId`.
    pub fn full_id(&self) -> String {
        match &self.album_id {
            Some(album) => format!("{}:{album}", self.id),
            None => self.id.clone(),
        }
    }
}

// ---- плейлисты ----

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct YPlaylist {
    #[serde(deserialize_with = "de_id")]
    pub kind: String,
    pub title: Option<String>,
    pub track_count: Option<u32>,
    #[serde(default)]
    pub tracks: Vec<YPlaylistItem>,
    pub cover: Option<YCover>,
    pub og_image: Option<String>,
    pub owner: Option<YOwner>,
    #[serde(default, deserialize_with = "de_opt_id")]
    pub uid: Option<String>,
}

#[derive(Deserialize)]
pub(crate) struct YOwner {
    #[serde(default, deserialize_with = "de_opt_id")]
    pub uid: Option<String>,
}

/// Обложка плейлиста: своя картинка (`uri`) или мозаика из обложек треков.
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct YCover {
    pub uri: Option<String>,
    #[serde(default)]
    pub items_uri: Vec<String>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct YPlaylistItem {
    #[serde(default, deserialize_with = "de_opt_id")]
    pub id: Option<String>,
    #[serde(default, deserialize_with = "de_opt_id")]
    pub album_id: Option<String>,
    pub track: Option<YTrack>,
}

impl YPlaylist {
    pub fn summary(&self) -> Playlist {
        Playlist {
            key: PlaylistKey {
                source: Source::Yandex,
                id: self.kind.clone(),
            },
            title: self.title.clone().unwrap_or_default(),
            track_count: self.track_count,
            cover_url: self
                .cover
                .as_ref()
                .and_then(|c| c.uri.clone().or_else(|| c.items_uri.first().cloned()))
                .or_else(|| self.og_image.clone())
                .map(|u| cover_url(&u, "400x400")),
            owner_uid: self
                .owner
                .as_ref()
                .and_then(|o| o.uid.clone())
                .or_else(|| self.uid.clone()),
        }
    }
}

// ---- альбомы ----

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct YAlbumInfo {
    #[serde(deserialize_with = "de_id")]
    pub id: String,
    pub title: Option<String>,
    pub version: Option<String>,
    pub year: Option<u32>,
    pub cover_uri: Option<String>,
    pub track_count: Option<u32>,
    #[serde(default)]
    pub artists: Vec<YArtist>,
    /// Диски альбома (только в `/albums/{id}/with-tracks`).
    #[serde(default)]
    pub volumes: Vec<Vec<YTrack>>,
}

impl YAlbumInfo {
    pub fn summary(&self) -> AlbumSummary {
        let title = self.title.clone().unwrap_or_default();
        AlbumSummary {
            id: self.id.clone(),
            title: match &self.version {
                Some(v) if !v.is_empty() => format!("{title} ({v})"),
                _ => title,
            },
            artists: self
                .artists
                .iter()
                .filter_map(|a| {
                    Some(Artist {
                        id: a.id.clone(),
                        name: a.name.clone()?,
                    })
                })
                .collect(),
            year: self.year,
            cover_url: self.cover_uri.as_deref().map(|u| cover_url(u, "400x400")),
            track_count: self.track_count,
        }
    }
}

#[derive(Deserialize)]
pub(crate) struct YLikedAlbum {
    pub album: Option<YAlbumInfo>,
}

// ---- поиск ----

#[derive(Deserialize)]
pub(crate) struct SearchResult {
    pub tracks: Option<Block<YTrack>>,
}

#[derive(Deserialize)]
pub(crate) struct Block<T> {
    #[serde(default = "Vec::new")]
    pub results: Vec<T>,
}

#[derive(Deserialize)]
pub(crate) struct YBest {
    #[serde(rename = "type")]
    pub kind: Option<String>,
}

/// Ответ `/search?type=all`.
#[derive(Deserialize)]
pub(crate) struct YSearchAll {
    pub best: Option<YBest>,
    pub artists: Option<Block<YArtistFull>>,
    pub albums: Option<Block<YAlbumInfo>>,
    pub playlists: Option<Block<YPlaylist>>,
    pub tracks: Option<Block<YTrack>>,
}

impl YSearchAll {
    pub fn into_results(self) -> SearchResults {
        SearchResults {
            best: self.best.and_then(|b| b.kind),
            artists: self
                .artists
                .map(|b| b.results.iter().map(YArtistFull::summary).collect())
                .unwrap_or_default(),
            albums: self
                .albums
                .map(|b| b.results.iter().map(YAlbumInfo::summary).collect())
                .unwrap_or_default(),
            playlists: self
                .playlists
                .map(|b| b.results.iter().map(YPlaylist::summary).collect())
                .unwrap_or_default(),
            tracks: self
                .tracks
                .map(|b| b.results.into_iter().map(Track::from).collect())
                .unwrap_or_default(),
        }
    }
}

// ---- исполнители ----

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct YArtistFull {
    #[serde(deserialize_with = "de_id")]
    pub id: String,
    pub name: Option<String>,
    pub cover: Option<YCover>,
    pub og_image: Option<String>,
}

impl YArtistFull {
    pub fn summary(&self) -> ArtistSummary {
        ArtistSummary {
            id: self.id.clone(),
            name: self.name.clone().unwrap_or_default(),
            cover_url: self
                .cover
                .as_ref()
                .and_then(|c| c.uri.clone())
                .or_else(|| self.og_image.clone())
                .map(|u| cover_url(&u, "400x400")),
        }
    }
}

/// Ответ `/artists/{id}/brief-info`.
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct YArtistBrief {
    pub artist: YArtistFull,
    #[serde(default)]
    pub albums: Vec<YAlbumInfo>,
    #[serde(default)]
    pub also_albums: Vec<YAlbumInfo>,
    #[serde(default)]
    pub popular_tracks: Vec<YTrack>,
}

impl YArtistBrief {
    pub fn into_page(self) -> ArtistPage {
        ArtistPage {
            artist: self.artist.summary(),
            albums: self.albums.iter().map(YAlbumInfo::summary).collect(),
            also_albums: self.also_albums.iter().map(YAlbumInfo::summary).collect(),
            popular_tracks: self.popular_tracks.into_iter().map(Track::from).collect(),
        }
    }
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct YPager {
    #[serde(default)]
    pub total: u32,
}

/// Ответ `/artists/{id}/tracks` (постранично).
#[derive(Deserialize)]
pub(crate) struct YArtistTracks {
    #[serde(default)]
    pub tracks: Vec<YTrack>,
    pub pager: Option<YPager>,
}

// ---- загрузка ----

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct DownloadInfo {
    pub codec: String,
    pub bitrate_in_kbps: Option<u32>,
    pub download_info_url: String,
    #[serde(default)]
    pub preview: bool,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn account_status_numeric_uid() {
        let json = r#"{"result":{"account":{"uid":12345,"login":"user","displayName":"User"},"plus":{"hasPlus":true}}}"#;
        let env: Envelope<AccountStatus> = serde_json::from_str(json).unwrap();
        let acc = env.result.unwrap().into_account().unwrap();
        assert_eq!(acc.uid, "12345");
        assert!(acc.has_plus);
    }

    #[test]
    fn account_without_plus_block() {
        let json = r#"{"account":{"uid":"1"}}"#;
        let st: AccountStatus = serde_json::from_str(json).unwrap();
        assert!(!st.into_account().unwrap().has_plus);
    }

    #[test]
    fn track_conversion() {
        let json = r#"{
            "id": "42", "title": "Song", "version": "Live",
            "artists": [{"id": 7, "name": "Band"}, {"various": true}],
            "albums": [{"id": 99, "title": "Album", "year": 2001}],
            "durationMs": 180000, "available": false,
            "coverUri": "avatars.yandex.net/get-music-content/1/2/%%"
        }"#;
        let t: Track = serde_json::from_str::<YTrack>(json).unwrap().into();
        assert_eq!(t.key.id, "42");
        assert_eq!(t.full_title(), "Song (Live)");
        assert_eq!(t.artist_line(), "Band");
        assert_eq!(t.album.as_ref().unwrap().id.as_deref(), Some("99"));
        assert!(!t.available);
        assert_eq!(
            t.cover_url.as_deref(),
            Some("https://avatars.yandex.net/get-music-content/1/2/400x400")
        );
    }

    #[test]
    fn likes_full_ids() {
        let json = r#"{"library":{"uid":1,"revision":5,"tracks":[
            {"id":"10","albumId":"20","timestamp":"2024-01-01T00:00:00+00:00"},
            {"id":11,"timestamp":"2024-01-01T00:00:00+00:00"}
        ]}}"#;
        let r: LikesResult = serde_json::from_str(json).unwrap();
        let ids: Vec<_> = r.library.tracks.iter().map(TrackShort::full_id).collect();
        assert_eq!(ids, ["10:20", "11"]);
    }

    #[test]
    fn playlist_cover_variants() {
        let pic: YPlaylist = serde_json::from_str(
            r#"{"kind":1,"title":"A","cover":{"type":"pic","uri":"avatars.yandex.net/p/1/%%?177"}}"#,
        )
        .unwrap();
        assert_eq!(
            pic.summary().cover_url.as_deref(),
            Some("https://avatars.yandex.net/p/1/400x400?177")
        );
        let mosaic: YPlaylist = serde_json::from_str(
            r#"{"kind":2,"cover":{"type":"mosaic","itemsUri":["avatars.yandex.net/a/%%","x/%%"]}}"#,
        )
        .unwrap();
        assert_eq!(
            mosaic.summary().cover_url.as_deref(),
            Some("https://avatars.yandex.net/a/400x400")
        );
        let og: YPlaylist =
            serde_json::from_str(r#"{"kind":3,"ogImage":"avatars.yandex.net/o/%%"}"#).unwrap();
        assert!(og.summary().cover_url.unwrap().ends_with("/o/400x400"));
        let none: YPlaylist = serde_json::from_str(r#"{"kind":4}"#).unwrap();
        assert!(none.summary().cover_url.is_none());
    }

    #[test]
    fn album_with_tracks() {
        let json = r#"{"id":41194184,"title":"Радио Пепел","year":2026,"trackCount":2,
            "coverUri":"avatars.yandex.net/c/%%","artists":[{"id":1,"name":"Группа"}],
            "volumes":[[{"id":"1","title":"Раз","albums":[{"id":41194184}]}],
                       [{"id":"2","title":"Два","albums":[{"id":41194184}]}]]}"#;
        let a: YAlbumInfo = serde_json::from_str(json).unwrap();
        let s = a.summary();
        assert_eq!(s.folder_name(), "Группа — Радио Пепел (2026)");
        assert_eq!(a.volumes.iter().flatten().count(), 2);
        let liked: Vec<YLikedAlbum> = serde_json::from_str(
            r#"[{"timestamp":"t","album":{"id":5,"title":"X"}},{"timestamp":"t"}]"#,
        )
        .unwrap();
        assert_eq!(liked.iter().filter(|x| x.album.is_some()).count(), 1);
    }

    #[test]
    fn search_all_sections() {
        let json = r#"{"best":{"type":"artist"},
            "artists":{"total":1,"results":[{"id":41075,"name":"КИНО",
                "cover":{"type":"from-artist-photos","uri":"avatars.yandex.net/a/%%"}}]},
            "albums":{"results":[{"id":5,"title":"Альбом","year":1988,"artists":[{"id":41075,"name":"КИНО"}]}]},
            "playlists":{"results":[{"kind":1,"title":"Лучшее","uid":457553308,
                "owner":{"uid":457553308,"login":"yamusic-bestsongs"}}]},
            "tracks":{"results":[{"id":"1","title":"Т"}]},
            "podcasts":{"results":[]}}"#;
        let r: YSearchAll = serde_json::from_str(json).unwrap();
        let r = r.into_results();
        assert_eq!(r.best.as_deref(), Some("artist"));
        assert_eq!(r.artists[0].name, "КИНО");
        assert_eq!(r.artists[0].cover_url.as_deref(), Some("https://avatars.yandex.net/a/400x400"));
        assert_eq!(r.albums[0].folder_name(), "КИНО — Альбом (1988)");
        assert_eq!(r.playlists[0].owner_uid.as_deref(), Some("457553308"));
        assert_eq!(r.tracks.len(), 1);

        let empty: YSearchAll = serde_json::from_str(r#"{"text":"x"}"#).unwrap();
        assert!(empty.into_results().artists.is_empty());
    }

    #[test]
    fn artist_brief() {
        let json = r#"{"artist":{"id":"41075","name":"КИНО"},
            "albums":[{"id":1,"title":"A"}],"alsoAlbums":[{"id":2,"title":"B"}],
            "popularTracks":[{"id":"9","title":"Т"}],"similarArtists":[]}"#;
        let b: YArtistBrief = serde_json::from_str(json).unwrap();
        let page = b.into_page();
        assert_eq!(page.artist.id, "41075");
        assert_eq!((page.albums.len(), page.also_albums.len(), page.popular_tracks.len()), (1, 1, 1));
        let t: YArtistTracks =
            serde_json::from_str(r#"{"pager":{"page":0,"perPage":100,"total":142},"tracks":[]}"#).unwrap();
        assert_eq!(t.pager.unwrap().total, 142);
    }

    #[test]
    fn download_info_list() {
        let json = r#"[
            {"codec":"mp3","bitrateInKbps":320,"downloadInfoUrl":"https://x/1","direct":false,"preview":false,"gain":false},
            {"codec":"aac","bitrateInKbps":64,"downloadInfoUrl":"https://x/2"}
        ]"#;
        let v: Vec<DownloadInfo> = serde_json::from_str(json).unwrap();
        assert_eq!(v[0].bitrate_in_kbps, Some(320));
        assert!(!v[1].preview);
    }
}
