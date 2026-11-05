# Provenance

## DVB-MABR session configuration fixtures (`fixtures/mabr/`)

| File | Source | Date | Licence |
|---|---|---|---|
| `annex-c1-server-config.xml` | Hand-authored `MulticastServerConfiguration` fixture, modeled on the *shape* of ETSI TS 103 769 V1.2.1 Annex C.1's worked example (two gateway-configuration transport sessions FLUTE+ROUTE, two linear services with DASH+HLS manifests, video+audio transport sessions, weighted unicast-repair `BaseURL`s, in-band object carousel, macro expansion) but original content throughout — not a transcription of the Annex C.1 text. Issue #755. | 2026-08 | Own work — MIT OR Apache-2.0 |
| `annex-c2-gateway-bootstrap.xml` | Hand-authored `MulticastGatewayConfiguration` "bootstrap" fixture (clause 10.4.5 method 3): only `MulticastGatewayConfigurationTransportSession`, no `MulticastSession`. Modeled on the shape of Annex C.2. Issue #755. | 2026-08 | Own work — MIT OR Apache-2.0 |
| `annex-c3-gateway-config.xml` | Hand-authored full `MulticastGatewayConfiguration` fixture: unicast-retrieval manifest locators including the empty-locator/`contentPlaybackPathPattern`-mandatory case, and the gateway-referencing object carousel (`@serviceIdRef`/`@transportSessionIdRef`). Modeled on the shape of Annex C.3. Issue #755. | 2026-08 | Own work — MIT OR Apache-2.0 |

### Excluded worked examples

| Source | Reason |
|---|---|
| ETSI TS 103 769 V1.2.1 Annex C.1/C.2/C.3 (the spec's own worked examples) | ETSI retains copyright over redistribution of the deliverable text ("No part may be reproduced or utilized... except as authorized by written permission of ETSI"). Not vendored in `private/` either (see `private/specs-verified/dvb/dvb-mabr/` "Source"). Replaced by the workspace's own fixtures above, modeled on the same *shape* (element/attribute coverage) but with original content. |
