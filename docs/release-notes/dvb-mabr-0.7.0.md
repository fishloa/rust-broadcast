# dvb-mabr 0.7.0

_Released 2026-11-22._

Minor for a 0.x crate. No API change of its own: the crate builds against `broadcast-common` 14 (the jiff 1.0 epoch), which is the breaking part. The work is the audit-22 test pass over the session-configuration XML reader.

## Breaking changes

- Builds against `broadcast-common` 14 (its `jiff` 1.0 epoch). A caret-epoch change is major-class under the epoch-purity rule (`docs/RELEASE-AUDIT.md` §2) whatever this crate's own API diff; a consumer must resolve a single `broadcast-common` 14.

## Tests

- Every `StructureError`, `ServiceComponentIdentifier` and `BaselineNamespace` variant's `name()`/`Display` label is asserted against its documented token (audit 22 #3047).
- The "first singleton child wins, unknown or reordered siblings are skipped" guards are pinned for `ForwardErrorCorrectionParameters`, `EndpointAddress`, `MulticastTransportSession`, each gateway transport-session child, and the `xsi:type` component selector (which must ignore a plain `type` and any other `xsi:*` attribute) (audit 22 #3046).
- All three `@transportSecurity` values (`none`, `integrity`, `integrityAndAuthenticity`) parse to their typed variant and round-trip, not only the value the fixtures happen to carry (audit 22 #3045).
- The root-element identity check is pinned: a gateway document is refused by the server entry point (and vice versa), and right-name/foreign-namespace and right-namespace/wrong-name roots are each rejected (audit 22 #3044).
- The XML envelope's trailing-comment/PI allowance, `MultipleRoots`, `CharDataOutsideRoot` and the `drain` precedence rule are pinned with hostile input (audit 22 #3043).
- Every DOCTYPE refusal site is pinned with hostile input (inside a skipped subtree, between children, inside modelled text, and after a semantic error via `drain`) (audit 22 #3042).

## Pre-release audit

This crate carries no crate-specific finding; it takes the workspace-wide release-documentation issues #4004 (release note) and #4008 (epoch entry).

---

Published from tag `dvb-mabr-v0.7.0`.
