use anyhow::Error;
use reqwest::header::HeaderValue;
use scraper::{Html, Selector};
use serde::{Deserialize, Serialize};
use std::ffi::OsString;
use std::hash::{Hash, Hasher};
use std::path::Path;
use std::str::{self, FromStr};
use std::{collections::hash_map::DefaultHasher, fmt};
use std::{fs, fs::File, io::Write, path::PathBuf};
use url::Url;

use crate::id::ResourceId;
use crate::meta::ResourceMeta;
const PREVIEWS_RELATIVE_PATH: &str = ".ark/previews";

#[derive(Debug, Deserialize, Serialize)]
pub struct Link {
    pub url: Url,
    pub metadata: Option<Metadata>,
}

#[derive(Debug, Deserialize, Serialize)]
pub struct Metadata {
    pub title: String,
    pub desc: String,
}

impl Link {
    pub fn new(title: String, desc: String, url: Url) -> Self {
        Self {
            url: url,
            metadata: Some(Metadata { title, desc }),
        }
    }
    /// Get formatted name for .link
    pub fn format_name(&self) -> String {
        let mut s = DefaultHasher::new();

        let url = self
            .url
            .to_string()
            .replace("http://", "")
            .replace("https://", "")
            .split(&['-', '?', '/'][..])
            .filter(|x| x != &"")
            .collect::<Vec<&str>>()
            .join("-");
        url.hash(&mut s);
        s.finish().to_string()
    }

    // Load the link json from the .link file.
    pub fn load_json<P: AsRef<Path>>(
        root: P,
        path: P,
    ) -> Result<String, Error> {
        let p = path.as_ref().to_path_buf();
        let mut link: Link = Link::from(p);

        let resource_id =
            ResourceId::compute_bytes(link.url.as_str().as_bytes());
        let json = serde_json::to_string(&link).unwrap();
        Ok(json)
    }

    /// Write zipped file to path
    pub async fn write_to_path<P: AsRef<Path>>(
        &mut self,
        root: P,
        path: P,
        save_preview: bool,
    ) {
        let resource_id =
            ResourceId::compute_bytes(self.url.as_str().as_bytes());
        let metadata_json = serde_json::to_string(&self.metadata).unwrap();
        let mut link_file = File::create(path.as_ref().to_owned()).unwrap();
        link_file
            .write(self.url.as_str().as_bytes())
            .unwrap();
        if save_preview {
            let preview_data = Link::get_preview(self.url.clone())
                .await
                .unwrap_or_default();
            let image_data = preview_data
                .fetch_image()
                .await
                .unwrap_or_default();
            self.save_preview(
                root.as_ref().to_owned(),
                image_data,
                resource_id.to_owned(),
            )
            .await;
        }
        ResourceMeta::store(
            root.as_ref().to_owned(),
            path.as_ref().to_owned(),
            resource_id.to_owned(),
            Some(OsString::from("name")),
            Some(OsString::from("link")),
            Some(metadata_json.into_bytes()),
            None,
        );
    }

    /// Synchronized version of Write zipped file to path
    pub fn write_to_path_sync<P: AsRef<Path>>(
        &mut self,
        root: P,
        path: P,
        save_preview: bool,
    ) {
        let runtime =
            tokio::runtime::Runtime::new().expect("Unable to create a runtime");
        runtime.block_on(self.write_to_path(root, path, save_preview));
    }

    pub async fn save_preview<P: AsRef<Path>>(
        &mut self,
        root: P,
        image_data: Vec<u8>,
        resource_id: ResourceId,
    ) {
        let previews_path = root.as_ref().join(PREVIEWS_RELATIVE_PATH);
        fs::create_dir_all(previews_path.to_owned())
            .expect(&format!("Creating {} directory", PREVIEWS_RELATIVE_PATH));
        let mut preview_file = File::create(
            previews_path
                .to_owned()
                .join(format!("{}.png", resource_id.crc32)),
        )
        .unwrap();
        preview_file.write(image_data.as_slice()).unwrap();
    }

    /// Get metadata of the link (synced).
    pub fn get_preview_synced<S>(url: S) -> Result<OpenGraph, reqwest::Error>
    where
        S: Into<String>,
    {
        let runtime =
            tokio::runtime::Runtime::new().expect("Unable to create a runtime");
        return runtime.block_on(Link::get_preview(url));
    }

    /// Get metadata of the link.
    pub async fn get_preview<S>(url: S) -> Result<OpenGraph, reqwest::Error>
    where
        S: Into<String>,
    {
        let mut header = reqwest::header::HeaderMap::new();
        header.insert(
            "User-Agent",
            HeaderValue::from_static(
                "Mozilla/5.0 (X11; Linux x86_64; rv:102.0) Gecko/20100101 Firefox/102.0",
            ),
        );
        let client = reqwest::Client::builder()
            .default_headers(header)
            .build()
            .unwrap();
        let scraper = client
            .get(url.into())
            .send()
            .await?
            .text()
            .await?;
        let html = Html::parse_document(&scraper.as_str());
        let title =
            select_og(&html, OpenGraphTag::Title).or(select_title(&html));
        Ok(OpenGraph {
            title,
            description: select_og(&html, OpenGraphTag::Description)
                .or(select_desc(&html)),
            url: select_og(&html, OpenGraphTag::Url),
            image: select_og(&html, OpenGraphTag::Image),
            object_type: select_og(&html, OpenGraphTag::Type),
            locale: select_og(&html, OpenGraphTag::Locale),
        })
    }
}

fn select_og(html: &Html, tag: OpenGraphTag) -> Option<String> {
    let selector =
        Selector::parse(&format!("meta[property=\"og:{}\"]", tag.as_str()))
            .unwrap();

    if let Some(element) = html.select(&selector).next() {
        if let Some(value) = element.value().attr("content") {
            return Some(value.to_string());
        }
    }

    None
}
fn select_desc(html: &Html) -> Option<String> {
    let selector = Selector::parse("meta[name=\"description\"]").unwrap();

    if let Some(element) = html.select(&selector).next() {
        if let Some(value) = element.value().attr("content") {
            return Some(value.to_string());
        }
    }

    None
}
fn select_title(html: &Html) -> Option<String> {
    let selector = Selector::parse("title").unwrap();
    if let Some(element) = html.select(&selector).next() {
        return element.text().next().map(|x| x.to_string());
    }

    None
}
#[derive(Debug, Serialize, Deserialize, Default)]
pub struct OpenGraph {
    /// Represents the "og:title" OpenGraph meta tag.
    ///
    /// The title of your object as it should appear within
    /// the graph, e.g., "The Rock".
    pub title: Option<String>,
    /// Represents the "og:description" OpenGraph meta tag
    pub description: Option<String>,
    /// Represents the "og:url" OpenGraph meta tag
    pub url: Option<String>,
    /// Represents the "og:image" OpenGraph meta tag
    pub image: Option<String>,
    /// Represents the "og:type" OpenGraph meta tag
    ///
    /// The type of your object, e.g., "video.movie". Depending on the type
    /// you specify, other properties may also be required.
    object_type: Option<String>,
    /// Represents the "og:locale" OpenGraph meta tag
    locale: Option<String>,
}
impl OpenGraph {
    pub async fn fetch_image(&self) -> Option<Vec<u8>> {
        if let Some(url) = &self.image {
            let res = reqwest::get(url).await.unwrap();
            Some(res.bytes().await.unwrap().to_vec())
        } else {
            None
        }
    }
}
/// OpenGraphTag meta tags collection
pub enum OpenGraphTag {
    /// Represents the "og:title" OpenGraph meta tag.
    ///
    /// The title of your object as it should appear within
    /// the graph, e.g., "The Rock".
    Title,
    /// Represents the "og:url" OpenGraph meta tag
    Url,
    /// Represents the "og:image" OpenGraph meta tag
    Image,
    /// Represents the "og:type" OpenGraph meta tag
    ///
    /// The type of your object, e.g., "video.movie". Depending on the type
    /// you specify, other properties may also be required.
    Type,
    /// Represents the "og:description" OpenGraph meta tag
    Description,
    /// Represents the "og:locale" OpenGraph meta tag
    Locale,
    /// Represents the "og:image:height" OpenGraph meta tag
    ImageHeight,
    /// Represents the "og:image:width" OpenGraph meta tag
    ImageWidth,
    /// Represents the "og:site_name" OpenGraph meta tag
    SiteName,
}

impl fmt::Debug for OpenGraphTag {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl OpenGraphTag {
    fn as_str(&self) -> &str {
        match self {
            OpenGraphTag::Title => "title",
            OpenGraphTag::Url => "url",
            OpenGraphTag::Image => "image",
            OpenGraphTag::Type => "type",
            OpenGraphTag::Description => "description",
            OpenGraphTag::Locale => "locale",
            OpenGraphTag::ImageHeight => "image:height",
            OpenGraphTag::ImageWidth => "image:width",
            OpenGraphTag::SiteName => "site_name",
        }
    }
}

impl From<PathBuf> for Link {
    fn from(path: PathBuf) -> Self {
        let url_raw = std::fs::read(path).expect("Open link file");
        let url_str = str::from_utf8(url_raw.as_slice()).unwrap();
        let url: Url = Url::from_str(url_str).unwrap();
        Self {
            url: url,
            metadata: None,
        }
    }
}

#[test]
fn test_create_link_file() {
    use tempdir::TempDir;
    let dir = TempDir::new("arklib_test").unwrap();
    let tmp_path = dir.path();
    println!("temp path: {}", tmp_path.display());
    let url = Url::parse("https://example.com/").unwrap();
    let mut link = Link::new(String::from("title"), String::from("desc"), url);
    let hash = link.format_name();
    assert_eq!(hash, "5257664237369877164");
    let link_file_path = tmp_path.join(format!("{}.link", hash));
    for save_preview in [false, true] {
        link.write_to_path_sync(
            tmp_path,
            link_file_path.as_path(),
            save_preview,
        );
        let link_file_bytes = std::fs::read(link_file_path.to_owned()).unwrap();
        let u: Url =
            Url::from_str(str::from_utf8(&link_file_bytes).unwrap()).unwrap();
        assert_eq!(u.as_str(), "https://example.com/");
        let link = Link::load_json(tmp_path.clone(), link_file_path.as_path())
            .unwrap();
        let link_obj: Link = serde_json::from_str(link.as_str()).unwrap();
        assert_eq!(link_obj.url.as_str(), u.as_str());
        let meta = link_obj.metadata.unwrap();
        assert_eq!(meta.desc, "desc");
        assert_eq!(meta.title, "title");

        let resource_id = ResourceId::compute_bytes(link_file_bytes.as_slice());
        println!("resource: {}, {}", resource_id.crc32, resource_id.data_size);

        if Path::new(tmp_path)
            .join(PREVIEWS_RELATIVE_PATH)
            .join(format!("{}.png", resource_id.crc32))
            .exists()
        {
            assert_eq!(save_preview, true)
        } else {
            assert_eq!(save_preview, false)
        }
        let metadata = ResourceMeta::locate(tmp_path.clone(), resource_id);
        let j: Metadata =
            serde_json::from_slice(metadata.unwrap().kind.unwrap().as_slice())
                .unwrap();
        assert_eq!(j.title, meta.title);
        assert_eq!(j.desc, meta.desc);
    }
}