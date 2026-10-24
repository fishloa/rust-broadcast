# Cookies for broadcast-client sessions (RFC 6265)

Verbatim from `specs/rfc6265_http_state_management.txt` (plain-text RFC, no OCR; page header/footer lines removed). Binding source for issue #1458.

## RFC 6265 section 4.2 Cookie (request header)

```text
4.2.  Cookie

4.2.1.  Syntax

   The user agent sends stored cookies to the origin server in the
   Cookie header.  If the server conforms to the requirements in
   Section 4.1 (and the user agent conforms to the requirements in
   Section 5), the user agent will send a Cookie header that conforms to
   the following grammar:

   cookie-header = "Cookie:" OWS cookie-string OWS
   cookie-string = cookie-pair *( ";" SP cookie-pair )

4.2.2.  Semantics

   Each cookie-pair represents a cookie stored by the user agent.  The
   cookie-pair contains the cookie-name and cookie-value the user agent
   received in the Set-Cookie header.

   Notice that the cookie attributes are not returned.  In particular,
   the server cannot determine from the Cookie header alone when a
   cookie will expire, for which hosts the cookie is valid, for which
   paths the cookie is valid, or whether the cookie was set with the
   Secure or HttpOnly attributes.

   The semantics of individual cookies in the Cookie header are not
   defined by this document.  Servers are expected to imbue these
   cookies with application-specific semantics.

   Although cookies are serialized linearly in the Cookie header,
   servers SHOULD NOT rely upon the serialization order.  In particular,
   if the Cookie header contains two cookies with the same name (e.g.,
   that were set with different Path or Domain attributes), servers
   SHOULD NOT rely upon the order in which these cookies appear in the
   header.
```

## RFC 6265 section 5.1.3 Domain Matching and 5.1.4 Paths and Path-Match

```text
5.1.3.  Domain Matching

   A string domain-matches a given domain string if at least one of the
   following conditions hold:

   o  The domain string and the string are identical.  (Note that both
      the domain string and the string will have been canonicalized to
      lower case at this point.)

   o  All of the following conditions hold:

      *  The domain string is a suffix of the string.

      *  The last character of the string that is not included in the
         domain string is a %x2E (".") character.

      *  The string is a host name (i.e., not an IP address).

5.1.4.  Paths and Path-Match

   The user agent MUST use an algorithm equivalent to the following
   algorithm to compute the default-path of a cookie:

   1.  Let uri-path be the path portion of the request-uri if such a
       portion exists (and empty otherwise).  For example, if the
       request-uri contains just a path (and optional query string),
       then the uri-path is that path (without the %x3F ("?") character
       or query string), and if the request-uri contains a full
       absoluteURI, the uri-path is the path component of that URI.

   2.  If the uri-path is empty or if the first character of the uri-
       path is not a %x2F ("/") character, output %x2F ("/") and skip
       the remaining steps.

   3.  If the uri-path contains no more than one %x2F ("/") character,
       output %x2F ("/") and skip the remaining step.

   4.  Output the characters of the uri-path from the first character up
       to, but not including, the right-most %x2F ("/").

   A request-path path-matches a given cookie-path if at least one of
   the following conditions holds:

   o  The cookie-path and the request-path are identical.

   o  The cookie-path is a prefix of the request-path, and the last
      character of the cookie-path is %x2F ("/").

   o  The cookie-path is a prefix of the request-path, and the first
      character of the request-path that is not included in the cookie-
      path is a %x2F ("/") character.
```

## RFC 6265 section 5.2 The Set-Cookie Header (parsing algorithm)

```text
5.2.  The Set-Cookie Header

   When a user agent receives a Set-Cookie header field in an HTTP
   response, the user agent MAY ignore the Set-Cookie header field in
   its entirety.  For example, the user agent might wish to block
   responses to "third-party" requests from setting cookies (see
   Section 7.1).

   If the user agent does not ignore the Set-Cookie header field in its
   entirety, the user agent MUST parse the field-value of the Set-Cookie
   header field as a set-cookie-string (defined below).

   NOTE: The algorithm below is more permissive than the grammar in
   Section 4.1.  For example, the algorithm strips leading and trailing
   whitespace from the cookie name and value (but maintains internal
   whitespace), whereas the grammar in Section 4.1 forbids whitespace in
   these positions.  User agents use this algorithm so as to
   interoperate with servers that do not follow the recommendations in
   Section 4.

   A user agent MUST use an algorithm equivalent to the following
   algorithm to parse a "set-cookie-string":

   1.  If the set-cookie-string contains a %x3B (";") character:

          The name-value-pair string consists of the characters up to,
          but not including, the first %x3B (";"), and the unparsed-
          attributes consist of the remainder of the set-cookie-string
          (including the %x3B (";") in question).

       Otherwise:

          The name-value-pair string consists of all the characters
          contained in the set-cookie-string, and the unparsed-
          attributes is the empty string.

   2.  If the name-value-pair string lacks a %x3D ("=") character,
       ignore the set-cookie-string entirely.

   3.  The (possibly empty) name string consists of the characters up
       to, but not including, the first %x3D ("=") character, and the
       (possibly empty) value string consists of the characters after
       the first %x3D ("=") character.

   4.  Remove any leading or trailing WSP characters from the name
       string and the value string.

   5.  If the name string is empty, ignore the set-cookie-string
       entirely.

   6.  The cookie-name is the name string, and the cookie-value is the
       value string.

   The user agent MUST use an algorithm equivalent to the following
   algorithm to parse the unparsed-attributes:

   1.  If the unparsed-attributes string is empty, skip the rest of
       these steps.

   2.  Discard the first character of the unparsed-attributes (which
       will be a %x3B (";") character).

   3.  If the remaining unparsed-attributes contains a %x3B (";")
       character:

          Consume the characters of the unparsed-attributes up to, but
          not including, the first %x3B (";") character.

       Otherwise:

          Consume the remainder of the unparsed-attributes.

       Let the cookie-av string be the characters consumed in this step.

   4.  If the cookie-av string contains a %x3D ("=") character:

          The (possibly empty) attribute-name string consists of the
          characters up to, but not including, the first %x3D ("=")
          character, and the (possibly empty) attribute-value string
          consists of the characters after the first %x3D ("=")
          character.

       Otherwise:

          The attribute-name string consists of the entire cookie-av
          string, and the attribute-value string is empty.

   5.  Remove any leading or trailing WSP characters from the attribute-
       name string and the attribute-value string.

   6.  Process the attribute-name and attribute-value according to the
       requirements in the following subsections.  (Notice that
       attributes with unrecognized attribute-names are ignored.)

   7.  Return to Step 1 of this algorithm.

   When the user agent finishes parsing the set-cookie-string, the user
   agent is said to "receive a cookie" from the request-uri with name
   cookie-name, value cookie-value, and attributes cookie-attribute-
   list.  (See Section 5.3 for additional requirements triggered by
   receiving a cookie.)
```

## RFC 6265 section 5.2.2 Max-Age, 5.2.4 Path, 5.2.5 Secure

```text
5.2.2.  The Max-Age Attribute

   If the attribute-name case-insensitively matches the string "Max-
   Age", the user agent MUST process the cookie-av as follows.

   If the first character of the attribute-value is not a DIGIT or a "-"
   character, ignore the cookie-av.

   If the remainder of attribute-value contains a non-DIGIT character,
   ignore the cookie-av.

   Let delta-seconds be the attribute-value converted to an integer.

   If delta-seconds is less than or equal to zero (0), let expiry-time
   be the earliest representable date and time.  Otherwise, let the
   expiry-time be the current date and time plus delta-seconds seconds.

   Append an attribute to the cookie-attribute-list with an attribute-
   name of Max-Age and an attribute-value of expiry-time.
```

## RFC 6265 section 5.2.4 Path / 5.2.5 Secure attributes

```text
5.2.4.  The Path Attribute

   If the attribute-name case-insensitively matches the string "Path",
   the user agent MUST process the cookie-av as follows.

   If the attribute-value is empty or if the first character of the
   attribute-value is not %x2F ("/"):

      Let cookie-path be the default-path.

   Otherwise:

      Let cookie-path be the attribute-value.

   Append an attribute to the cookie-attribute-list with an attribute-
   name of Path and an attribute-value of cookie-path.

5.2.5.  The Secure Attribute

   If the attribute-name case-insensitively matches the string "Secure",
   the user agent MUST append an attribute to the cookie-attribute-list
   with an attribute-name of Secure and an empty attribute-value.
```

## RFC 6265 section 5.3 Storage Model

```text
5.3.  Storage Model

   The user agent stores the following fields about each cookie: name,
   value, expiry-time, domain, path, creation-time, last-access-time,
   persistent-flag, host-only-flag, secure-only-flag, and http-only-
   flag.

   When the user agent "receives a cookie" from a request-uri with name
   cookie-name, value cookie-value, and attributes cookie-attribute-
   list, the user agent MUST process the cookie as follows:

   1.   A user agent MAY ignore a received cookie in its entirety.  For
        example, the user agent might wish to block receiving cookies
        from "third-party" responses or the user agent might not wish to
        store cookies that exceed some size.

   2.   Create a new cookie with name cookie-name, value cookie-value.
        Set the creation-time and the last-access-time to the current
        date and time.

   3.   If the cookie-attribute-list contains an attribute with an
        attribute-name of "Max-Age":

           Set the cookie's persistent-flag to true.

           Set the cookie's expiry-time to attribute-value of the last
           attribute in the cookie-attribute-list with an attribute-name
           of "Max-Age".

        Otherwise, if the cookie-attribute-list contains an attribute
        with an attribute-name of "Expires" (and does not contain an
        attribute with an attribute-name of "Max-Age"):

           Set the cookie's persistent-flag to true.

           Set the cookie's expiry-time to attribute-value of the last
           attribute in the cookie-attribute-list with an attribute-name
           of "Expires".

        Otherwise:

           Set the cookie's persistent-flag to false.

           Set the cookie's expiry-time to the latest representable
           date.

   4.   If the cookie-attribute-list contains an attribute with an
        attribute-name of "Domain":

           Let the domain-attribute be the attribute-value of the last
           attribute in the cookie-attribute-list with an attribute-name
           of "Domain".

        Otherwise:

           Let the domain-attribute be the empty string.

   5.   If the user agent is configured to reject "public suffixes" and
        the domain-attribute is a public suffix:

           If the domain-attribute is identical to the canonicalized
           request-host:

              Let the domain-attribute be the empty string.

           Otherwise:

              Ignore the cookie entirely and abort these steps.

           NOTE: A "public suffix" is a domain that is controlled by a
           public registry, such as "com", "co.uk", and "pvt.k12.wy.us".
           This step is essential for preventing attacker.com from
           disrupting the integrity of example.com by setting a cookie
           with a Domain attribute of "com".  Unfortunately, the set of
           public suffixes (also known as "registry controlled domains")
           changes over time.  If feasible, user agents SHOULD use an
           up-to-date public suffix list, such as the one maintained by
           the Mozilla project at <http://publicsuffix.org/>.

   6.   If the domain-attribute is non-empty:

           If the canonicalized request-host does not domain-match the
           domain-attribute:

              Ignore the cookie entirely and abort these steps.

           Otherwise:

              Set the cookie's host-only-flag to false.

              Set the cookie's domain to the domain-attribute.

        Otherwise:

           Set the cookie's host-only-flag to true.

           Set the cookie's domain to the canonicalized request-host.

   7.   If the cookie-attribute-list contains an attribute with an
        attribute-name of "Path", set the cookie's path to attribute-
        value of the last attribute in the cookie-attribute-list with an
        attribute-name of "Path".  Otherwise, set the cookie's path to
        the default-path of the request-uri.

   8.   If the cookie-attribute-list contains an attribute with an
        attribute-name of "Secure", set the cookie's secure-only-flag to
        true.  Otherwise, set the cookie's secure-only-flag to false.

   9.   If the cookie-attribute-list contains an attribute with an
        attribute-name of "HttpOnly", set the cookie's http-only-flag to
        true.  Otherwise, set the cookie's http-only-flag to false.

   10.  If the cookie was received from a "non-HTTP" API and the
        cookie's http-only-flag is set, abort these steps and ignore the
        cookie entirely.

   11.  If the cookie store contains a cookie with the same name,
        domain, and path as the newly created cookie:

        1.  Let old-cookie be the existing cookie with the same name,
            domain, and path as the newly created cookie.  (Notice that
            this algorithm maintains the invariant that there is at most
            one such cookie.)

        2.  If the newly created cookie was received from a "non-HTTP"
            API and the old-cookie's http-only-flag is set, abort these
            steps and ignore the newly created cookie entirely.

        3.  Update the creation-time of the newly created cookie to
            match the creation-time of the old-cookie.

        4.  Remove the old-cookie from the cookie store.

   12.  Insert the newly created cookie into the cookie store.

   A cookie is "expired" if the cookie has an expiry date in the past.

   The user agent MUST evict all expired cookies from the cookie store
   if, at any time, an expired cookie exists in the cookie store.

   At any time, the user agent MAY "remove excess cookies" from the
   cookie store if the number of cookies sharing a domain field exceeds
   some implementation-defined upper bound (such as 50 cookies).

   At any time, the user agent MAY "remove excess cookies" from the
   cookie store if the cookie store exceeds some predetermined upper
   bound (such as 3000 cookies).

   When the user agent removes excess cookies from the cookie store, the
   user agent MUST evict cookies in the following priority order:

   1.  Expired cookies.

   2.  Cookies that share a domain field with more than a predetermined
       number of other cookies.

   3.  All cookies.

   If two cookies have the same removal priority, the user agent MUST
   evict the cookie with the earliest last-access date first.

   When "the current session is over" (as defined by the user agent),
   the user agent MUST remove from the cookie store all cookies with the
   persistent-flag set to false.
```

## RFC 6265 section 5.4 The Cookie Header (user agent)

```text
5.4.  The Cookie Header

   The user agent includes stored cookies in the Cookie HTTP request
   header.

   When the user agent generates an HTTP request, the user agent MUST
   NOT attach more than one Cookie header field.

   A user agent MAY omit the Cookie header in its entirety.  For
   example, the user agent might wish to block sending cookies during
   "third-party" requests from setting cookies (see Section 7.1).

   If the user agent does attach a Cookie header field to an HTTP
   request, the user agent MUST send the cookie-string (defined below)
   as the value of the header field.

   The user agent MUST use an algorithm equivalent to the following
   algorithm to compute the "cookie-string" from a cookie store and a
   request-uri:

   1.  Let cookie-list be the set of cookies from the cookie store that
       meets all of the following requirements:

       *  Either:

             The cookie's host-only-flag is true and the canonicalized
             request-host is identical to the cookie's domain.

          Or:

             The cookie's host-only-flag is false and the canonicalized
             request-host domain-matches the cookie's domain.

       *  The request-uri's path path-matches the cookie's path.

       *  If the cookie's secure-only-flag is true, then the request-
          uri's scheme must denote a "secure" protocol (as defined by
          the user agent).

             NOTE: The notion of a "secure" protocol is not defined by
             this document.  Typically, user agents consider a protocol
             secure if the protocol makes use of transport-layer

             security, such as SSL or TLS.  For example, most user
             agents consider "https" to be a scheme that denotes a
             secure protocol.

       *  If the cookie's http-only-flag is true, then exclude the
          cookie if the cookie-string is being generated for a "non-
          HTTP" API (as defined by the user agent).

   2.  The user agent SHOULD sort the cookie-list in the following
       order:

       *  Cookies with longer paths are listed before cookies with
          shorter paths.

       *  Among cookies that have equal-length path fields, cookies with
          earlier creation-times are listed before cookies with later
          creation-times.

       NOTE: Not all user agents sort the cookie-list in this order, but
       this order reflects common practice when this document was
       written, and, historically, there have been servers that
       (erroneously) depended on this order.

   3.  Update the last-access-time of each cookie in the cookie-list to
       the current date and time.

   4.  Serialize the cookie-list into a cookie-string by processing each
       cookie in the cookie-list in order:

       1.  Output the cookie's name, the %x3D ("=") character, and the
           cookie's value.

       2.  If there is an unprocessed cookie in the cookie-list, output
           the characters %x3B and %x20 ("; ").

   NOTE: Despite its name, the cookie-string is actually a sequence of
   octets, not a sequence of characters.  To convert the cookie-string
   (or components thereof) into a sequence of characters (e.g., for
   presentation to the user), the user agent might wish to try using the
   UTF-8 character encoding [RFC3629] to decode the octet sequence.
   This decoding might fail, however, because not every sequence of
   octets is valid UTF-8.
```

## Decisions for the first slice (safe subset, documented deviations)

- **Jar scoped to the session ORIGIN; matching by `cookie_store`.** Only cookies set by responses from the session origin are stored, and a cookie is sent only to requests whose origin (scheme+host+port) equals the session origin, so no cookie ever reaches a sibling subdomain. Domain-match (section 5.1.3) and path-match/default-path (section 5.1.4) are `cookie_store::CookieDomain` / `CookiePath` (pure; `default-features = false`, no wall clock, no public-suffix list). No `Domain` = host-only (identical host). A `Domain` attribute is accepted only if the origin host domain-matches it (section 5.3 step 6): a `Domain` naming another host, a child host, a non-suffix, an invalid IDNA label, a trailing-dot name, a suffix of an IP literal or an IPv6 literal is REFUSED (the old jar ignored `Domain` and stored every such cookie). A leading dot and case are normalised (`Domain=.Example.COM` on `example.com` is accepted). A public-suffix `Domain` (`com`, `co.uk`) on a host under it is still accepted, as the old jar did: the `publicsuffix` crate ships no list (the caller must supply the data), and the cookie stays bound to the session origin, so nothing can leak; `public_suffix` is therefore not enabled. `cookie_store`'s `Cookie`, `CookieStore` and `is_expired` are never used (they read the wall clock); `tests/no_handroll_guard.rs` bans them.
- `Path`: default-path (section 5.1.4) and path-match applied. `Secure`: sent only on https requests. `HttpOnly`/`SameSite`: irrelevant to a non-browser client, ignored.
- Expiry: `Expires` is IGNORED (the session clock is a monotonic `now_us`, not a calendar, so an absolute date has nothing to be compared with); `Max-Age` is honoured relative to the session `now_us` (Max-Age <= 0 deletes immediately; otherwise the cookie expires `max_age` seconds after it was stored). Session cookies live until the Session is dropped. Max-Age wins over Expires per section 5.3 step 3 anyway.
- Hardening beyond RFC 6265 (RFC 6265bis behaviour): a cookie with `Secure` received from an `http` origin is rejected; `__Secure-` (ASCII case-insensitive) requires `Secure`; `__Host-` requires `Secure`, an explicit `Path=/` and no `Domain`; a `Path` longer than 1024 bytes rejects the cookie. `Config::initial_cookies` names must be RFC 7230 tokens.
- Replacement (section 5.3 step 11): same name+domain+path replaces the old cookie. The `Cookie` header orders by longest path first then earliest storage order (section 5.4 step 2); `name=value` pairs joined with `; `. Limits (section 6.1): at least 4096 bytes per cookie, 50 cookies per origin; beyond that oldest evicted.
- Parsing the `Set-Cookie` line uses the `cookie` crate (no hand-rolled parser). Several `Set-Cookie` header lines per response are all processed.
- API: `Config::initial_cookies: Vec<(String, String)>` (sent to the origin, path `/`), `Session::cookies() -> Vec<(String, String)>` for the player to persist and re-seed (values are secrets: never in Debug, never logged). The jar applies to EVERY request of the session to the origin (playlists, segments, keys, reconnects, redirects within the origin); a redirect to another origin gets no cookies.
- Custom headers: `Config::extra_headers` are sent ONLY to requests on the session origin (never to a redirect/CDN on another origin); names `Authorization`, `Cookie`, `Host`, `Content-Length`, `Transfer-Encoding`, `Connection`, `Range` are rejected at `Session::open` with `Error::ReservedHeader{name}` (the session owns them); names/values validated as HTTP field names/values (no CR/LF) with the `http` crate types.
