//! Your screenshots on Steam: the uploaded list, uploading from this PC,
//! changing a caption, spoiler tag or visibility, and deleting.
//!
//! An upload is what Steam's client does: the picture and its thumbnail go
//! into Steam Cloud under the screenshots app (760), then
//! `ClientUCMAddScreenshot` publishes them to your profile. Files on this PC
//! are only touched after Steam has answered.

use local::screenshots::{Edit, Shot, Visibility};
use model::{AppId, SteamId};
use net::Connection;
use proto::*;
use std::path::PathBuf;

/// Steam's own app for screenshots.
const SCREENSHOTS_APP: AppId = 760;
const THUMB_WIDTH: u32 = 200;
const PER_PAGE: u32 = 50;
const OK: i32 = 1;
const NO_ID: u64 = u64::MAX;

/// Steam's limit for a screenshot caption.
pub const CAPTION_MAX: usize = 140;

const CLIENT_UCM_ADD_SCREENSHOT: u32 = 7301;
const CLIENT_UCM_DELETE_SCREENSHOT: u32 = 7309;

// From Steam's `steammessages_clientserver_ucm.proto`: only these messages
// are used, so they're written here rather than vendoring the whole file.
#[derive(Clone, PartialEq, prost::Message)]
struct UcmAddScreenshot {
    #[prost(uint32, optional, tag = "1")]
    appid: Option<u32>,
    #[prost(string, optional, tag = "2")]
    filename: Option<String>,
    #[prost(string, optional, tag = "3")]
    thumbname: Option<String>,
    #[prost(fixed32, optional, tag = "4")]
    rtime32_created: Option<u32>,
    #[prost(uint32, optional, tag = "5")]
    width: Option<u32>,
    #[prost(uint32, optional, tag = "6")]
    height: Option<u32>,
    #[prost(uint32, optional, tag = "7")]
    permissions: Option<u32>,
    #[prost(string, optional, tag = "8")]
    caption: Option<String>,
    #[prost(bool, optional, tag = "12")]
    spoiler_tag: Option<bool>,
}

#[derive(Clone, PartialEq, prost::Message)]
struct UcmAddScreenshotResponse {
    #[prost(int32, optional, tag = "1", default = "2")]
    eresult: Option<i32>,
    #[prost(fixed64, optional, tag = "2", default = "18446744073709551615")]
    screenshotid: Option<u64>,
    #[prost(uint64, optional, tag = "3")]
    publishedfileid: Option<u64>,
}

#[derive(Clone, PartialEq, prost::Message)]
struct UcmDeleteScreenshot {
    #[prost(fixed64, optional, tag = "1", default = "18446744073709551615")]
    screenshotid: Option<u64>,
}

#[derive(Clone, PartialEq, prost::Message)]
struct UcmDeleteScreenshotResponse {
    #[prost(int32, optional, tag = "1", default = "2")]
    eresult: Option<i32>,
}

/// A screenshot on your Steam profile.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Uploaded {
    pub id: u64,
    pub app_id: AppId,
    pub caption: String,
    pub preview: Option<String>,
    pub image: Option<String>,
    pub created: u64,
    pub bytes: u64,
    pub spoiler: bool,
    pub visibility: Visibility,
}

/// What to ask Steam. Send as [`crate::Command::Screenshots`]; the answer
/// comes back as [`crate::Event::Screenshots`].
#[derive(Debug, Clone)]
pub enum Request {
    /// A page (from 1) of your uploaded screenshots, newest first.
    List { page: u32 },
    /// Upload this screenshot with these details.
    Upload { steam_root: PathBuf, account: u32, shot: Box<Shot>, edit: Edit },
    /// Change an uploaded screenshot's details.
    Update { id: u64, edit: Edit },
    /// Delete an uploaded screenshot. `local` is the copy on this PC, whose
    /// record is updated (the picture itself stays).
    Delete { id: u64, handle: Option<u64>, local: Option<(PathBuf, u32, Box<Shot>)> },
}

#[derive(Debug, Clone)]
pub enum Response {
    List { page: u32, result: Result<(u64, Vec<Uploaded>), String> },
    Uploaded { name: String, result: Result<u64, String> },
    Updated { id: u64, edit: Edit, result: Result<(), String> },
    Deleted { id: u64, result: Result<(), String> },
}

fn uploaded(d: &PublishedFileDetails) -> Uploaded {
    let url = |s: &str| (s.starts_with("https://") || s.starts_with("http://")).then(|| s.to_string());
    // Steam keeps a screenshot's caption as its description.
    let caption = if d.file_description().is_empty() { d.title() } else { d.file_description() };
    Uploaded {
        id: d.publishedfileid(),
        app_id: d.consumer_appid(),
        caption: caption.to_string(),
        preview: url(d.preview_url()),
        image: url(d.image_url()).or_else(|| url(d.file_url())),
        created: d.time_created() as u64,
        bytes: d.file_size(),
        spoiler: d.spoiler_tag(),
        visibility: Visibility::from_published(d.visibility() as i64),
    }
}

async fn list(conn: &Connection, me: SteamId, page: u32) -> Result<(u64, Vec<Uploaded>), String> {
    let req = CPublishedFileGetUserFilesRequest {
        steamid: Some(me),
        appid: Some(0),
        page: Some(page.max(1)),
        numperpage: Some(PER_PAGE),
        r#type: Some("myfiles".into()),
        sortmethod: Some("created".into()),
        // EPublishedFileInfoMatchingFileType: screenshots.
        filetype: Some(4),
        return_short_description: Some(true),
        ..Default::default()
    };
    let r: CPublishedFileGetUserFilesResponse = conn.call("PublishedFile.GetUserFiles#1", &req).await.map_err(|e| e.to_string())?;
    Ok((r.total() as u64, r.publishedfiledetails.iter().filter(|d| d.result() == OK as u32).map(uploaded).collect()))
}

/// The picture's size and a thumbnail's bytes (the saved one, or a new one).
fn prepare(shot: &Shot) -> Result<(Vec<u8>, Vec<u8>, u32, u32), String> {
    let data = std::fs::read(&shot.path).map_err(|e| e.to_string())?;
    let img = image::load_from_memory(&data).map_err(|e| e.to_string())?;
    let (w, h) = (img.width(), img.height());
    let thumb = match shot.thumbnail.as_ref().and_then(|p| std::fs::read(p).ok()) {
        Some(t) => t,
        None => {
            let tw = THUMB_WIDTH.min(w).max(1);
            let th = (h as u64 * tw as u64 / w.max(1) as u64).max(1) as u32;
            let small = image::imageops::thumbnail(&img.to_rgb8(), tw, th);
            let mut out = std::io::Cursor::new(Vec::new());
            small.write_to(&mut out, image::ImageFormat::Jpeg).map_err(|e| e.to_string())?;
            out.into_inner()
        }
    };
    Ok((data, thumb, w, h))
}

fn add_message(shot: &Shot, edit: &Edit, file: String, thumb: String, w: u32, h: u32) -> UcmAddScreenshot {
    UcmAddScreenshot {
        appid: Some(shot.game as u32),
        filename: Some(file),
        thumbname: Some(thumb),
        rtime32_created: Some(shot.created as u32),
        width: Some(w),
        height: Some(h),
        permissions: Some(edit.visibility.ucm()),
        caption: Some(clean_caption(&edit.caption)),
        spoiler_tag: Some(edit.spoiler),
    }
}

/// One line, at most Steam's length.
pub fn clean_caption(text: &str) -> String {
    text.replace(['\r', '\n'], " ").trim().chars().take(CAPTION_MAX).collect()
}

async fn upload(conn: &Connection, cell: u32, root: PathBuf, account: u32, shot: Shot, edit: Edit) -> Result<u64, String> {
    // Non-Steam games have ids too big for an app id: Steam's upload has no
    // way to name them.
    if shot.game > u32::MAX as u64 {
        return Err("Screenshots of non-Steam games can't be uploaded.".into());
    }
    let for_files = shot.clone();
    let (data, thumb, w, h) = tokio::task::spawn_blocking(move || prepare(&for_files)).await.map_err(|e| e.to_string())??;
    let file = format!("{}/screenshots/{}", shot.game, shot.name);
    let thumb_name = format!("{}/screenshots/thumbnails/{}", shot.game, shot.name);
    let files = vec![(file.clone(), shot.path.clone(), data), (thumb_name.clone(), shot.path.clone(), thumb)];
    let done = crate::cloud::upload_files(conn, SCREENSHOTS_APP, cell, &files).await?;
    if done.len() != files.len() {
        return Err("Steam Cloud didn't accept the screenshot.".into());
    }
    let reply = conn.request(CLIENT_UCM_ADD_SCREENSHOT, &add_message(&shot, &edit, file, thumb_name, w, h)).await.map_err(|e| e.to_string())?;
    let r: UcmAddScreenshotResponse = reply.body().map_err(|e| e.to_string())?;
    if r.eresult() != OK {
        return Err(format!("Steam refused the screenshot (error {}).", r.eresult()));
    }
    let handle = r.screenshotid();
    let id = r.publishedfileid.filter(|v| *v != 0).unwrap_or(handle);
    tokio::task::spawn_blocking(move || {
        local::screenshots::save_edit(&root, account, &shot, &edit)?;
        local::screenshots::set_uploaded(&root, account, &shot, Some((id, handle)))
    })
    .await
    .map_err(|e| e.to_string())??;
    Ok(id)
}

async fn update(conn: &Connection, id: u64, edit: &Edit) -> Result<(), String> {
    let req = CPublishedFileUpdateRequest {
        appid: Some(SCREENSHOTS_APP),
        publishedfileid: Some(id),
        file_description: Some(clean_caption(&edit.caption)),
        visibility: Some(edit.visibility.published()),
        spoiler_tag: Some(edit.spoiler),
        ..Default::default()
    };
    conn.call::<_, CPublishedFileUpdateResponse>("PublishedFile.Update#1", &req).await.map(|_| ()).map_err(|e| e.to_string())
}

async fn delete(conn: &Connection, id: u64, handle: Option<u64>) -> Result<(), String> {
    if let Some(h) = handle.filter(|h| *h != NO_ID) {
        let reply = conn.request(CLIENT_UCM_DELETE_SCREENSHOT, &UcmDeleteScreenshot { screenshotid: Some(h) }).await.map_err(|e| e.to_string())?;
        let r: UcmDeleteScreenshotResponse = reply.body().map_err(|e| e.to_string())?;
        return if r.eresult() == OK { Ok(()) } else { Err(format!("Steam refused to delete it (error {}).", r.eresult())) };
    }
    let req = CPublishedFileDeleteRequest { publishedfileid: Some(id), appid: Some(SCREENSHOTS_APP) };
    conn.call::<_, CPublishedFileDeleteResponse>("PublishedFile.Delete#1", &req).await.map(|_| ()).map_err(|e| e.to_string())
}

/// Do what was asked and report it.
pub async fn run(conn: Connection, me: SteamId, cell: u32, request: Request, out: impl Fn(Response)) {
    match request {
        Request::List { page } => out(Response::List { page, result: list(&conn, me, page).await }),
        Request::Upload { steam_root, account, shot, edit } => {
            let name = shot.name.clone();
            out(Response::Uploaded { name, result: upload(&conn, cell, steam_root, account, *shot, edit).await });
        }
        Request::Update { id, edit } => {
            let result = update(&conn, id, &edit).await;
            out(Response::Updated { id, edit, result });
        }
        Request::Delete { id, handle, local } => {
            let mut result = delete(&conn, id, handle).await;
            if result.is_ok()
                && let Some((root, account, shot)) = local
            {
                let cleared = tokio::task::spawn_blocking(move || local::screenshots::set_uploaded(&root, account, &shot, None)).await;
                result = cleared.map_err(|e| e.to_string()).and_then(|r| r);
            }
            out(Response::Deleted { id, result });
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use prost::Message;

    fn shot() -> Shot {
        Shot {
            game: 440,
            name: "20240101120000_1.jpg".into(),
            path: PathBuf::from("x.jpg"),
            thumbnail: None,
            created: 1_704_110_400,
            width: 0,
            height: 0,
            bytes: 0,
            caption: String::new(),
            spoiler: false,
            visibility: Visibility::Public,
            published: None,
            handle: None,
        }
    }

    /// Field numbers and wire types from Steam's definition.
    #[test]
    fn add_message_matches_steams_format() {
        let edit = Edit { caption: "Hi".into(), spoiler: true, visibility: Visibility::Friends };
        let m = add_message(&shot(), &edit, "440/screenshots/a.jpg".into(), "440/screenshots/thumbnails/a.jpg".into(), 1920, 1080);
        let b = m.encode_to_vec();
        assert_eq!(&b[..2], &[1 << 3, 0xB8], "appid is field 1, a varint");
        // Field 4 is fixed32 (wire type 5), field 12 a bool.
        let created = (4 << 3 | 5) as u8;
        let at = b.iter().position(|x| *x == created).unwrap();
        assert_eq!(u32::from_le_bytes(b[at + 1..at + 5].try_into().unwrap()), 1_704_110_400);
        assert_eq!(&b[b.len() - 2..], &[12 << 3, 1]);
        assert_eq!(UcmAddScreenshot::decode(b.as_slice()).unwrap().permissions, Some(4), "friends-only in the UCM numbering");
    }

    #[test]
    fn reads_steams_answers() {
        let mut b = vec![1 << 3, 1];
        b.push(2 << 3 | 1);
        b.extend(77u64.to_le_bytes());
        b.extend([3 << 3, 99]);
        let r = UcmAddScreenshotResponse::decode(b.as_slice()).unwrap();
        assert_eq!((r.eresult(), r.screenshotid(), r.publishedfileid), (1, 77, Some(99)));
        // Nothing in the reply means failure, not success.
        let empty = UcmAddScreenshotResponse::decode(&[][..]).unwrap();
        assert_eq!((empty.eresult(), empty.screenshotid()), (2, NO_ID));
        assert_eq!(UcmDeleteScreenshotResponse::decode(&[][..]).unwrap().eresult(), 2);
        let del = UcmDeleteScreenshot { screenshotid: Some(5) }.encode_to_vec();
        assert_eq!(del[0], 1 << 3 | 1, "the id is fixed64");
    }

    #[test]
    fn captions_are_one_line_and_short() {
        assert_eq!(clean_caption("  a\nb\r\nc "), "a b  c");
        assert_eq!(clean_caption(&"я".repeat(500)).chars().count(), CAPTION_MAX);
    }

    #[test]
    fn reads_an_uploaded_screenshot() {
        let d = PublishedFileDetails {
            publishedfileid: Some(9),
            consumer_appid: Some(440),
            file_description: Some("Caption".into()),
            preview_url: Some("https://images.example/p.jpg".into()),
            file_url: Some("javascript:alert(1)".into()),
            visibility: Some(2),
            spoiler_tag: Some(true),
            time_created: Some(100),
            ..Default::default()
        };
        let u = uploaded(&d);
        assert_eq!((u.id, u.app_id, u.caption.as_str()), (9, 440, "Caption"));
        assert_eq!(u.visibility, Visibility::Private, "published-file numbering");
        assert!(u.spoiler);
        assert_eq!(u.image, None, "only web addresses are kept");
    }
}
