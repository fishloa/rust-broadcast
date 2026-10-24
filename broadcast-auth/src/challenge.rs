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
#[non_exhaustive]
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

const RESOURCE_METADATA_SCHEME: &str = "https";

/// Maximum number of challenges accepted from one [`parse_challenges`] call.
pub const MAX_CHALLENGES: usize = 16;

/// Outcome of [`parse_challenges`]: every challenge that parsed, plus one
/// error per challenge (or header line) that did not.
///
/// The `token68` form (`Bearer abc123==`, RFC 9110 §11.6.1) is not supported
/// by the underlying `http-auth` tokeniser and is reported in `errors`.
#[non_exhaustive]
#[derive(Debug, Default)]
pub struct ParsedChallenges {
    /// Challenges that parsed, in input order.
    pub challenges: Vec<Challenge>,
    /// Errors for the challenges that did not.
    pub errors: Vec<Error>,
}

/// Parse one or more `WWW-Authenticate` header lines into challenges.
///
/// A malformed challenge never discards the valid ones around it. After
/// [`MAX_CHALLENGES`] challenges, [`Error::TooManyChallenges`] is recorded and
/// the rest are ignored.
pub fn parse_challenges(lines: &[&str]) -> ParsedChallenges {
    let mut out = ParsedChallenges::default();
    for line in lines {
        if line.trim().is_empty() {
            continue;
        }
        for item in ChallengeParser::new(line) {
            if out.challenges.len() >= MAX_CHALLENGES {
                out.errors.push(Error::TooManyChallenges {
                    max: MAX_CHALLENGES,
                });
                return out;
            }
            match item
                .map_err(|e| Error::ChallengeParse(e.to_string()))
                .and_then(convert)
            {
                Ok(c) => out.challenges.push(c),
                Err(e) => out.errors.push(e),
            }
        }
    }
    out
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
                let url = url::Url::parse(&val)
                    .ok()
                    .filter(|u| u.scheme() == RESOURCE_METADATA_SCHEME)
                    .ok_or(Error::InvalidResourceMetadata)?;
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
