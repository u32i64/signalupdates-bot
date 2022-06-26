use anyhow::{anyhow, bail, Context};
use semver::Version;
use serde::de::DeserializeOwned;
use serde_json::Value;
use sha2::{Digest, Sha256};
use worker::{
    console_log, wasm_bindgen::JsValue, Env, Fetch, Headers, Method, Request, RequestInit,
    Response, Url,
};

use crate::{
    localization_change::LocalizationChange, platform::Platform, types::github::Comparison,
};

pub const USER_AGENT: &str = "updates-bot";

pub fn version_from_tag(tag: &str) -> anyhow::Result<Version> {
    lenient_semver::parse(tag)
        .map_err(|e| anyhow!(e.to_string()))
        .context("could not parse version from tag")
}

pub fn exact_version_string_from_tag(tag: &str) -> String {
    tag.replace('v', "")
}

#[derive(Debug)]
enum StringBindingKind {
    Secret,
    Var,
}
use StringBindingKind::*;

fn get_env_string(env: &Env, kind: StringBindingKind, name: &str) -> anyhow::Result<String> {
    let string_binding = match kind {
        Secret => env.secret(name),
        Var => env.var(name),
    }
    .map_err(|e| anyhow!(e.to_string()))
    .with_context(|| anyhow!("couldn't get string binding kind = {kind:?}, name = {name}"))?;

    JsValue::from(string_binding)
        .as_string()
        .ok_or_else(|| anyhow!("couldn't get value of string binding"))
}

pub fn api_key(env: &Env) -> anyhow::Result<String> {
    get_env_string(env, Secret, "DISCOURSE_API_KEY")
}

pub fn topic_id_override(env: &Env) -> anyhow::Result<Option<u64>> {
    get_env_string(env, Var, "TOPIC_ID_OVERRIDE").map(|string| string.parse().ok())
}

pub async fn get_topic_id(
    api_key: String,
    platform: Platform,
    version: &Version,
) -> anyhow::Result<Option<u64>> {
    console_log!("getting topic id for version {version}");

    let url =
        Url::parse(&platform.discourse_topic_slug_url(version)).context("could not parse URL")?;

    let request = create_request(url, Method::Get, None, Some(api_key))?;
    let response: crate::types::discourse::TopicResponse = get_json_from_request(request).await?;

    match response.post_stream.posts.first() {
        Some(post) => Ok(Some(post.topic_id)),
        None => {
            console_log!("no posts in topic");
            Ok(None)
        }
    }
}

pub async fn get_json_from_url<T: DeserializeOwned>(url: impl Into<String>) -> anyhow::Result<T> {
    let url = Url::parse(&url.into()).context("could not parse URL")?;
    let request = create_request(url, Method::Get, None, None)?;
    json_from_configuration(Fetch::Request(request)).await
}

pub async fn get_json_from_request<T: DeserializeOwned>(request: Request) -> anyhow::Result<T> {
    json_from_configuration(Fetch::Request(request)).await
}

async fn json_from_configuration<T: DeserializeOwned>(configuration: Fetch) -> anyhow::Result<T> {
    let mut response = fetch(configuration).await?;
    json_from_response(&mut response).await
}

async fn fetch(configuration: Fetch) -> anyhow::Result<Response> {
    let result = configuration
        .send()
        .await
        .map_err(|e| anyhow!(e.to_string()))
        .context("could not fetch");

    if let Ok(response) = &result {
        console_log!("response.status_code() = {}", response.status_code());
    }

    result
}

async fn json_from_response<T: DeserializeOwned>(response: &mut Response) -> anyhow::Result<T> {
    response
        .json()
        .await
        .map_err(|e| anyhow!(e.to_string()))
        .context("could not get JSON")
}

pub fn create_request(
    url: Url,
    method: Method,
    body: Option<Value>,
    discourse_api_key: Option<String>,
) -> anyhow::Result<Request> {
    console_log!("constructing request for url {url}");

    let mut headers = Headers::new();

    if let Some(api_key) = discourse_api_key {
        headers.set("User-Api-Key", &api_key).unwrap();
    }

    headers.set("Content-Type", "application/json").unwrap();
    headers.set("Accept", "application/json").unwrap();
    headers.set("User-Agent", USER_AGENT).unwrap();

    let mut request_init = RequestInit::new();
    request_init.with_method(method).with_headers(headers);

    if let Some(body) = body {
        request_init.with_body(Some(JsValue::from_str(&body.to_string())));
    }

    Request::new_with_init(url.as_ref(), &request_init)
        .map_err(|e| anyhow!(e.to_string()))
        .context("could not create request")
}

#[derive(Debug)]
pub enum GitHubComparisonKind {
    /// Full comparison including all commits and files.
    Full,
    /// A comparison that only includes 1 commit, but includes all files.
    /// `total_commits` still indicates the total number of commits in the comparison.
    JustAllFiles,
}
use GitHubComparisonKind::*;

pub async fn get_github_comparison(
    kind: GitHubComparisonKind,
    platform: Platform,
    old_tag: &str,
    new_tag: &str,
) -> anyhow::Result<Comparison> {
    console_log!("getting comparison kind = {kind:?}");

    let mut page = 1;
    let per_page = match kind {
        Full => 100,
        JustAllFiles => 1,
    };

    let mut url_string = platform.github_api_comparison_url(old_tag, new_tag, page, per_page);

    let mut total_commits;
    let mut commits = vec![];
    let mut files = vec![];

    loop {
        console_log!("getting page = {page}, url = {url_string}");

        let url = Url::parse(&url_string).context("could not parse URL")?;
        let request = create_request(url, Method::Get, None, None)?;

        let mut response = fetch(Fetch::Request(request))
            .await
            .context("could not fetch comparison from GitHub")?;

        let mut comparison_part: Comparison = json_from_response(&mut response)
            .await
            .context("could not get JSON for comparison part")?;

        total_commits = comparison_part.total_commits; // always the total number of commits
        commits.append(&mut comparison_part.commits);
        if let Some(part_files) = &mut comparison_part.files {
            files.append(part_files);
        }

        if let JustAllFiles = kind {
            // All files are on the first page according to
            // https://docs.github.com/en/rest/commits/commits#compare-two-commits
            console_log!("just all files were requested, which are all on the first page, done");
            break;
        }

        let link_header_string = response
            .headers()
            .get("Link")
            .unwrap()
            .ok_or_else(|| anyhow!("no `Link` header in GitHub's response"))?;
        let link_header = parse_link_header::parse_with_rel(&link_header_string)
            .context("could not parse `Link` header")?;

        match link_header.get("next") {
            Some(link) => {
                url_string = link.raw_uri.clone();
                page += 1;
            }
            None => {
                console_log!("no `next` link, done getting full comparison");
                break;
            }
        }
    }

    if let Full = kind {
        if total_commits != commits.len() {
            bail!(
                "incomplete full comparison: total_commits = {total_commits} but commits.len() = {}, commits = {commits:?}",
                commits.len()
            )
        };
    }

    Ok(Comparison {
        total_commits,
        commits,
        files: Some(files),
    })
}

pub fn localization_changes_from_comparison(
    platform: Platform,
    comparison: &Comparison,
) -> Vec<LocalizationChange> {
    let mut changes = comparison
        .files
        .clone()
        .unwrap()
        .iter()
        .filter_map(|file| platform.localization_change(&file.filename))
        .collect::<Vec<_>>();

    changes.sort_unstable();

    changes
}

pub fn sha256_string(input: &str) -> String {
    let result = Sha256::digest(input.as_bytes());
    base16ct::lower::encode_string(&result)
}

#[cfg(test)]
mod tests {
    use super::*;
    use test_case::test_case;

    fn test_version(pre: Option<&str>, build: Option<&str>) -> Version {
        use semver::{BuildMetadata, Prerelease};

        Version {
            major: 1,
            minor: 2,
            patch: 3,
            pre: match pre {
                Some(pre) => Prerelease::new(pre).unwrap(),
                None => Prerelease::EMPTY,
            },
            build: match build {
                Some(build) => BuildMetadata::new(build).unwrap(),
                None => BuildMetadata::EMPTY,
            },
        }
    }

    #[test_case("v1.2.3" => test_version(None, None); "3 digits with v")]
    #[test_case("1.2.3" => test_version(None, None); "3 digits without v")]
    #[test_case("v1.2.3.4" => test_version(None, Some("4")); "4 digits with v")]
    #[test_case("1.2.3.4" => test_version(None, Some("4")); "4 digits without v")]
    #[test_case("v1.2.3-beta.1" => test_version(Some("beta.1"), None); "3 digits beta with v")]
    #[test_case("1.2.3.4-beta" => test_version(Some("beta"), Some("4")); "4 digits beta without v")]
    fn version_from_tag(tag: &str) -> Version {
        super::version_from_tag(tag).unwrap()
    }
}
