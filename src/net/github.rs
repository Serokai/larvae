//! GitHub releases API (the slice `self update` needs)

use anyhow::Result;
use serde::Deserialize;

#[derive(Debug, Deserialize)]
pub struct Release {
    pub tag_name: String,
    pub assets: Vec<Asset>,
}

#[derive(Debug, Deserialize)]
pub struct Asset {
    pub name: String,
    pub browser_download_url: String,
}

/// Latest non prerelease of owner/repo, honors GITHUB_TOKEN
pub fn latest_release(repo: &str) -> Result<Release> {
    let url = format!("https://api.github.com/repos/{repo}/releases/latest");
    let token = std::env::var("GITHUB_TOKEN").ok();
    let auth = token.map(|t| format!("Bearer {t}"));
    let mut headers: Vec<(&str, &str)> = vec![("Accept", "application/vnd.github.v3+json")];
    if let Some(auth) = &auth {
        headers.push(("Authorization", auth));
    }
    crate::net::http::get_json(&url, &headers)
}
