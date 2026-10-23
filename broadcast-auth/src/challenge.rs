//! Typed `WWW-Authenticate` challenge parsing (RFC 9110 §11.6.1, RFC 6750
//! §§3/3.1, RFC 9728 §§5.1/5.2/5.4).
//!
//! Tokenisation uses `http-auth`'s [`http_auth::parser::ChallengeParser`]; no
//! hand-rolled lexer.

use http_auth::parser::ChallengeParser;

use crate::error::{Error, Result};

/// Auth scheme of a challenge (RFC 9110 §11.1: ASCII case-insensitive).
#[non_exhaustive]
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Scheme {
    /// `Basic`.
    Basic,
    /// `Digest`.
    Digest,
    /// `Bearer` (RFC 6750).
    Bearer,
    /// Any other scheme, as written.
    Other(String),
}

impl Scheme {
    /// Build from a scheme token (ASCII case-insensitive).
    pub fn from_token(token: &str) -> Self {
        if token.eq_ignore_ascii_case("Basic") {
            Self::Basic
        } else if token.eq_ignore_ascii_case("Digest") {
            Self::Digest
        } else if token.eq_ignore_ascii_case("Bearer") {
            Self::Bearer
        } else {
            Self::Other(token.to_string())
        }
    }

    /// The scheme token.
    pub fn name(&self) -> &str {
        match self {
            Self::Basic => "Basic",
            Self::Digest => "Digest",
            Self::Bearer => "Bearer",
            Self::Other(s) => s,
        }
    }
}

impl core::fmt::Display for Scheme {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str(self.name())
    }
}

/// Bearer error code (RFC 6750 §3.1).
#[non_exhaustive]
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BearerError {
    /// `invalid_request`.
    InvalidRequest,
    /// `invalid_token`.
    InvalidToken,
    /// `insufficient_scope`.
    InsufficientScope,
    /// Any other (extension) code, as written.
    Other(String),
}

impl BearerError {
    /// Build from a code string (case-sensitive, per RFC 6750).
    pub fn from_code(code: &str) -> Self {
        match code {
            "invalid_request" => Self::InvalidRequest,
            "invalid_token" => Self::InvalidToken,
            "insufficient_scope" => Self::InsufficientScope,
            _ => Self::Other(code.to_string()),
        }
    }

    /// The code string.
    pub fn name(&self) -> &str {
        match self {
            Self::InvalidRequest => "invalid_request",
            Self::InvalidToken => "invalid_token",
            Self::InsufficientScope => "insufficient_scope",
            Self::Other(s) => s,
        }
    }
}

impl core::fmt::Display for BearerError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str(self.name())
    }
}

/// One parsed `WWW-Authenticate` challenge.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Challenge {
    /// Auth scheme.
    pub scheme: Scheme,
    /// `realm` parameter, if present.
    pub realm: Option<String>,
    /// `scope` parameter (space-delimited), if present.
    pub scope: Option<String>,
    /// Bearer `error` code, if present.
    pub error: Option<BearerError>,
    /// `error_description`, if present.
    pub error_description: Option<String>,
    /// `error_uri`, if present.
    pub error_uri: Option<String>,
    /// `resource_metadata` URL (RFC 9728), if present.
    pub resource_metadata: Option<url::Url>,
    /// Parameters not modelled above, in input order.
    pub other_params: Vec<(String, String)>,
}

/// Parse one or more `WWW-Authenticate` header lines into challenges.
pub fn parse_challenges(lines: &[&str]) -> Result<Vec<Challenge>> {
    let mut out = Vec::new();
    for line in lines {
        if line.trim().is_empty() {
            continue;
        }
        let parser = ChallengeParser::new(line);
        for item in parser {
            let cr = item.map_err(|e| Error::ChallengeParse(e.to_string()))?;
            out.push(convert(cr)?);
        }
    }
    Ok(out)
}

fn convert(cr: http_auth::ChallengeRef<'_>) -> Result<Challenge> {
    let scheme = Scheme::from_token(cr.scheme);

    let mut realm: Option<String> = None;
    let mut scope: Option<String> = None;
    let mut error: Option<BearerError> = None;
    let mut error_description: Option<String> = None;
    let mut error_uri: Option<String> = None;
    let mut resource_metadata: Option<url::Url> = None;
    let mut other_params: Vec<(String, String)> = Vec::new();

    for (name, value) in &cr.params {
        let key = name.to_ascii_lowercase();
        let val = value.to_unescaped();
        match key.as_str() {
            "realm" => {
                if realm.is_some() {
                    return Err(Error::DuplicateParameter { name: key });
                }
                realm = Some(val);
            }
            "scope" => {
                if scope.is_some() {
                    return Err(Error::DuplicateParameter { name: key });
                }
                scope = Some(val);
            }
            "error" => {
                if error.is_some() {
                    return Err(Error::DuplicateParameter { name: key });
                }
                error = Some(BearerError::from_code(&val));
            }
            "error_description" => {
                if error_description.is_some() {
                    return Err(Error::DuplicateParameter { name: key });
                }
                error_description = Some(val);
            }
            "error_uri" => {
                if error_uri.is_some() {
                    return Err(Error::DuplicateParameter { name: key });
                }
                error_uri = Some(val);
            }
            "resource_metadata" => {
                if resource_metadata.is_some() {
                    return Err(Error::DuplicateParameter { name: key });
                }
                let url = url::Url::parse(&val).map_err(|_| Error::InvalidResourceMetadata)?;
                resource_metadata = Some(url);
            }
            _ => other_params.push((name.to_string(), val)),
        }
    }

    Ok(Challenge {
        scheme,
        realm,
        scope,
        error,
        error_description,
        error_uri,
        resource_metadata,
        other_params,
    })
}
