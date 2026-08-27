//! Minimal Jira Cloud boundary used by login and future ticket queries.

use futures_util::StreamExt;
use graduate::jira::{JiraCredentials, JiraIdentity};
use graduate::promotion::JiraIssueSummary;
use reqwest::{Client, Method, Request, Response, StatusCode};
use serde::Deserialize;
use url::Url;

use crate::shared::error::CliError;

const MAX_RESPONSE_BYTES: usize = 64 * 1024;

pub(crate) struct JiraClient {
    client: Client,
    base: Url,
}

impl JiraClient {
    pub(crate) fn new(credentials: &JiraCredentials) -> Result<Self, CliError> {
        let base = Url::parse(&format!(
            "https://{}/rest/api/3/",
            credentials.site().as_str()
        ))?;
        Self::with_base(base)
    }

    fn with_base(base: Url) -> Result<Self, CliError> {
        let client = Client::builder()
            .user_agent(concat!("graduate/", env!("CARGO_PKG_VERSION")))
            .connect_timeout(std::time::Duration::from_secs(5))
            .timeout(std::time::Duration::from_secs(15))
            .build()?;
        Ok(Self { client, base })
    }

    pub(crate) async fn verify(
        &self,
        credentials: &JiraCredentials,
    ) -> Result<JiraIdentity, CliError> {
        let request = self.verification_request(credentials)?;
        let response = self.client.execute(request).await?;
        let status = response.status();
        if matches!(status, StatusCode::UNAUTHORIZED | StatusCode::FORBIDDEN) {
            return Err(CliError::Authentication);
        }
        if !status.is_success() {
            return Err(CliError::JiraStatus(status.as_u16()));
        }
        let bytes = read_bounded_response(response).await?;
        #[derive(Deserialize)]
        #[serde(rename_all = "camelCase")]
        struct CurrentUser {
            account_id: String,
            #[serde(default)]
            display_name: String,
        }
        let current: CurrentUser = serde_json::from_slice(&bytes)?;
        JiraIdentity::new(current.account_id, current.display_name)
            .map_err(|error| CliError::JiraResponse(error.to_string()))
    }

    pub(crate) async fn issue(
        &self,
        credentials: &JiraCredentials,
        key: &str,
    ) -> Result<JiraIssueSummary, CliError> {
        let request = self.issue_request(credentials, key)?;
        let response = self.client.execute(request).await?;
        let status = response.status();
        if matches!(status, StatusCode::UNAUTHORIZED | StatusCode::FORBIDDEN) {
            return Err(CliError::Authentication);
        }
        if !status.is_success() {
            return Err(CliError::JiraStatus(status.as_u16()));
        }
        #[derive(Deserialize)]
        #[serde(rename_all = "camelCase")]
        struct Issue {
            #[serde(rename = "self")]
            api_url: String,
            fields: Fields,
        }
        #[derive(Deserialize)]
        #[serde(rename_all = "camelCase")]
        struct Fields {
            #[serde(default)]
            summary: String,
            status: NamedValue,
            assignee: Option<Assignee>,
            #[serde(default)]
            fix_versions: Vec<NamedValue>,
        }
        #[derive(Deserialize)]
        #[serde(rename_all = "camelCase")]
        struct NamedValue {
            name: String,
        }
        #[derive(Deserialize)]
        #[serde(rename_all = "camelCase")]
        struct Assignee {
            display_name: String,
        }

        let bytes = read_bounded_response(response).await?;
        let issue: Issue = serde_json::from_slice(&bytes)?;
        Ok(JiraIssueSummary {
            url: format!("https://{}/browse/{}", credentials.site().as_str(), key),
            key: key.to_owned(),
            api_url: issue.api_url,
            summary: issue.fields.summary,
            status: issue.fields.status.name,
            assignee: issue.fields.assignee.map(|assignee| assignee.display_name),
            fix_versions: issue
                .fields
                .fix_versions
                .into_iter()
                .map(|version| version.name)
                .collect(),
        })
    }

    fn issue_request(&self, credentials: &JiraCredentials, key: &str) -> Result<Request, CliError> {
        let url = self.base.join(&format!("issue/{key}"))?;
        self.client
            .get(url)
            .query(&[("fields", "summary,status,assignee,fixVersions")])
            .basic_auth(
                credentials.email().as_str(),
                Some(credentials.token().expose_secret()),
            )
            .build()
            .map_err(CliError::from)
    }

    fn verification_request(&self, credentials: &JiraCredentials) -> Result<Request, CliError> {
        let url = self.base.join("myself")?;
        self.client
            .request(Method::GET, url)
            .basic_auth(
                credentials.email().as_str(),
                Some(credentials.token().expose_secret()),
            )
            .build()
            .map_err(CliError::from)
    }
}

async fn read_bounded_response(response: Response) -> Result<Vec<u8>, CliError> {
    if response
        .content_length()
        .is_some_and(|length| length > MAX_RESPONSE_BYTES as u64)
    {
        return Err(response_too_large());
    }
    let capacity = response
        .content_length()
        .and_then(|length| usize::try_from(length).ok())
        .unwrap_or(0)
        .min(MAX_RESPONSE_BYTES);
    let mut body = Vec::with_capacity(capacity);
    let mut stream = response.bytes_stream();
    while let Some(chunk) = stream.next().await {
        append_bounded(&mut body, &chunk?)?;
    }
    Ok(body)
}

fn append_bounded(body: &mut Vec<u8>, chunk: &[u8]) -> Result<(), CliError> {
    if chunk.len() > MAX_RESPONSE_BYTES.saturating_sub(body.len()) {
        return Err(response_too_large());
    }
    body.extend_from_slice(chunk);
    Ok(())
}

fn response_too_large() -> CliError {
    CliError::JiraResponse(format!(
        "response exceeded the {MAX_RESPONSE_BYTES}-byte limit"
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn verification_is_a_read_only_jira_v3_request() -> Result<(), Box<dyn std::error::Error>> {
        let client =
            JiraClient::with_base(Url::parse("https://example.atlassian.net/rest/api/3/")?)?;
        let credentials =
            JiraCredentials::parse("example.atlassian.net", "person@example.com", "secret")?;
        let request = client.verification_request(&credentials)?;

        assert_eq!(request.method(), Method::GET);
        assert_eq!(
            request.url().as_str(),
            "https://example.atlassian.net/rest/api/3/myself"
        );
        assert!(request
            .headers()
            .contains_key(reqwest::header::AUTHORIZATION));
        Ok(())
    }

    #[test]
    fn bounded_response_body_rejects_data_before_extending_past_the_limit() -> Result<(), CliError>
    {
        let mut body = vec![b'a'; MAX_RESPONSE_BYTES];

        let error = append_bounded(&mut body, b"b").err();

        assert!(matches!(error, Some(CliError::JiraResponse(_))));
        assert_eq!(body.len(), MAX_RESPONSE_BYTES);
        Ok(())
    }

    #[test]
    fn issue_lookup_requests_only_promotion_fields() -> Result<(), Box<dyn std::error::Error>> {
        let client =
            JiraClient::with_base(Url::parse("https://example.atlassian.net/rest/api/3/")?)?;
        let credentials =
            JiraCredentials::parse("example.atlassian.net", "person@example.com", "secret")?;

        let request = client.issue_request(&credentials, "PROJ-123")?;

        assert_eq!(request.method(), Method::GET);
        assert_eq!(
            request.url().as_str(),
            "https://example.atlassian.net/rest/api/3/issue/PROJ-123?fields=summary%2Cstatus%2Cassignee%2CfixVersions"
        );
        assert!(request
            .headers()
            .contains_key(reqwest::header::AUTHORIZATION));
        Ok(())
    }
}
