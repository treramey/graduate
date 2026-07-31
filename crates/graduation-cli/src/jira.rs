//! Minimal Jira Cloud boundary used by login and future ticket queries.

use futures_util::StreamExt;
use graduation::jira::{JiraCredentials, JiraIdentity};
use reqwest::{Client, Method, Request, Response, StatusCode};
use serde::Deserialize;
use url::Url;

use crate::error::CliError;

const MAX_IDENTITY_BYTES: usize = 64 * 1024;

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
            .user_agent(concat!("grad/", env!("CARGO_PKG_VERSION")))
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
        let bytes = read_bounded_identity(response).await?;
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

async fn read_bounded_identity(response: Response) -> Result<Vec<u8>, CliError> {
    if response
        .content_length()
        .is_some_and(|length| length > MAX_IDENTITY_BYTES as u64)
    {
        return Err(identity_too_large());
    }
    let capacity = response
        .content_length()
        .and_then(|length| usize::try_from(length).ok())
        .unwrap_or(0)
        .min(MAX_IDENTITY_BYTES);
    let mut body = Vec::with_capacity(capacity);
    let mut stream = response.bytes_stream();
    while let Some(chunk) = stream.next().await {
        append_bounded(&mut body, &chunk?)?;
    }
    Ok(body)
}

fn append_bounded(body: &mut Vec<u8>, chunk: &[u8]) -> Result<(), CliError> {
    if chunk.len() > MAX_IDENTITY_BYTES.saturating_sub(body.len()) {
        return Err(identity_too_large());
    }
    body.extend_from_slice(chunk);
    Ok(())
}

fn identity_too_large() -> CliError {
    CliError::JiraResponse(format!(
        "identity exceeded the {MAX_IDENTITY_BYTES}-byte limit"
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
    fn bounded_identity_body_rejects_data_before_extending_past_the_limit() -> Result<(), CliError>
    {
        let mut body = vec![b'a'; MAX_IDENTITY_BYTES];

        let error = append_bounded(&mut body, b"b").err();

        assert!(matches!(error, Some(CliError::JiraResponse(_))));
        assert_eq!(body.len(), MAX_IDENTITY_BYTES);
        Ok(())
    }
}
