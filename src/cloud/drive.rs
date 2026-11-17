//! A thin client for the parts of the Google Drive REST API (v3) the app
//! uses. The app has the `drive.file` scope, so Drive shows it only the
//! files and folders it created itself.
//!
//! The app's files are found by their `appProperties`, not by name: Drive
//! allows several files of one name in a folder.
//! - Folders: `osFolder` = `root` (the "Oversample" folder), `annotations`
//!   or `projects`.
//! - Files: `osKind` = `batm` or `batproj`, and `osKey` = the local key
//!   (see `rules::drive_key`).

use super::google_auth::{self, TokenError};
use super::http::{self, Reply, Request};
use serde::Deserialize;
use std::collections::BTreeMap;

const API: &str = "https://www.googleapis.com/drive/v3";
const UPLOAD_API: &str = "https://www.googleapis.com/upload/drive/v3";
const FILE_FIELDS: &str = "id,name,createdTime,modifiedTime,headRevisionId,appProperties";
const FOLDER_MIME: &str = "application/vnd.google-apps.folder";
/// Type given to uploaded files, so Drive shows them as text.
const FILE_MIME: &str = "text/yaml";

pub const KIND_PROPERTY: &str = "osKind";
pub const FOLDER_PROPERTY: &str = "osFolder";

/// A Drive file or folder, with the fields the app asks for.
#[derive(Clone, Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DriveFile {
    pub id: String,
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub created_time: String,
    #[serde(default)]
    pub modified_time: String,
    #[serde(default)]
    pub head_revision_id: Option<String>,
    #[serde(default)]
    pub app_properties: BTreeMap<String, String>,
    #[serde(default)]
    pub trashed: bool,
}

impl DriveFile {
    pub fn property(&self, name: &str) -> Option<&str> {
        self.app_properties.get(name).map(String::as_str)
    }
}

/// Why a Drive call failed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DriveError {
    /// Not signed in any more (the token was refused twice, or the server
    /// would not give one).
    SignedOut,
    /// No answer, a server error or rate limiting: worth trying later.
    Unreachable,
    /// The user's Drive storage is full.
    Full,
    /// Drive refused the request with this status.
    Refused(u16),
}

/// Send a request with the access token. On 401 the token is renewed and the
/// request sent once more.
async fn call(
    method: &str,
    url: &str,
    content_type: Option<&str>,
    body: Option<&str>,
) -> Result<Reply, DriveError> {
    for attempt in 0..2 {
        let token = google_auth::access_token().await.map_err(|e| match e {
            TokenError::SignedOut => DriveError::SignedOut,
            TokenError::Unreachable => DriveError::Unreachable,
        })?;
        let mut req = Request::new(method, url).header("Authorization", format!("Bearer {token}"));
        if let Some(ct) = content_type {
            req = req.header("Content-Type", ct);
        }
        if let Some(b) = body {
            req = req.body(b.to_string());
        }
        req.keepalive = method != "GET";
        let reply = http::send(req).await.map_err(|_| DriveError::Unreachable)?;
        if reply.status == 401 {
            google_auth::forget_token();
            if attempt == 0 {
                continue;
            }
            return Err(DriveError::SignedOut);
        }
        return classify(reply);
    }
    Err(DriveError::SignedOut)
}

fn classify(reply: Reply) -> Result<Reply, DriveError> {
    match reply.status {
        s if (200..300).contains(&s) => Ok(reply),
        403 if reply.body.contains("storageQuotaExceeded") => Err(DriveError::Full),
        403 if reply.body.contains("rateLimitExceeded") => Err(DriveError::Unreachable),
        429 | 500..=599 => Err(DriveError::Unreachable),
        s => Err(DriveError::Refused(s)),
    }
}

fn parse<T: for<'de> Deserialize<'de>>(reply: &Reply) -> Result<T, DriveError> {
    serde_json::from_str(&reply.body).map_err(|e| {
        log::warn!("Google Drive reply not understood: {e}");
        DriveError::Refused(reply.status)
    })
}

/// The signed-in account's email address.
pub async fn account_email() -> Result<String, DriveError> {
    Ok(account().await?.0)
}

/// The signed-in account's email address and its stable Drive id
/// (`permissionId`, which stays the same if the email changes).
pub async fn account() -> Result<(String, String), DriveError> {
    #[derive(Deserialize)]
    #[serde(rename_all = "camelCase")]
    struct User {
        #[serde(default)]
        email_address: String,
        #[serde(default)]
        permission_id: String,
    }
    #[derive(Deserialize)]
    struct About {
        user: User,
    }
    let url = format!(
        "{API}/about?fields={}",
        http::encode("user(emailAddress,displayName,permissionId)")
    );
    let reply = call("GET", &url, None, None).await?;
    let about: About = parse(&reply)?;
    Ok((about.user.email_address, about.user.permission_id))
}

/// A Drive query string literal: in single quotes, with `\` and `'` escaped.
fn quoted(value: &str) -> String {
    format!("'{}'", value.replace('\\', "\\\\").replace('\'', "\\'"))
}

fn has_property(name: &str, value: &str) -> String {
    format!(
        "appProperties has {{ key={} and value={} }}",
        quoted(name),
        quoted(value)
    )
}

/// Every file matching the query `q`, page by page.
async fn query(q: &str) -> Result<Vec<DriveFile>, DriveError> {
    #[derive(Deserialize)]
    #[serde(rename_all = "camelCase")]
    struct Page {
        #[serde(default)]
        files: Vec<DriveFile>,
        #[serde(default)]
        next_page_token: Option<String>,
    }
    let fields = format!("nextPageToken,files({FILE_FIELDS})");
    let mut out = Vec::new();
    let mut page_token: Option<String> = None;
    loop {
        let mut url = format!(
            "{API}/files?spaces=drive&pageSize=1000&q={}&fields={}",
            http::encode(q),
            http::encode(&fields)
        );
        if let Some(t) = &page_token {
            url.push_str(&format!("&pageToken={}", http::encode(t)));
        }
        let reply = call("GET", &url, None, None).await?;
        let page: Page = parse(&reply)?;
        out.extend(page.files);
        match page.next_page_token {
            Some(t) if !t.is_empty() => page_token = Some(t),
            _ => break,
        }
    }
    Ok(out)
}

/// The app's files of one kind (`batm` or `batproj`), not in the trash.
pub async fn files_of_kind(kind: &str) -> Result<Vec<DriveFile>, DriveError> {
    query(&format!(
        "{} and trashed=false",
        has_property(KIND_PROPERTY, kind)
    ))
    .await
}

/// The app's files of one kind with the key property `drive_key`.
pub async fn files_with_key(kind: &str, drive_key: &str) -> Result<Vec<DriveFile>, DriveError> {
    query(&format!(
        "{} and {} and trashed=false",
        has_property(KIND_PROPERTY, kind),
        has_property(super::rules::KEY_PROPERTY, drive_key)
    ))
    .await
}

/// The app's folders with `osFolder` = `marker` that this client can see.
/// Creates nothing.
pub async fn find_folders(marker: &str) -> Result<Vec<DriveFile>, DriveError> {
    query(&format!(
        "mimeType={} and {} and trashed=false",
        quoted(FOLDER_MIME),
        has_property(FOLDER_PROPERTY, marker)
    ))
    .await
}

/// The id of the app's folder with `osFolder` = `marker`, creating it (in
/// `parent`, or in My Drive) if there is none. Of several, the oldest is
/// used, so that every device picks the same one.
pub async fn folder(marker: &str, name: &str, parent: Option<&str>) -> Result<String, DriveError> {
    let found = find_folders(marker).await?;
    if let Some(oldest) = found
        .iter()
        .min_by(|a, b| a.created_time.cmp(&b.created_time))
    {
        return Ok(oldest.id.clone());
    }
    let mut meta = serde_json::json!({
        "name": name,
        "mimeType": FOLDER_MIME,
        "appProperties": { FOLDER_PROPERTY: marker },
    });
    if let Some(p) = parent {
        meta["parents"] = serde_json::json!([p]);
    }
    let url = format!("{API}/files?fields={}", http::encode(FILE_FIELDS));
    let reply = call(
        "POST",
        &url,
        Some("application/json; charset=UTF-8"),
        Some(&meta.to_string()),
    )
    .await?;
    let created: DriveFile = parse(&reply)?;
    Ok(created.id)
}

/// A file's current metadata, or `None` if it is gone or in the trash.
pub async fn metadata(id: &str) -> Result<Option<DriveFile>, DriveError> {
    let fields = format!("{FILE_FIELDS},trashed");
    let url = format!(
        "{API}/files/{}?fields={}",
        http::encode(id),
        http::encode(&fields)
    );
    match call("GET", &url, None, None).await {
        Ok(reply) => {
            let file: DriveFile = parse(&reply)?;
            Ok((!file.trashed).then_some(file))
        }
        Err(DriveError::Refused(404)) => Ok(None),
        Err(e) => Err(e),
    }
}

/// A file's content as text.
pub async fn download(id: &str) -> Result<String, DriveError> {
    let url = format!("{API}/files/{}?alt=media", http::encode(id));
    Ok(call("GET", &url, None, None).await?.body)
}

/// The text of a multipart upload: JSON metadata, then the file content.
pub fn multipart_body(boundary: &str, metadata_json: &str, content: &str) -> String {
    format!(
        "--{boundary}\r\nContent-Type: application/json; charset=UTF-8\r\n\r\n{metadata_json}\r\n\
         --{boundary}\r\nContent-Type: {FILE_MIME}; charset=UTF-8\r\n\r\n{content}\r\n--{boundary}--"
    )
}

/// A boundary that does not occur in `content`.
fn boundary_for(content: &str) -> String {
    let mut n = 0u32;
    loop {
        let b = format!("oversample-part-{n}");
        if !content.contains(&b) {
            return b;
        }
        n += 1;
    }
}

async fn upload(
    method: &str,
    url_path: &str,
    metadata: serde_json::Value,
    content: &str,
) -> Result<DriveFile, DriveError> {
    let boundary = boundary_for(content);
    let body = multipart_body(&boundary, &metadata.to_string(), content);
    let url = format!(
        "{UPLOAD_API}/{url_path}?uploadType=multipart&fields={}",
        http::encode(FILE_FIELDS)
    );
    let reply = call(
        method,
        &url,
        Some(&format!("multipart/related; boundary={boundary}")),
        Some(&body),
    )
    .await?;
    parse(&reply)
}

/// Create a file named `name` in folder `parent`.
pub async fn create(
    parent: &str,
    name: &str,
    properties: &BTreeMap<String, String>,
    content: &str,
) -> Result<DriveFile, DriveError> {
    let meta = serde_json::json!({
        "name": name,
        "mimeType": FILE_MIME,
        "parents": [parent],
        "appProperties": properties,
    });
    upload("POST", "files", meta, content).await
}

/// Replace the content of file `id` (and set its name and properties).
pub async fn update(
    id: &str,
    name: &str,
    properties: &BTreeMap<String, String>,
    content: &str,
) -> Result<DriveFile, DriveError> {
    let meta = serde_json::json!({
        "name": name,
        "appProperties": properties,
    });
    upload(
        "PATCH",
        &format!("files/{}", http::encode(id)),
        meta,
        content,
    )
    .await
}

/// Move file `id` to the Drive trash.
pub async fn trash(id: &str) -> Result<(), DriveError> {
    let url = format!("{API}/files/{}?fields=id", http::encode(id));
    match call(
        "PATCH",
        &url,
        Some("application/json; charset=UTF-8"),
        Some(r#"{"trashed":true}"#),
    )
    .await
    {
        Ok(_) | Err(DriveError::Refused(404)) => Ok(()),
        Err(e) => Err(e),
    }
}
