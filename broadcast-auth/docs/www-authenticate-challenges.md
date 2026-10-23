# WWW-Authenticate challenges: Bearer + resource_metadata

Transcribed verbatim from the text RFCs committed in `specs/` (plain-text sources, no OCR):
`rfc9110.txt` section 11.6.1, `rfc6750_bearer_token_usage.txt` sections 3 and 3.1,
`rfc9728_oauth_protected_resource_metadata.txt` sections 5.1, 5.2, 5.4. Quotes are exact.

## RFC 9110 section 11.6.1 WWW-Authenticate

> WWW-Authenticate = #challenge
> A server generating a 401 (Unauthorized) response MUST send a WWW-Authenticate header field containing at least one challenge.
> User agents are advised to take special care in parsing the field value, as it might contain more than one challenge, and each challenge can contain a comma-separated list of authentication parameters. Furthermore, the header field itself can occur multiple times.
> `WWW-Authenticate: Basic realm="simple", Newauth realm="apps", type=1, title="Login to \"apps\""`
> This header field contains two challenges, one for the "Basic" scheme with a realm value of "simple" and another for the "Newauth" scheme with a realm value of "apps". It also contains two additional parameters, "type" and "title".

Consequence: parse N header lines x M challenges per line; parameter values may be quoted-strings with backslash escapes; unknown schemes/params are preserved, never errors.

## RFC 6750 section 3 (Bearer challenge)

- All challenges defined by this specification MUST use the auth-scheme value "Bearer". This scheme MUST be followed by one or more auth-param values.
- `realm` MAY be included; "The 'realm' attribute MUST NOT appear more than once."
- `scope`: space-delimited list of case-sensitive scope values; OPTIONAL; "MUST NOT appear more than once". Characters limited to %x21 / %x23-5B / %x5D-7E plus %x20 delimiters.
- If the request included an access token and failed authentication, the server SHOULD include `error`; MAY include `error_description` and `error_uri`. "The 'error', 'error_description', and 'error_uri' attributes MUST NOT appear more than once."
- Example: `WWW-Authenticate: Bearer realm="example"` (no credentials supplied: SHOULD NOT include error info).
- Example: `WWW-Authenticate: Bearer realm="example", error="invalid_token", error_description="The access token expired"`.

### Section 3.1 error codes
- `invalid_request`: SHOULD respond 400.
- `invalid_token`: expired/revoked/malformed/invalid; SHOULD respond 401; client MAY request a new access token and retry.
- `insufficient_scope`: SHOULD respond 403; MAY include `scope` with the scope needed.
- Other codes may appear (extensions); preserve unknown values.

## RFC 9728 section 5.1 WWW-Authenticate Response

> resource_metadata: The URL of the protected resource metadata.
> `HTTP/1.1 401 Unauthorized` / `WWW-Authenticate: Bearer resource_metadata="https://resource.example.com/.well-known/oauth-protected-resource"`
> This parameter MAY also be used in WWW-Authenticate responses using authorization schemes other than "Bearer" [RFC6750], such as the DPoP scheme defined by [RFC9449].
> The resource_metadata parameter MAY be combined with other parameters defined in other extensions, such as the max_age parameter defined by [RFC9470].

Section 5.2: a later challenge with a `resource_metadata` value means the metadata may have changed; the client SHOULD re-fetch. Section 5.4: servers MAY return other WWW-Authenticate headers with other schemes (client chooses its preferred scheme). The spec does not require the metadata URL itself to be https (only the resource identifier, section 2); the parser requires an absolute URL and leaves policy to the caller.

## Typed model decided for broadcast-auth

`challenge::parse_challenges(lines: &[&str]) -> Result<Vec<Challenge>>`;
`Challenge { scheme: Scheme(Basic|Digest|Bearer|Other(String)), realm, scope, error: Option<BearerError(InvalidRequest|InvalidToken|InsufficientScope|Other(String))>, error_description, error_uri, resource_metadata: Option<url::Url>, other_params: Vec<(String,String)> }`.
Scheme match is ASCII case-insensitive (RFC 9110 section 11.1); parameter names case-insensitive. A parameter that RFC 6750 says MUST NOT repeat (realm, scope, error, error_description, error_uri) or that is resource_metadata appearing twice in one challenge is a typed error `DuplicateParameter{name}` for Bearer challenges. `resource_metadata` that is not an absolute URL: typed error `InvalidResourceMetadata`. Parsing uses `http-auth` `ChallengeParser` for tokenisation (no hand-rolled lexer).
