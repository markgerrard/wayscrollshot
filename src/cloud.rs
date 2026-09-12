//! DigitalOcean Spaces upload and the lightweight local cloud-gallery cache.

use std::fs;
use std::io::Write;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::Arc;

use anyhow::{anyhow, bail, Context, Result};
use chrono::Utc;
use hmac::{Hmac, Mac};
use image::codecs::png::PngEncoder;
use image::{ExtendedColorType, ImageEncoder, RgbaImage};
use rusqlite::{params, Connection};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::stitch::build_preview;
use crate::types::PreviewImage;

type HmacSha256 = Hmac<Sha256>;

#[derive(Clone, Debug, Deserialize)]
pub struct CloudConfig {
    pub enabled: bool,
    pub endpoint: String,
    pub region: String,
    pub bucket: String,
    #[serde(default = "default_key_prefix")]
    pub key_prefix: String,
    pub public_base_url: String,
    #[serde(default = "default_acl")]
    pub object_acl: String,
    #[serde(default = "default_cache_ttl_hours")]
    pub cache_ttl_hours: u64,
}

#[derive(Clone, Debug)]
pub struct CloudEntry {
    pub id: i64,
    pub url: String,
    pub local_path: Option<PathBuf>,
    pub width: u32,
    pub height: u32,
    pub created_at: i64,
    pub preview_expires_at: Option<i64>,
}

#[derive(Serialize)]
struct PanelEntry {
    id: i64,
    url: String,
    thumbnail: String,
    width: u32,
    height: u32,
    created_at: i64,
}

fn default_key_prefix() -> String {
    "screenshots".into()
}

fn default_acl() -> String {
    "public-read".into()
}

fn default_cache_ttl_hours() -> u64 {
    24
}

fn config_root() -> Result<PathBuf> {
    Ok(
        PathBuf::from(std::env::var_os("HOME").context("HOME is not set")?)
            .join(".config/wayscrollshot"),
    )
}

fn cache_root() -> Result<PathBuf> {
    if let Some(root) = std::env::var_os("XDG_CACHE_HOME") {
        return Ok(PathBuf::from(root).join("wayscrollshot"));
    }
    Ok(
        PathBuf::from(std::env::var_os("HOME").context("HOME is not set")?)
            .join(".cache/wayscrollshot"),
    )
}

pub fn load_config() -> Result<CloudConfig> {
    let path = config_root()?.join("cloud.toml");
    let raw = fs::read_to_string(&path)
        .with_context(|| format!("cloud is not configured; expected {}", path.display()))?;
    let config: CloudConfig = toml::from_str(&raw).context("invalid cloud.toml")?;
    if !config.enabled {
        bail!("cloud uploads are disabled in cloud.toml");
    }
    if !config.endpoint.starts_with("https://")
        || config.region.trim().is_empty()
        || config.bucket.trim().is_empty()
        || !config.public_base_url.starts_with("https://")
    {
        bail!("cloud.toml contains incomplete Spaces settings");
    }
    Ok(config)
}

fn secret(kind: &str) -> Result<String> {
    let output = Command::new("secret-tool")
        .args([
            "lookup",
            "app",
            "wayscrollshot",
            "provider",
            "digitalocean-spaces",
            "profile",
            "default",
            "kind",
            kind,
        ])
        .output()
        .context("failed to read the desktop keyring")?;
    if !output.status.success() {
        bail!("could not read {kind} from the desktop keyring");
    }
    let value = String::from_utf8(output.stdout)
        .context("keyring returned invalid text")?
        .trim()
        .to_owned();
    if value.is_empty() {
        bail!("missing {kind} in the desktop keyring");
    }
    Ok(value)
}

pub fn upload(image: &Arc<RgbaImage>, local_path: &Path) -> Result<CloudEntry> {
    let config = load_config()?;
    let access_key = secret("access-key-id")?;
    let secret_key = secret("secret-access-key")?;
    let png = encode_png(image)?;
    let filename = format!(
        "wayscrollshot-{}.png",
        Utc::now().format("%Y%m%d-%H%M%S-%3f")
    );
    let prefix = config.key_prefix.trim_matches('/');
    let key = if prefix.is_empty() {
        filename
    } else {
        format!("{prefix}/{filename}")
    };
    put_object(&config, &access_key, &secret_key, &key, &png)?;

    let public_url = format!(
        "{}/{}",
        config.public_base_url.trim_end_matches('/'),
        encode_key(&key)
    );
    let id = record_upload(&config, &key, &public_url, local_path, image)?;
    Ok(CloudEntry {
        id,
        url: public_url,
        local_path: Some(local_path.to_owned()),
        width: image.width(),
        height: image.height(),
        created_at: Utc::now().timestamp(),
        preview_expires_at: Some(
            Utc::now().timestamp() + (config.cache_ttl_hours.min(24 * 30) * 3600) as i64,
        ),
    })
}

fn put_object(
    config: &CloudConfig,
    access_key: &str,
    secret_key: &str,
    key: &str,
    body: &[u8],
) -> Result<()> {
    let endpoint_host = config
        .endpoint
        .trim_end_matches('/')
        .strip_prefix("https://")
        .context("Spaces endpoint must use https")?;
    let host = format!("{}.{}", config.bucket, endpoint_host);
    let canonical_uri = format!("/{}", encode_key(key));
    let url = format!("https://{host}{canonical_uri}");
    let now = Utc::now();
    let amz_date = now.format("%Y%m%dT%H%M%SZ").to_string();
    let short_date = now.format("%Y%m%d").to_string();
    let content_hash = sha256_hex(body);
    let canonical_headers = format!(
        "content-type:image/png\nhost:{host}\nx-amz-acl:{}\nx-amz-content-sha256:{content_hash}\nx-amz-date:{amz_date}\n",
        config.object_acl
    );
    let signed_headers = "content-type;host;x-amz-acl;x-amz-content-sha256;x-amz-date";
    let canonical_request =
        format!("PUT\n{canonical_uri}\n\n{canonical_headers}\n{signed_headers}\n{content_hash}");
    let scope = format!("{short_date}/{}/s3/aws4_request", config.region);
    let string_to_sign = format!(
        "AWS4-HMAC-SHA256\n{amz_date}\n{scope}\n{}",
        sha256_hex(canonical_request.as_bytes())
    );
    let date_key = hmac(
        format!("AWS4{secret_key}").as_bytes(),
        short_date.as_bytes(),
    )?;
    let region_key = hmac(&date_key, config.region.as_bytes())?;
    let service_key = hmac(&region_key, b"s3")?;
    let signing_key = hmac(&service_key, b"aws4_request")?;
    let signature = hex(&hmac(&signing_key, string_to_sign.as_bytes())?);
    let authorization = format!(
        "AWS4-HMAC-SHA256 Credential={access_key}/{scope}, SignedHeaders={signed_headers}, Signature={signature}"
    );

    let mut child = Command::new("curl")
        .args(["--fail", "--silent", "--show-error", "--request", "PUT"])
        .arg(&url)
        .args(["--header", "Content-Type: image/png"])
        .args(["--header", &format!("Host: {host}")])
        .args(["--header", &format!("x-amz-acl: {}", config.object_acl)])
        .args(["--header", &format!("x-amz-content-sha256: {content_hash}")])
        .args(["--header", &format!("x-amz-date: {amz_date}")])
        .args(["--header", &format!("Authorization: {authorization}")])
        .args(["--data-binary", "@-"])
        .stdin(Stdio::piped())
        .spawn()
        .context("failed to start curl for Spaces upload")?;
    child
        .stdin
        .as_mut()
        .context("failed to open upload stream")?
        .write_all(body)?;
    let status = child.wait()?;
    if !status.success() {
        bail!("Spaces upload failed");
    }
    Ok(())
}

#[cfg(test)]
fn delete_object(
    config: &CloudConfig,
    access_key: &str,
    secret_key: &str,
    key: &str,
) -> Result<()> {
    let endpoint_host = config
        .endpoint
        .trim_end_matches('/')
        .strip_prefix("https://")
        .context("Spaces endpoint must use https")?;
    let host = format!("{}.{}", config.bucket, endpoint_host);
    let canonical_uri = format!("/{}", encode_key(key));
    let url = format!("https://{host}{canonical_uri}");
    let now = Utc::now();
    let amz_date = now.format("%Y%m%dT%H%M%SZ").to_string();
    let short_date = now.format("%Y%m%d").to_string();
    let content_hash = sha256_hex(&[]);
    let canonical_headers =
        format!("host:{host}\nx-amz-content-sha256:{content_hash}\nx-amz-date:{amz_date}\n");
    let signed_headers = "host;x-amz-content-sha256;x-amz-date";
    let canonical_request =
        format!("DELETE\n{canonical_uri}\n\n{canonical_headers}\n{signed_headers}\n{content_hash}");
    let scope = format!("{short_date}/{}/s3/aws4_request", config.region);
    let string_to_sign = format!(
        "AWS4-HMAC-SHA256\n{amz_date}\n{scope}\n{}",
        sha256_hex(canonical_request.as_bytes())
    );
    let date_key = hmac(
        format!("AWS4{secret_key}").as_bytes(),
        short_date.as_bytes(),
    )?;
    let region_key = hmac(&date_key, config.region.as_bytes())?;
    let service_key = hmac(&region_key, b"s3")?;
    let signing_key = hmac(&service_key, b"aws4_request")?;
    let signature = hex(&hmac(&signing_key, string_to_sign.as_bytes())?);
    let authorization = format!(
        "AWS4-HMAC-SHA256 Credential={access_key}/{scope}, SignedHeaders={signed_headers}, Signature={signature}"
    );
    let status = Command::new("curl")
        .args(["--fail", "--silent", "--show-error", "--request", "DELETE"])
        .arg(url)
        .args(["--header", &format!("Host: {host}")])
        .args(["--header", &format!("x-amz-content-sha256: {content_hash}")])
        .args(["--header", &format!("x-amz-date: {amz_date}")])
        .args(["--header", &format!("Authorization: {authorization}")])
        .status()
        .context("failed to start curl for Spaces deletion")?;
    if !status.success() {
        bail!("Spaces deletion failed");
    }
    Ok(())
}

pub fn copy_url(url: &str) -> Result<()> {
    let mut child = Command::new("wl-copy")
        .arg("--type")
        .arg("text/plain;charset=utf-8")
        .stdin(Stdio::piped())
        .spawn()
        .context("failed to start wl-copy")?;
    child
        .stdin
        .as_mut()
        .context("failed to open clipboard stream")?
        .write_all(url.as_bytes())?;
    if !child.wait()?.success() {
        bail!("wl-copy failed");
    }
    Ok(())
}

pub fn gallery_entries() -> Result<Vec<CloudEntry>> {
    let conn = open_cache()?;
    conn.execute(
        "UPDATE cloud_items SET preview_png = NULL, preview_expires_at = NULL
         WHERE preview_expires_at IS NOT NULL AND preview_expires_at < ?1",
        [Utc::now().timestamp()],
    )?;
    let mut statement = conn.prepare(
        "SELECT id, url, local_path, width, height, created_at, preview_expires_at
         FROM cloud_items ORDER BY created_at DESC",
    )?;
    let rows = statement.query_map([], |row| {
        let local: Option<String> = row.get(2)?;
        Ok(CloudEntry {
            id: row.get(0)?,
            url: row.get(1)?,
            local_path: local.map(PathBuf::from),
            width: row.get::<_, i64>(3)? as u32,
            height: row.get::<_, i64>(4)? as u32,
            created_at: row.get(5)?,
            preview_expires_at: row.get(6)?,
        })
    })?;
    rows.collect::<rusqlite::Result<Vec<_>>>()
        .map_err(Into::into)
}

pub fn panel_entries_json() -> Result<String> {
    let entries = gallery_entries()?;
    let thumbnail_root = cache_root()?.join("thumbnails");
    fs::create_dir_all(&thumbnail_root)?;
    fs::set_permissions(&thumbnail_root, fs::Permissions::from_mode(0o700))?;
    let now = Utc::now().timestamp();
    let mut panel_entries = Vec::with_capacity(entries.len());
    for entry in entries {
        let thumbnail_path = thumbnail_root.join(format!("{}.png", entry.id));
        if !thumbnail_path.is_file() || entry.preview_expires_at.is_none_or(|expiry| expiry <= now)
        {
            let preview = gallery_preview(&entry, 360)?;
            let image = RgbaImage::from_raw(preview.width, preview.height, preview.pixels)
                .context("invalid panel thumbnail pixels")?;
            fs::write(&thumbnail_path, encode_png(&Arc::new(image))?)?;
            fs::set_permissions(&thumbnail_path, fs::Permissions::from_mode(0o600))?;
        }
        panel_entries.push(PanelEntry {
            id: entry.id,
            url: entry.url,
            thumbnail: format!("file://{}", thumbnail_path.to_string_lossy()),
            width: entry.width,
            height: entry.height,
            created_at: entry.created_at,
        });
    }
    serde_json::to_string(&panel_entries).context("failed to encode cloud gallery")
}

pub fn gallery_preview(entry: &CloudEntry, width: u32) -> Result<PreviewImage> {
    let config = load_config()?;
    let conn = open_cache()?;
    let cached: Option<Vec<u8>> = conn
        .query_row(
            "SELECT preview_png FROM cloud_items WHERE id = ?1",
            [entry.id],
            |row| row.get(0),
        )
        .unwrap_or(None);
    if let Some(png) = cached {
        let image = image::load_from_memory(&png)?.to_rgba8();
        return Ok(build_preview(&image, width));
    }

    let image = load_image(entry)?;
    let preview = build_preview(&image, width.max(1));
    let preview_image = RgbaImage::from_raw(preview.width, preview.height, preview.pixels.clone())
        .context("invalid preview pixels")?;
    let png = encode_png(&Arc::new(preview_image))?;
    let expiry = Utc::now().timestamp() + (config.cache_ttl_hours.min(24 * 30) * 3600) as i64;
    conn.execute(
        "UPDATE cloud_items SET preview_png = ?1, preview_expires_at = ?2 WHERE id = ?3",
        params![png, expiry, entry.id],
    )?;
    Ok(preview)
}

/// Loads the original capture from disk, falling back to its public cloud URL.
pub fn load_image(entry: &CloudEntry) -> Result<RgbaImage> {
    if let Some(path) = entry.local_path.as_ref().filter(|path| path.is_file()) {
        return Ok(image::open(path)?.to_rgba8());
    }
    let output = Command::new("curl")
        .args(["--fail", "--silent", "--show-error"])
        .arg(&entry.url)
        .output()
        .context("failed to fetch cloud capture")?;
    if !output.status.success() {
        bail!("could not fetch cloud capture");
    }
    Ok(image::load_from_memory(&output.stdout)?.to_rgba8())
}

fn record_upload(
    config: &CloudConfig,
    key: &str,
    url: &str,
    local_path: &Path,
    image: &RgbaImage,
) -> Result<i64> {
    let conn = open_cache()?;
    let preview = build_preview(image, 480);
    let preview_image = RgbaImage::from_raw(preview.width, preview.height, preview.pixels)
        .context("invalid preview pixels")?;
    let preview_png = encode_png(&Arc::new(preview_image))?;
    let now = Utc::now().timestamp();
    let expiry = now + (config.cache_ttl_hours.min(24 * 30) * 3600) as i64;
    conn.execute(
        "INSERT INTO cloud_items
         (object_key, url, local_path, width, height, created_at, preview_png, preview_expires_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
        params![
            key,
            url,
            local_path.to_string_lossy(),
            image.width() as i64,
            image.height() as i64,
            now,
            preview_png,
            expiry
        ],
    )?;
    Ok(conn.last_insert_rowid())
}

fn open_cache() -> Result<Connection> {
    let root = cache_root()?;
    fs::create_dir_all(&root)?;
    fs::set_permissions(&root, fs::Permissions::from_mode(0o700))?;
    let path = root.join("cloud.sqlite3");
    let conn = Connection::open(&path)?;
    fs::set_permissions(&path, fs::Permissions::from_mode(0o600))?;
    conn.execute_batch(
        "PRAGMA journal_mode = WAL;
         CREATE TABLE IF NOT EXISTS cloud_items (
           id INTEGER PRIMARY KEY,
           object_key TEXT NOT NULL UNIQUE,
           url TEXT NOT NULL,
           local_path TEXT,
           width INTEGER NOT NULL,
           height INTEGER NOT NULL,
           created_at INTEGER NOT NULL,
           preview_png BLOB,
           preview_expires_at INTEGER
         );",
    )?;
    Ok(conn)
}

fn encode_png(image: &RgbaImage) -> Result<Vec<u8>> {
    let mut output = Vec::new();
    PngEncoder::new(&mut output).write_image(
        image.as_raw(),
        image.width(),
        image.height(),
        ExtendedColorType::Rgba8,
    )?;
    Ok(output)
}

fn encode_key(key: &str) -> String {
    let mut encoded = String::new();
    for byte in key.bytes() {
        if byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.' | b'~' | b'/') {
            encoded.push(byte as char);
        } else {
            encoded.push_str(&format!("%{byte:02X}"));
        }
    }
    encoded
}

fn sha256_hex(bytes: &[u8]) -> String {
    hex(&Sha256::digest(bytes))
}

fn hmac(key: &[u8], bytes: &[u8]) -> Result<Vec<u8>> {
    let mut mac = HmacSha256::new_from_slice(key).map_err(|_| anyhow!("invalid signing key"))?;
    mac.update(bytes);
    Ok(mac.finalize().into_bytes().to_vec())
}

fn hex(bytes: &[u8]) -> String {
    const DIGITS: &[u8; 16] = b"0123456789abcdef";
    let mut result = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        result.push(DIGITS[(byte >> 4) as usize] as char);
        result.push(DIGITS[(byte & 0xf) as usize] as char);
    }
    result
}

#[cfg(test)]
mod tests {
    use super::{
        default_cache_ttl_hours, delete_object, encode_key, load_config, put_object, secret,
    };

    #[test]
    fn object_keys_keep_path_separators_and_escape_spaces() {
        assert_eq!(encode_key("screenshots/a b.png"), "screenshots/a%20b.png");
    }

    #[test]
    fn preview_cache_defaults_to_one_day() {
        assert_eq!(default_cache_ttl_hours(), 24);
    }

    #[test]
    #[ignore = "uses the configured DigitalOcean Space"]
    fn live_spaces_signing_round_trip() {
        let config = load_config().unwrap();
        let access_key = secret("access-key-id").unwrap();
        let secret_key = secret("secret-access-key").unwrap();
        let key = "screenshots/wayscrollshot-signing-test.txt";
        put_object(&config, &access_key, &secret_key, key, b"wayscrollshot").unwrap();
        delete_object(&config, &access_key, &secret_key, key).unwrap();
    }
}
