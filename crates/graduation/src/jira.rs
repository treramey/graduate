//! Validated Jira domain contracts shared by every delivery path.

use thiserror::Error;
use url::Url;

/// Input field associated with a Jira validation failure.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum JiraField {
    Site,
    AtlassianEmail,
    AtlassianToken,
    AccountId,
}

/// Pure validation failures for Jira domain values.
#[derive(Clone, Debug, Error, PartialEq, Eq)]
pub enum JiraValidationError {
    #[error("Jira site is required")]
    SiteRequired,
    #[error("Jira site must be a valid hostname or HTTPS URL")]
    SiteInvalid,
    #[error("Jira site must be a bare hostname or complete HTTPS URL")]
    SiteMustBeHostnameOrHttpsUrl,
    #[error("Jira site must use HTTPS without credentials or a custom port")]
    SiteMustUseHttps,
    #[error("Jira site must contain a valid hostname")]
    SiteHostnameInvalid,
    #[error("Atlassian email is required")]
    AtlassianEmailRequired,
    #[error("Atlassian email contains control characters")]
    AtlassianEmailUnsafe,
    #[error("Atlassian API token is required")]
    AtlassianTokenRequired,
    #[error("Jira identity contained an empty account ID")]
    AccountIdRequired,
}

impl JiraValidationError {
    /// Identify the input responsible for this failure.
    #[must_use]
    pub const fn field(&self) -> JiraField {
        match self {
            Self::SiteRequired
            | Self::SiteInvalid
            | Self::SiteMustBeHostnameOrHttpsUrl
            | Self::SiteMustUseHttps
            | Self::SiteHostnameInvalid => JiraField::Site,
            Self::AtlassianEmailRequired | Self::AtlassianEmailUnsafe => JiraField::AtlassianEmail,
            Self::AtlassianTokenRequired => JiraField::AtlassianToken,
            Self::AccountIdRequired => JiraField::AccountId,
        }
    }
}

/// A normalized Jira Cloud hostname.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct JiraSite(String);

impl JiraSite {
    /// Parse a bare hostname or HTTPS URL into a normalized hostname.
    pub fn parse(input: &str) -> Result<Self, JiraValidationError> {
        let input = input.trim();
        if input.is_empty() {
            return Err(JiraValidationError::SiteRequired);
        }

        let url = if input.contains("://") {
            Url::parse(input).map_err(|_| JiraValidationError::SiteInvalid)?
        } else {
            if input.contains(['/', '?', '#', '@', ':']) {
                return Err(JiraValidationError::SiteMustBeHostnameOrHttpsUrl);
            }
            Url::parse(&format!("https://{input}")).map_err(|_| JiraValidationError::SiteInvalid)?
        };

        if url.scheme() != "https"
            || !url.username().is_empty()
            || url.password().is_some()
            || url.port().is_some()
        {
            return Err(JiraValidationError::SiteMustUseHttps);
        }
        let domain = match url.host() {
            Some(url::Host::Domain(domain)) => domain,
            _ => return Err(JiraValidationError::SiteHostnameInvalid),
        };
        if !valid_domain(domain) {
            return Err(JiraValidationError::SiteHostnameInvalid);
        }
        Ok(Self(domain.to_owned()))
    }

    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

fn valid_domain(domain: &str) -> bool {
    domain.split('.').all(|label| {
        !label.is_empty()
            && label
                .chars()
                .all(|character| character.is_ascii_alphanumeric() || character == '-')
            && label
                .chars()
                .next()
                .is_some_and(|character| character.is_ascii_alphanumeric())
            && label
                .chars()
                .last()
                .is_some_and(|character| character.is_ascii_alphanumeric())
    })
}

/// A normalized Atlassian account email.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AtlassianEmail(String);

impl AtlassianEmail {
    pub fn parse(input: &str) -> Result<Self, JiraValidationError> {
        let input = input.trim();
        if input.is_empty() {
            return Err(JiraValidationError::AtlassianEmailRequired);
        }
        if input.chars().any(char::is_control) {
            return Err(JiraValidationError::AtlassianEmailUnsafe);
        }
        Ok(Self(input.to_owned()))
    }

    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// An Atlassian API token. This type intentionally does not implement `Debug`.
#[derive(Clone, PartialEq, Eq)]
pub struct AtlassianToken(String);

impl AtlassianToken {
    pub fn parse(input: &str) -> Result<Self, JiraValidationError> {
        let input = input.trim();
        if input.is_empty() {
            return Err(JiraValidationError::AtlassianTokenRequired);
        }
        Ok(Self(input.to_owned()))
    }

    #[must_use]
    pub fn expose_secret(&self) -> &str {
        &self.0
    }
}

/// Validated, non-secret Jira account details.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct JiraAccountDetails {
    site: JiraSite,
    email: AtlassianEmail,
}

impl JiraAccountDetails {
    pub fn parse(site: &str, email: &str) -> Result<Self, JiraValidationError> {
        Ok(Self {
            site: JiraSite::parse(site)?,
            email: AtlassianEmail::parse(email)?,
        })
    }

    #[must_use]
    pub const fn site(&self) -> &JiraSite {
        &self.site
    }

    #[must_use]
    pub const fn email(&self) -> &AtlassianEmail {
        &self.email
    }
}

/// Validated Jira credentials. This type intentionally does not implement `Debug`.
#[derive(Clone, PartialEq, Eq)]
pub struct JiraCredentials {
    details: JiraAccountDetails,
    token: AtlassianToken,
}

impl JiraCredentials {
    pub fn parse(site: &str, email: &str, token: &str) -> Result<Self, JiraValidationError> {
        Ok(Self {
            details: JiraAccountDetails::parse(site, email)?,
            token: AtlassianToken::parse(token)?,
        })
    }

    #[must_use]
    pub const fn from_parts(details: JiraAccountDetails, token: AtlassianToken) -> Self {
        Self { details, token }
    }

    #[must_use]
    pub const fn site(&self) -> &JiraSite {
        self.details.site()
    }

    #[must_use]
    pub const fn email(&self) -> &AtlassianEmail {
        self.details.email()
    }

    #[must_use]
    pub const fn token(&self) -> &AtlassianToken {
        &self.token
    }
}

/// Identity returned after Jira verifies credentials.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct JiraIdentity {
    account_id: String,
    display_name: String,
}

impl JiraIdentity {
    pub fn new(
        account_id: impl Into<String>,
        display_name: impl Into<String>,
    ) -> Result<Self, JiraValidationError> {
        let account_id = account_id.into();
        let account_id = account_id.trim();
        if account_id.is_empty() {
            return Err(JiraValidationError::AccountIdRequired);
        }
        let display_name = display_name.into();
        Ok(Self {
            account_id: account_id.to_owned(),
            display_name: display_name.trim().to_owned(),
        })
    }

    #[must_use]
    pub fn account_id(&self) -> &str {
        &self.account_id
    }

    #[must_use]
    pub fn display_name(&self) -> &str {
        &self.display_name
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn jira_urls_are_normalized_to_lowercase_hostnames() -> Result<(), JiraValidationError> {
        assert_eq!(
            JiraSite::parse(" https://Example.atlassian.net/jira ")?.as_str(),
            "example.atlassian.net"
        );
        Ok(())
    }

    #[test]
    fn credentials_validate_every_domain_input() {
        let result = JiraCredentials::parse("example.atlassian.net", "person@example.com", "  ");
        assert!(matches!(
            result,
            Err(JiraValidationError::AtlassianTokenRequired)
        ));
    }

    #[test]
    fn identity_requires_an_account_id() {
        let result = JiraIdentity::new("  ", "Person");
        assert!(matches!(
            result,
            Err(JiraValidationError::AccountIdRequired)
        ));
    }
}
