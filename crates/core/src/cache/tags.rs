//! Запись тегов в файлы кэша: ID3v2 для mp3/aac, Vorbis-комментарии для flac.
//! После этого файл кэша — обычный тегированный трек, как собственные
//! mp3/flac пользователя.

use std::path::Path;

use lofty::config::WriteOptions;
use lofty::file::{AudioFile, TaggedFileExt};
use lofty::picture::{MimeType, Picture, PictureType};
use lofty::probe::Probe;
use lofty::tag::{Accessor, ItemKey, Tag};

use crate::model::TrackMeta;
use crate::{Error, Result};

fn tag_error(e: impl std::fmt::Display) -> Error {
    Error::Unexpected(format!("теги: {e}"))
}

/// Вшить метаданные и обложку (JPEG) в файл. Тип файла определяется по
/// содержимому, поэтому работает и для недокачанного `.part`.
pub fn write_tags(path: &Path, meta: &TrackMeta, cover_jpeg: Option<&[u8]>) -> Result<()> {
    let mut file = Probe::open(path)
        .map_err(tag_error)?
        .guess_file_type()
        .map_err(tag_error)?
        .read()
        .map_err(tag_error)?;

    if file.primary_tag().is_none() {
        file.insert_tag(Tag::new(file.primary_tag_type()));
    }
    let tag = file
        .primary_tag_mut()
        .ok_or_else(|| Error::Unexpected("теги: формат не поддерживает теги".into()))?;

    tag.set_title(meta.title.clone());
    if !meta.artists.is_empty() {
        tag.set_artist(meta.artists.join(", "));
    }
    if let Some(album) = &meta.album {
        tag.set_album(album.clone());
    }
    if let Some(year) = meta.year {
        tag.insert_text(ItemKey::Year, year.to_string());
    }
    // Откуда трек — чтобы потом сопоставлять файлы с каталогом.
    tag.insert_text(ItemKey::Comment, format!("moth-amp:{}:{}", meta.source, meta.id));
    if let Some(jpeg) = cover_jpeg {
        tag.remove_picture_type(PictureType::CoverFront);
        tag.push_picture(Picture::unchecked(jpeg.to_vec())
            .pic_type(PictureType::CoverFront)
            .mime_type(MimeType::Jpeg)
            .build());
    }

    file.save_to_path(path, WriteOptions::default())
        .map_err(tag_error)
}

/// Что удалось прочитать из собственного файла пользователя.
pub struct FileInfo {
    /// `mp3` или `flac`.
    pub codec: &'static str,
    pub title: Option<String>,
    pub artists: Vec<String>,
    pub album: Option<String>,
    pub year: Option<u32>,
    pub duration_ms: u64,
    /// Обложка из тегов и её MIME-тип.
    pub cover: Option<(Vec<u8>, String)>,
}

/// Прочитать теги и свойства mp3/flac. Другие форматы не принимаются.
pub fn read_file_info(path: &Path) -> Result<FileInfo> {
    use lofty::file::FileType;

    let file = Probe::open(path)
        .map_err(tag_error)?
        .guess_file_type()
        .map_err(tag_error)?
        .read()
        .map_err(tag_error)?;
    let codec = match file.file_type() {
        FileType::Mpeg => "mp3",
        FileType::Flac => "flac",
        other => {
            return Err(Error::Unexpected(format!(
                "формат {other:?} не поддерживается: только mp3 и flac"
            )))
        }
    };
    let duration_ms = u64::try_from(file.properties().duration().as_millis()).unwrap_or(0);
    let Some(tag) = file.primary_tag().or_else(|| file.first_tag()) else {
        return Ok(FileInfo {
            codec,
            title: None,
            artists: Vec::new(),
            album: None,
            year: None,
            duration_ms,
            cover: None,
        });
    };
    let artists = tag
        .artist()
        .map(|a| {
            a.split([';', '/'])
                .map(|s| s.trim().to_owned())
                .filter(|s| !s.is_empty())
                .collect()
        })
        .unwrap_or_default();
    let year = tag
        .get_string(ItemKey::Year)
        .or_else(|| tag.get_string(ItemKey::RecordingDate))
        .and_then(|s| s.get(..4))
        .and_then(|y| y.parse().ok());
    let cover = tag
        .pictures()
        .iter()
        .find(|p| p.pic_type() == PictureType::CoverFront)
        .or_else(|| tag.pictures().first())
        .map(|p| {
            let mime = p
                .mime_type()
                .map(|m| m.as_str().to_owned())
                .unwrap_or_else(|| "image/jpeg".to_owned());
            (p.data().to_vec(), mime)
        });
    Ok(FileInfo {
        codec,
        title: tag.title().map(|s| s.into_owned()),
        artists,
        album: tag.album().map(|s| s.into_owned()),
        year,
        duration_ms,
        cover,
    })
}

/// Прочитать основные теги (для проверки и для будущей локальной библиотеки).
pub fn read_title_artist(path: &Path) -> Result<(Option<String>, Option<String>, bool)> {
    let file = Probe::open(path)
        .map_err(tag_error)?
        .guess_file_type()
        .map_err(tag_error)?
        .read()
        .map_err(tag_error)?;
    let Some(tag) = file.primary_tag() else {
        return Ok((None, None, false));
    };
    let has_cover = tag
        .pictures()
        .iter()
        .any(|p| p.pic_type() == PictureType::CoverFront);
    Ok((
        tag.title().map(|s| s.into_owned()),
        tag.artist().map(|s| s.into_owned()),
        has_cover,
    ))
}

#[cfg(test)]
pub(crate) mod tests_support {
    /// Минимальный корректный mp3: кадры MPEG-1 Layer III, 128 кбит/с, 44,1 кГц.
    pub(crate) fn tiny_mp3() -> Vec<u8> {
        let frame_len = 144 * 128_000 / 44_100; // 417 байт
        let mut frame = vec![0u8; frame_len];
        frame[..4].copy_from_slice(&[0xFF, 0xFB, 0x90, 0x64]);
        frame.repeat(20)
    }
}

#[cfg(test)]
mod tests {
    use super::tests_support::tiny_mp3;
    use super::*;

    #[test]
    fn writes_and_reads_tags() {
        let dir = std::env::temp_dir().join(format!("moth-tags-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("1.part");
        std::fs::write(&path, tiny_mp3()).unwrap();

        let meta = TrackMeta {
            source: "yandex".into(),
            id: "1".into(),
            title: "Название".into(),
            artists: vec!["Исполнитель".into(), "Гость".into()],
            album: Some("Альбом".into()),
            year: Some(2001),
            ..Default::default()
        };
        // Минимальный «JPEG»: lofty не проверяет содержимое картинки.
        write_tags(&path, &meta, Some(&[0xFF, 0xD8, 0xFF, 0xD9])).unwrap();
        let (title, artist, cover) = read_title_artist(&path).unwrap();
        assert_eq!(title.as_deref(), Some("Название"));
        assert_eq!(artist.as_deref(), Some("Исполнитель, Гость"));
        assert!(cover);
        std::fs::remove_dir_all(dir).unwrap();
    }
}
