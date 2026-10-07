//! Получение прямой ссылки на mp3/aac через `download-info`.
//!
//! Схема: `/tracks/{id}/download-info` → список вариантов (кодек, битрейт,
//! `downloadInfoUrl`) → XML с `host`, `path`, `ts`, `s` → подписанная ссылка.

use crate::model::Quality;
use crate::{Error, Result};

use super::dto::DownloadInfo;

const SIGN_SALT: &str = "XGRlBW9FXlekgbPrRHuSiA";

#[derive(Debug, PartialEq, Eq)]
pub(crate) struct DownloadXml {
    pub host: String,
    pub path: String,
    pub ts: String,
    pub s: String,
}

fn extract_tag<'a>(xml: &'a str, tag: &str) -> Option<&'a str> {
    let open = format!("<{tag}>");
    let close = format!("</{tag}>");
    let start = xml.find(&open)? + open.len();
    let end = start + xml[start..].find(&close)?;
    Some(xml[start..end].trim())
}

pub(crate) fn parse_download_xml(xml: &str) -> Result<DownloadXml> {
    let get = |tag: &str| {
        extract_tag(xml, tag)
            .map(str::to_owned)
            .ok_or_else(|| Error::Unexpected(format!("в download-info нет <{tag}>")))
    };
    Ok(DownloadXml {
        host: get("host")?,
        path: get("path")?,
        ts: get("ts")?,
        s: get("s")?,
    })
}

pub(crate) fn build_direct_url(d: &DownloadXml) -> String {
    let path_tail = d.path.strip_prefix('/').unwrap_or(&d.path);
    let sign = md5::compute(format!("{SIGN_SALT}{path_tail}{}", d.s));
    format!("https://{}/get-mp3/{:x}/{}{}", d.host, sign, d.ts, d.path)
}

/// Выбор варианта под желаемое качество. Полные версии всегда важнее превью.
pub(crate) fn pick_variant(variants: &[DownloadInfo], quality: Quality) -> Option<&DownloadInfo> {
    let codec_rank = |codec: &str| match codec {
        "mp3" => 2,
        "aac" => 1,
        _ => 0,
    };
    let bitrate = |v: &DownloadInfo| v.bitrate_in_kbps.unwrap_or(0);

    variants.iter().max_by(|a, b| {
        let full = (!a.preview).cmp(&!b.preview);
        let by_quality = match quality {
            // Самый лёгкий поток; mp3 и aac на равных.
            Quality::Low => bitrate(b).cmp(&bitrate(a)),
            Quality::High => bitrate(a)
                .cmp(&bitrate(b))
                .then(codec_rank(&a.codec).cmp(&codec_rank(&b.codec))),
        };
        full.then(by_quality)
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    const XML: &str = r#"<?xml version="1.0" encoding="utf-8"?>
<download-info><host>s123.storage.yandex.net</host><path>/rmusic/U2FsdGVk/abc</path><ts>0005f1c2a3b4</ts><region>-1</region><s>deadbeef</s></download-info>"#;

    #[test]
    fn parses_xml() {
        let d = parse_download_xml(XML).unwrap();
        assert_eq!(d.host, "s123.storage.yandex.net");
        assert_eq!(d.path, "/rmusic/U2FsdGVk/abc");
        assert_eq!(d.ts, "0005f1c2a3b4");
        assert_eq!(d.s, "deadbeef");
    }

    #[test]
    fn missing_tag_is_error() {
        assert!(parse_download_xml("<download-info><host>h</host></download-info>").is_err());
    }

    #[test]
    fn direct_url_shape() {
        let d = parse_download_xml(XML).unwrap();
        let url = build_direct_url(&d);
        let expected_sign = format!(
            "{:x}",
            md5::compute(format!("{SIGN_SALT}rmusic/U2FsdGVk/abcdeadbeef"))
        );
        assert_eq!(
            url,
            format!("https://s123.storage.yandex.net/get-mp3/{expected_sign}/0005f1c2a3b4/rmusic/U2FsdGVk/abc")
        );
    }

    fn v(codec: &str, kbps: u32, preview: bool) -> DownloadInfo {
        DownloadInfo {
            codec: codec.into(),
            bitrate_in_kbps: Some(kbps),
            download_info_url: format!("{codec}{kbps}"),
            preview,
        }
    }

    #[test]
    fn picks_by_quality() {
        let list = [v("mp3", 320, false), v("aac", 64, false), v("mp3", 192, false)];
        assert_eq!(pick_variant(&list, Quality::High).unwrap().download_info_url, "mp3320");
        assert_eq!(pick_variant(&list, Quality::Low).unwrap().download_info_url, "aac64");
    }

    #[test]
    fn full_beats_preview() {
        let list = [v("mp3", 320, true), v("aac", 64, false)];
        assert_eq!(pick_variant(&list, Quality::High).unwrap().download_info_url, "aac64");
    }

    #[test]
    fn empty_list() {
        assert!(pick_variant(&[], Quality::High).is_none());
    }
}
