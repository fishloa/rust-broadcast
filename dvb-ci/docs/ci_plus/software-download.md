# Software Download / Download Resource

_Source: ETSI TS 101 699 V1.1.1 §6.7, Tables 74–83 (PDF pp. 64–74), render-verified_

The download resource provides a framework within which manufacturer-specific firmware-loading protocols can be implemented: a CI module acts as a source of firmware updates to a host. The host is the DSM-CC client and the module is the download server; the file transfer is based on the DSM-CC (ISO/IEC 13818-6) User-Network Download protocol (non-flow-controlled scenario, optionally flow-controlled).

The download module provides a **Download resource** with resource identifier **`0x00051041`** (§6.7.3; note the §6.7.3 prose prints `0x000510041`, an evident typo — the canonical value from the master resource registry Table 87 is `0x00051041`).

The life cycle has four distinct phases (§6.7.2):
1. determine if the update is required;
2. get user authorization;
3. download the file from the module to the host;
4. verification of code before use (encoding of signatures / linking information is a matter for manufacturer development, not specified here).

Four APDU objects are defined (§6.7.4): **Download Enquiry** (`0x9F8000`) and **Download Reply** (`0x9F8001`), which encapsulate certain DSM-CC User-to-Network messages, plus **User Authorization Initiate** (`0x9F8002`) and **User Authorization Result** (`0x9F8003`), which help the host determine that user authorization has been given.

---

## §6.7.3.1 — Identification of manufacturer binaries (Table 74, p. 66)

The description of binaries is encoded on 7 bytes. In DSM-CC messages these values are carried in the corresponding fields of the compatibility descriptor.

| Field | No. of bits | Mnemonic |
|-------|-------------|----------|
| specifier | 24 | bslbf |
| model | 16 | bslbf |
| version | 16 | bslbf |

Field notes:
- `specifier` — a 24-bit IEEE OUI obtained by the manufacturer from the IEEE.
- `model` — 16-bit value with semantics specified by the organization identified by `specifier`; distinguishes between various models defined by the organization.
- `version` — 16-bit value with semantics specified by the organization identified by `specifier`; distinguishes between different versions of a model.

---

## §6.7.4.1 — Download Enquiry object (Table 75, p. 67)

apdu_tag `download_enq_tag` = `0x9F 80 00`, Direction host ---> app.

Download Enquiry is used by the host to send DSM-CC messages to the module. Messages supported: download info request; download data request; download cancel.

| Syntax | No. of bits | Mnemonic |
|--------|-------------|----------|
| `download_enq() {` | | |
| &nbsp;&nbsp;download_enq_tag | 24 | uimsbf |
| &nbsp;&nbsp;length_field() | | |
| &nbsp;&nbsp;`for (i=0; i<n; i++) {` | | |
| &nbsp;&nbsp;&nbsp;&nbsp;DSMCC_descriptor() | 8 | uimsbf |
| &nbsp;&nbsp;`}` | | |
| `}` | | |

Field notes:
- `download_enq_tag` — 24-bit field with value `0x9F8000` identifying this message.
- `DSMCC_descriptor()` — the encapsulated DSM-CC message bytes (one of the messages above); walked byte-by-byte.

---

## §6.7.4.2 — Download Reply object (Table 76, p. 67)

apdu_tag `download_rep_tag` = `0x9F 80 01`, Direction app <--- host (module to host).

Download Reply is used by the module to send DSM-CC messages to the host. Messages supported: download info response; download data block; download cancel.

| Syntax | No. of bits | Mnemonic |
|--------|-------------|----------|
| `download_reply() {` | | |
| &nbsp;&nbsp;download_rep_tag | 24 | uimsbf |
| &nbsp;&nbsp;length_field() | | |
| &nbsp;&nbsp;`for (i=0; i<n; i++) {` | | |
| &nbsp;&nbsp;&nbsp;&nbsp;DSMCC_descriptor() | 8 | uimsbf |
| &nbsp;&nbsp;`}` | | |
| `}` | | |

Field notes:
- `download_rep_tag` — 24-bit field with value `0x9F8001` identifying this message.
- `DSMCC_descriptor()` — the encapsulated DSM-CC message bytes; walked byte-by-byte.

---

## §6.7.4.3 — User Authorization Initiate object (Table 77, p. 68)

apdu_tag `user_authorization_initiate_tag` = `0x9F 80 02`, Direction host ---> app.

User Authorization Initiate is sent from the host to the module, requesting that the module obtain user authorization to initiate a firmware download to the host of a specified binary (see §6.7.3.1). After sending this object the host shall enable the module to open an MMI session.

| Syntax | No. of bits | Mnemonic |
|--------|-------------|----------|
| `user_authorization_initiate() {` | | |
| &nbsp;&nbsp;user_authorization_initiate_tag | 24 | uimsbf |
| &nbsp;&nbsp;length_field() | | |
| &nbsp;&nbsp;specifier | 24 | bslbf |
| &nbsp;&nbsp;model | 16 | bslbf |
| &nbsp;&nbsp;version | 16 | bslbf |
| &nbsp;&nbsp;`for (i=0; i<n; i++) {` | | |
| &nbsp;&nbsp;&nbsp;&nbsp;data_byte | 8 | bslbf |
| &nbsp;&nbsp;`}` | | |
| `}` | | |

Field notes:
- `user_authorization_initiate_tag` — 24-bit field with value `0x9F8002` identifying this message.
- `specifier` / `model` / `version` — the 7-byte binary identification of §6.7.3.1.
- `data_byte` — optional field with meaning defined by the specifier.

---

## §6.7.4.4 — User Authorization Result object (Table 78, p. 68)

apdu_tag `user_authorization_result_tag` = `0x9F 80 03`, Direction app <--- host (module to host).

User Authorization Result is sent from the module to the host. It indicates if the user has agreed to the download of the specified binary.

| Syntax | No. of bits | Mnemonic |
|--------|-------------|----------|
| `user_authorization_result() {` | | |
| &nbsp;&nbsp;user_authorization_result_tag | 24 | uimsbf |
| &nbsp;&nbsp;length_field() | | |
| &nbsp;&nbsp;specifier | 24 | bslbf |
| &nbsp;&nbsp;model | 16 | bslbf |
| &nbsp;&nbsp;version | 16 | bslbf |
| &nbsp;&nbsp;`for (i=0; i<n; i++) {` | | |
| &nbsp;&nbsp;&nbsp;&nbsp;result_byte | 8 | uimsbf |
| &nbsp;&nbsp;`}` | | |
| `}` | | |

Field notes:
- `user_authorization_result_tag` — 24-bit field with value `0x9F8003` identifying this message.
- `specifier` / `model` / `version` — the 7-byte binary identification of §6.7.3.1.
- `result_byte` — conveys the user response; meaning defined by the specifier.

---

# §6.7.5 — Host-module exchanges

The following DSM-CC messages are reproduced from ISO/IEC 13818-6. They are the payloads carried inside the `DSMCC_descriptor()` loop of the Download Enquiry / Download Reply objects above. The "DSM-CC element" column maps each block of fields onto its DSM-CC structure; the "Size (bytes)" column gives the field width in bytes (not bits) as printed in the spec; the "Value" column gives the fixed value where the spec fixes it.

## §6.7.5.1 — Initial host-module negotiation

### Download Info Request (Table 79, p. 70)

When a host that supports firmware download detects a module providing the download resource, it opens a session and sends a Download Enquiry object encapsulating a Download Info Request, communicating the firmware version(s) currently loaded in the host (in one or more compatibility descriptors) and the buffer size / maximum block size the host can accommodate. The `transactionId` is assigned by the client (host); per DSM-CC, the 2 most significant bits shall be zero.

| Syntax | DSM-CC element | Size (bytes) | Value | Notes |
|--------|----------------|--------------|-------|-------|
| `DownloadInfoRequest() {` | | | | |
| &nbsp;&nbsp;protocolDiscriminator | dsmccMessageHeader | 1 | 0x11 | MPEG-2 DSM-CC |
| &nbsp;&nbsp;dsmccType | | 1 | 0x03 | U-N Download message |
| &nbsp;&nbsp;messageId | | 2 | 0x1001 | DownloadInfoRequest |
| &nbsp;&nbsp;transactionId | | 4 | | Client assigned |
| &nbsp;&nbsp;reserved | | 1 | 0xFF | |
| &nbsp;&nbsp;adaptationLength | | 1 | | |
| &nbsp;&nbsp;messageLength | | 2 | | |
| &nbsp;&nbsp;`if (adaptationLength > 0) {` | | | | |
| &nbsp;&nbsp;&nbsp;&nbsp;adaptationType | dsmccAdaptationHeader | 1 | | Optional CA or private information |
| &nbsp;&nbsp;&nbsp;&nbsp;`for (i=0; i<(adaptationLength-1); i++) {` | | | | |
| &nbsp;&nbsp;&nbsp;&nbsp;&nbsp;&nbsp;adaptationDataByte | | 1 | | |
| &nbsp;&nbsp;&nbsp;&nbsp;`}` | | | | |
| &nbsp;&nbsp;`}` | | | | |
| &nbsp;&nbsp;bufferSize | | 4 | | |
| &nbsp;&nbsp;maximumBlockSize | | 2 | | |
| &nbsp;&nbsp;compatibilityDescriptorLength | compatibilityDescriptor | 2 | | |
| &nbsp;&nbsp;descriptorCount | | 2 | | |
| &nbsp;&nbsp;`for (i=0; i<descriptorCount; i++) {` | | | | |
| &nbsp;&nbsp;&nbsp;&nbsp;descriptorType | | 1 | 0x02 | System Software |
| &nbsp;&nbsp;&nbsp;&nbsp;descriptorLength | | 1 | | |
| &nbsp;&nbsp;&nbsp;&nbsp;specifierType | | 1 | 0x01 | IEEE OUI |
| &nbsp;&nbsp;&nbsp;&nbsp;specifierData | | 3 | | OUI |
| &nbsp;&nbsp;&nbsp;&nbsp;model | | 2 | | |
| &nbsp;&nbsp;&nbsp;&nbsp;version | | 2 | | |
| &nbsp;&nbsp;&nbsp;&nbsp;subDescriptorCount | | 1 | | |
| &nbsp;&nbsp;&nbsp;&nbsp;`for (j=0; j<subDescriptorCount; j++) {` | | | | |
| &nbsp;&nbsp;&nbsp;&nbsp;&nbsp;&nbsp;subDescriptorType | subDescriptor | 1 | | |
| &nbsp;&nbsp;&nbsp;&nbsp;&nbsp;&nbsp;subDescriptorLength | | 1 | | |
| &nbsp;&nbsp;&nbsp;&nbsp;&nbsp;&nbsp;`for (k=0; k<subDescriptorLength; k++) {` | | | | |
| &nbsp;&nbsp;&nbsp;&nbsp;&nbsp;&nbsp;&nbsp;&nbsp;additionalInformation | | 1 | | |
| &nbsp;&nbsp;&nbsp;&nbsp;&nbsp;&nbsp;`}` | | | | |
| &nbsp;&nbsp;&nbsp;&nbsp;`}` | | | | |
| &nbsp;&nbsp;`}` | | | | |
| &nbsp;&nbsp;privateDataLength | | 2 | | |
| &nbsp;&nbsp;`for (i=0; i<privateDataLength; i++) {` | | | | |
| &nbsp;&nbsp;&nbsp;&nbsp;privateDataByte | | 1 | | |
| &nbsp;&nbsp;`}` | | | | |
| `}` | | | | |

### Download Info Response (Table 80, pp. 71–72)

The server (module) replies to the Download Info Request with a Download Reply object encapsulating either a Download Info Response or a Download Cancel. A Download Info Response communicates the relevant firmware version(s) the module can update (one or more compatibility descriptors; zero descriptors if the module is not compatible with the host), the `downloadId` to identify the download, and the buffer size / `windowSize` / `ackPeriod` etc. characterizing the download dynamics. The `transactionID` matches that in the Download Info Request.

| Syntax | DSM-CC element | Size (bytes) | Value | Notes |
|--------|----------------|--------------|-------|-------|
| `DownloadInfoResponse() {` | | | | |
| &nbsp;&nbsp;protocolDiscriminator | dsmccMessageHeader | 1 | 0x11 | MPEG-2 DSM-CC |
| &nbsp;&nbsp;dsmccType | | 1 | 0x03 | U-N Download message |
| &nbsp;&nbsp;messageId | | 2 | 0x1002 | DownloadInfoResponse |
| &nbsp;&nbsp;transactionId | | 4 | | Client assigned |
| &nbsp;&nbsp;reserved | | 1 | 0xFF | |
| &nbsp;&nbsp;adaptationLength | | 1 | | |
| &nbsp;&nbsp;messageLength | | 2 | | |
| &nbsp;&nbsp;`if (adaptationLength > 0) {` | | | | |
| &nbsp;&nbsp;&nbsp;&nbsp;adaptationType | dsmccAdaptationHeader | 1 | | Optional CA or private information |
| &nbsp;&nbsp;&nbsp;&nbsp;`for (i=0; i<(adaptationLength-1); i++) {` | | | | |
| &nbsp;&nbsp;&nbsp;&nbsp;&nbsp;&nbsp;adaptationDataByte | | 1 | | |
| &nbsp;&nbsp;&nbsp;&nbsp;`}` | | | | |
| &nbsp;&nbsp;`}` | | | | |
| &nbsp;&nbsp;downloadId | | 4 | | |
| &nbsp;&nbsp;blockSize | | 2 | | |
| &nbsp;&nbsp;windowSize | | 1 | | |
| &nbsp;&nbsp;ackPeriod | | 1 | | |
| &nbsp;&nbsp;tCDownloadWindow | | 4 | | |
| &nbsp;&nbsp;tCDownloadScenario | | 4 | | |
| &nbsp;&nbsp;compatibilityDescriptorLength | compatibilityDescriptor | 2 | | |
| &nbsp;&nbsp;descriptorCount | | 2 | | |
| &nbsp;&nbsp;`for (i=0; i<descriptorCount; i++) {` | | | | |
| &nbsp;&nbsp;&nbsp;&nbsp;descriptorType | | 1 | 0x02 | System Software |
| &nbsp;&nbsp;&nbsp;&nbsp;descriptorLength | | 1 | | |
| &nbsp;&nbsp;&nbsp;&nbsp;specifierType | | 1 | 0x01 | IEEE OUI |
| &nbsp;&nbsp;&nbsp;&nbsp;specifierData | | 3 | | OUI |
| &nbsp;&nbsp;&nbsp;&nbsp;model | | 2 | | |
| &nbsp;&nbsp;&nbsp;&nbsp;version | | 2 | | |
| &nbsp;&nbsp;&nbsp;&nbsp;subDescriptorCount | | 1 | | |
| &nbsp;&nbsp;&nbsp;&nbsp;`for (j=0; j<subDescriptorCount; j++) {` | | | | |
| &nbsp;&nbsp;&nbsp;&nbsp;&nbsp;&nbsp;subDescriptorType | subDescriptor | 1 | | |
| &nbsp;&nbsp;&nbsp;&nbsp;&nbsp;&nbsp;subDescriptorLength | | 1 | | |
| &nbsp;&nbsp;&nbsp;&nbsp;&nbsp;&nbsp;`for (k=0; k<subDescriptorLength; k++) {` | | | | |
| &nbsp;&nbsp;&nbsp;&nbsp;&nbsp;&nbsp;&nbsp;&nbsp;additionalInformation | | 1 | | |
| &nbsp;&nbsp;&nbsp;&nbsp;&nbsp;&nbsp;`}` | | | | |
| &nbsp;&nbsp;&nbsp;&nbsp;`}` | | | | |
| &nbsp;&nbsp;`}` | | | | |
| &nbsp;&nbsp;numberOfModules | | 2 | | |
| &nbsp;&nbsp;`for (i=0; i<numberOfModules; i++) {` | | | | |
| &nbsp;&nbsp;&nbsp;&nbsp;moduleId | | 2 | | |
| &nbsp;&nbsp;&nbsp;&nbsp;moduleSize | | 4 | | |
| &nbsp;&nbsp;&nbsp;&nbsp;moduleVersion | | 1 | | |
| &nbsp;&nbsp;&nbsp;&nbsp;moduleInfoLength | | 1 | | |
| &nbsp;&nbsp;&nbsp;&nbsp;`for (i=0; i<moduleInfoLength; i++) {` | | | | |
| &nbsp;&nbsp;&nbsp;&nbsp;&nbsp;&nbsp;moduleInfoByte | | 1 | | |
| &nbsp;&nbsp;&nbsp;&nbsp;`}` | | | | |
| &nbsp;&nbsp;`}` | | | | |
| &nbsp;&nbsp;privateDataLength | | 2 | | |
| &nbsp;&nbsp;`for (i=0; i<privateDataLength; i++) {` | | | | |
| &nbsp;&nbsp;&nbsp;&nbsp;privateDataByte | | 1 | | |
| &nbsp;&nbsp;`}` | | | | |
| `}` | | | | |

The Download Info Response may also carry other data defined by the specifier in the adaptation data bytes, additional information, moduleInfo and private data bytes.

## §6.7.5.2 — User Authorization

If the client (host) is satisfied that the download is technically appropriate it has the option to seek user authorization before initiating the download (the User Authorization Initiate / Result objects of §6.7.4.3–4 support this). If the host determines not to proceed it shall send a Download Cancel to the module with `downloadCancelReason` `rsnAbort` (or a specifier private reason).

### Download Cancel (Table 81, p. 72)

| Syntax | DSM-CC element | Size (bytes) | Value | Notes |
|--------|----------------|--------------|-------|-------|
| `DownloadCancel() {` | | | | |
| &nbsp;&nbsp;protocolDiscriminator | dsmccMessageHeader | 1 | 0x11 | MPEG-2 DSM-CC |
| &nbsp;&nbsp;dsmccType | | 1 | 0x03 | U-N Download message |
| &nbsp;&nbsp;messageId | | 2 | 0x1005 | DownloadCancel |
| &nbsp;&nbsp;transactionId | | 4 | | Server assigned |
| &nbsp;&nbsp;reserved | | 1 | 0xFF | |
| &nbsp;&nbsp;adaptationLength | | 1 | | |
| &nbsp;&nbsp;messageLength | | 2 | | |
| &nbsp;&nbsp;`if (adaptationLength > 0) {` | | | | |
| &nbsp;&nbsp;&nbsp;&nbsp;adaptationType | dsmccAdaptationHeader | 1 | | Optional CA or private information |
| &nbsp;&nbsp;&nbsp;&nbsp;`for (i=0; i<(adaptationLength-1); i++) {` | | | | |
| &nbsp;&nbsp;&nbsp;&nbsp;&nbsp;&nbsp;adaptationDataByte | | 1 | | |
| &nbsp;&nbsp;&nbsp;&nbsp;`}` | | | | |
| &nbsp;&nbsp;`}` | | | | |
| &nbsp;&nbsp;downloadId | | 4 | | |
| &nbsp;&nbsp;moduleId | | 2 | | |
| &nbsp;&nbsp;blockNumber | | 2 | | |
| &nbsp;&nbsp;downloadCancelReason | | 1 | | |
| &nbsp;&nbsp;privateDataLength | | 2 | | |
| &nbsp;&nbsp;`for (i=0; i<privateDataLength; i++) {` | | | | |
| &nbsp;&nbsp;&nbsp;&nbsp;privateDataByte | | 1 | | |
| &nbsp;&nbsp;`}` | | | | |
| `}` | | | | |

## §6.7.5.3 — Data Download

A Download Data Request message (encapsulated in a Download Enquiry object) is sent by the client (host) to the server to initiate the download using the `DownloadID` from the Download Info Response, with `downloadReason` `rsnStart`. Further Download Data Request messages acknowledge transfer of blocks and ultimately completion of the transfer (or Download Cancel terminates it). In response the server (module) transmits portions of the data in Download Data Block messages.

### Download Data Request (Table 82, p. 73)

| Syntax | DSM-CC element | Size (bytes) | Value | Notes |
|--------|----------------|--------------|-------|-------|
| `DownloadDataRequest() {` | | | | |
| &nbsp;&nbsp;protocolDiscriminator | dsmccDownloadDataHeader | 1 | 0x11 | MPEG-2 DSM-CC |
| &nbsp;&nbsp;dsmccType | | 1 | 0x03 | U-N Download message |
| &nbsp;&nbsp;messageId | | 2 | 0x1004 | DownloadDataRequest |
| &nbsp;&nbsp;DownloadId | | 4 | | |
| &nbsp;&nbsp;reserved | | 1 | 0xFF | |
| &nbsp;&nbsp;adaptationLength | | 1 | | |
| &nbsp;&nbsp;messageLength | | 2 | | |
| &nbsp;&nbsp;`if (adaptationLength > 0) {` | | | | |
| &nbsp;&nbsp;&nbsp;&nbsp;adaptationType | dsmccAdaptationHeader | 1 | | Optional CA or private information |
| &nbsp;&nbsp;&nbsp;&nbsp;`for (i=0; i<(adaptationLength-1); i++) {` | | | | |
| &nbsp;&nbsp;&nbsp;&nbsp;&nbsp;&nbsp;adaptationDataByte | | 1 | | |
| &nbsp;&nbsp;&nbsp;&nbsp;`}` | | | | |
| &nbsp;&nbsp;`}` | | | | |
| &nbsp;&nbsp;moduleId | | 2 | | |
| &nbsp;&nbsp;blockNumber | | 2 | | |
| &nbsp;&nbsp;downloadReason | | 1 | | |
| `}` | | | | |

### Download Data Block (Table 83, p. 74)

In response to Download Data Request messages the server (module) shall transmit portions of the data to be downloaded in Download Data Block messages as described by DSM-CC.

| Syntax | DSM-CC element | Size (bytes) | Value | Notes |
|--------|----------------|--------------|-------|-------|
| `DownloadDataBlock() {` | | | | |
| &nbsp;&nbsp;protocolDiscriminator | dsmccDownloadDataHeader | 1 | 0x11 | MPEG-2 DSM-CC |
| &nbsp;&nbsp;dsmccType | | 1 | 0x03 | U-N Download message |
| &nbsp;&nbsp;messageId | | 2 | 0x1003 | DownloadDataBlock |
| &nbsp;&nbsp;DownloadId | | 4 | | |
| &nbsp;&nbsp;reserved | | 1 | 0xFF | |
| &nbsp;&nbsp;adaptationLength | | 1 | | |
| &nbsp;&nbsp;messageLength | | 2 | | |
| &nbsp;&nbsp;`if (adaptationLength > 0) {` | | | | |
| &nbsp;&nbsp;&nbsp;&nbsp;adaptationType | dsmccAdaptationHeader | 1 | | Optional CA or private information |
| &nbsp;&nbsp;&nbsp;&nbsp;`for (i=0; i<(adaptationLength-1); i++) {` | | | | |
| &nbsp;&nbsp;&nbsp;&nbsp;&nbsp;&nbsp;adaptationDataByte | | 1 | | |
| &nbsp;&nbsp;&nbsp;&nbsp;`}` | | | | |
| &nbsp;&nbsp;`}` | | | | |
| &nbsp;&nbsp;moduleId | | 2 | | |
| &nbsp;&nbsp;moduleVersion | | 1 | | |
| &nbsp;&nbsp;reserved | | 1 | | |
| &nbsp;&nbsp;blockNumber | | 2 | | |
| &nbsp;&nbsp;`for (i=0; i<N; i++) {` | | | | |
| &nbsp;&nbsp;&nbsp;&nbsp;blockDataByte | | 1 | | |
| &nbsp;&nbsp;`}` | | | | |
| `}` | | | | |

The `blockDataByte` bytes carry the firmware payload; the present document does not describe the encoding of this data (signatures, linking information etc. are a matter for manufacturer development — §6.7.1 / phase 4 §6.7.2). Treat as an **opaque variable-length byte blob**.

## §6.7.5.4 — Private Data Fields in DSM-CC messages

- `dsmccAdaptationHeader` — shall either be empty or carry one or more DSM-CC conditional access adaptation fields. If used to carry CA adaptation fields, the `caSystemId` field shall carry a `CA_system_id` value registered in ETR 162.
- CompatibilityDescriptor `subDescriptor` — shall either be empty or carry information defined by the specifier of the enclosing CompatibilityDescriptor.
- `PrivateData` — this field shall be empty.

## §6.7.5.5 — Minimum compatibility

- Modules providing a download resource shall tolerate insertion into hosts with which they are not compatible and not disturb such hosts. If the module does not recognize the host from the Download Info Request message(s) it should respond with Download Cancel and then close the session.
- Hosts recognizing the download resource shall tolerate insertion of modules with which they are not compatible. If the host does not recognize the module from the Download Info Response message(s) it should respond with Download Cancel and then close the session.
