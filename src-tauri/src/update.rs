//! Update check against GitHub.

use serde::{Deserialize, Serialize};

use crate::error::{Result, WalletError};

const REPO: &str = "Nodropedd/ShinyFlakes";

pub const DOWNLOAD_PAGE: &str = "https://nodropedd.github.io/ShinyFlakes/";

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdateInfo {

    pub current: String,

    pub latest: String,

    pub available: bool,
}

#[derive(Deserialize)]
struct Release {
    tag_name: String,
}

pub fn current() -> &'static str {
    env!("CARGO_PKG_VERSION")
}

fn parse(version: &str) -> Option<Vec<u64>> {
    let core = version
        .trim()
        .trim_start_matches(['v', 'V'])
        .split(['-', '+'])
        .next()?;
    core.split('.').map(|part| part.parse().ok()).collect()
}

pub fn is_newer(latest: &str, current: &str) -> bool {
    match (parse(latest), parse(current)) {
        (Some(mut a), Some(mut b)) => {
            let len = a.len().max(b.len());
            a.resize(len, 0);
            b.resize(len, 0);
            a > b
        }
        _ => false,
    }
}

pub async fn check() -> Result<UpdateInfo> {
    let net = |e: reqwest::Error| WalletError::Network(format!("update check: {e}"));

    let release: Release = crate::chains::rpc::client()?
        .get(format!("https://api.github.com/repos/{REPO}/releases/latest"))
        .header("Accept", "application/vnd.github+json")
        .send()
        .await
        .map_err(net)?
        .error_for_status()
        .map_err(net)?
        .json()
        .await
        .map_err(net)?;

    let latest = release.tag_name.trim_start_matches(['v', 'V']).to_string();
    Ok(UpdateInfo {
        available: is_newer(&latest, current()),
        current: current().to_string(),
        latest,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn compares_versions_numerically() {
        assert!(is_newer("0.1.1", "0.1.0"));
        assert!(is_newer("v0.2.0", "0.1.9"));
        assert!(is_newer("0.10.0", "0.9.0"));
        assert!(is_newer("1.0", "0.9.9"));
        assert!(!is_newer("0.1.0", "0.1.0"));
        assert!(!is_newer("v0.1.0", "0.1.1"));
        assert!(!is_newer("0.1", "0.1.0"));
    }

    #[test]
    fn ignores_pre_release_and_build_suffixes() {
        assert!(!is_newer("0.1.1-beta", "0.1.1"));
        assert!(is_newer("0.2.0+build.7", "0.1.9"));
    }

    #[test]
    fn a_malformed_tag_is_never_an_update() {
        assert!(!is_newer("latest", "0.1.0"));
        assert!(!is_newer("", "0.1.0"));
        assert!(!is_newer("v1.x", "0.1.0"));
    }

    #[tokio::test]
    #[ignore]
    async fn live_release_check() {
        let info = check().await.expect("GitHub answered");
        println!("{info:?}");
        assert!(parse(&info.latest).is_some());
    }
}
