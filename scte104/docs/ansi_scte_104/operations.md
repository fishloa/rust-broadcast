# ANSI/SCTE 104 2023 — message framing + operations (machine transcription)

_Source: ANSI/SCTE 104 2023 (specs/ansi_scte_104_2023.pdf), pp. 26-66, via BlazeDocs OCR.
This is the working transcription of the single/multiple operation message framing,
the op_id allocation table, and the per-operation data structures. High-stakes
bit/byte layouts must be spot-verified against the PDF render before/while coding
(blaze is the table oracle; verify vector/critical values). One-table-per-file
split is a follow-up refinement._

ANSI/SCTE 104 2023

system (DCS). This is especially important when there are multiple DPI PIDs referenced by the PMT of a single MPEG program.

**DPI_PID_index** is required only if multiple Injector Instances (logical injectors) are present for any physical connection or if one or more Injector Instances are generating more than one DPI PID. Examples of situations requiring non-zero values of **DPI_PID_index** are multiple injectors listening to the same physical connection, such as multiple injectors receiving the same video stream, or multiple Injector Instances located behind a single IP address and port number.

Ordinarily, there **shall** be one value of **DPI_PID_index** for each DPI PID referenced by a program’s PMT for each program within the purview of the DCS. The exception to the rule is the case where a single DPI PID is shared by more than one program within a single TS. In this case, more than one PMT **may** make reference to the same shared DPI PID via a common value for **DPI_PID_index**.

Multiple language versions of the same movie are an example where this facility **may** be utilized. The AS is expected to know what these programs are and that the same value of **DPI_PID_index** **may** be assigned for each. In this example, the different programs share a video PID but have different audio PIDs for each language. The associated DPI PID for the video could be the same or different in this case.

The AS **may** validate for shared PIDs before sending a provisioning_response message (see Section 10.5.1.2).

In all other circumstances, each value of **DPI_PID_index** **shall** be unique.

This value is normally furnished to the AS by the PAMS during system initialization as part of the Injector Service Notification (via the provisioning_request message, see Section 10.4). In systems without PAMS to AS service, this value must be manually provided to the automation system.

It is recommended that even trivial system architectures utilize non-zero values of **DPI_PID_index**.

## 8.2.2. Single Operation Message

This variable length structure carries a single instance of an operation (request or response as it will be normally termed) listed in Table 8-3 and whose structural details are provided in Section 9 and Section 9.8.11 of this document.

Operations listed in Table 8-3 **shall** use the single_operation_message() and **shall not** use multiple_operation_message().

AMERICAN NATIONAL STANDARD

©2023 SCTE

ANSI/SCTE 104 2023

Table 8-1: single operation message

|  Syntax | Bytes | Type  |
| --- | --- | --- |
|  single operation message() |  |   |
|  opID | 2 | uimsbf  |
|  messageSize | 2 | uimsbf  |
|  result | 2 | uimsbf  |
|  result extension | 2 | uimsbf  |
|  protocol version | 1 | uimsbf  |
|  AS index | 1 | uimsbf  |
|  message_number | 1 | uimsbf  |
|  DPI_PID_index | 2 | uimsbf  |
|  data() | * | Varies  |

## 8.2.2.1. Semantics of fields in single_operation_message()

opID – An integer value that indicates what message is being sent. See Table 8-3. It shall only take values whose “Usage” column entries are listed as “Basic Request” or “Basic Response.”

messageSize – The size of the entire single_operation_message() structure in bytes.

result – The results to the requested message. See Section 14 (Result Codes) for details on the result codes. For message Usage types (as shown in the Usage column of Table 8-3) other than Basic Response messages, this shall be set to 0xFFFF.

result_extension – This shall be set to 0xFFFF unless used to send additional result information in a response message.

protocol_version – An 8-bit unsigned integer field whose function is to allow, in the future, this message type to carry parameters that may be structured differently than those defined in the current protocol. It shall be zero (0x00). Non-zero values of protocol_version may be used by a future version of this standard to indicate structurally different messages.

## 8.2.3. Multiple Operation Message

This variable length structure carries one or more of the operations (or requests) listed in Table 8-4 which must be either “Normal”, “Control”, or “Supplemental” in Usage category and whose structural details are provided in Section 9 of this document. Each request in the data() structure includes a opID value (two bytes) and a length (two bytes). Thus the first four bytes of every request within the repeating structure is identical to easily permit a receive device to skip a request if the opID is unknown. This allows for extensions to the protocol in the future.

AMERICAN NATIONAL STANDARD

©2023 SCTE

ANSI/SCTE 104 2023

Use of the multiple_operation_message() will normally result in the insertion of at least one SCTE 35 [SCTE35] splice_info_section into the resultant TS, unless the Injector (IJ) detects fatal errors in the message. In multiple byte fields the first byte received is the most significant byte. The value placed in the SCTE 35 [SCTE35] splice_info_section variable named "tier" may be user specified by the insert_tier_data() request (See Section 9.8.9). In the absence of an insert_tier_data() request, the Injector shall set "tier" to the default value 0xFFF.

Note that the use of the multiple_operation_message() will result in a single_operation message in response, since response messages are defined as Basic Usage responses (which, by definition, use the single_operation_message).

## 8.2.3.1. Order of Request Execution

This structure permits multiple requests to be grouped together to permit transmission in one message (and execution as appropriate). Its use is permitted in both bi-directional (serial or TCP/IP-based) and uni-directional systems. The data() structure is populated with one or more of the request structures defined in Section 9 (within the constraints identified elsewhere in this document). The time of processing may be instantaneous or Deferred, as required.

All requests are executed in the order that they exist within the data() structure. If requests are time based, then the time is referenced to the start of the video frame that the last byte is received, not the frame in which it was actually processed.

Requests listed in Table 8-3 shall not use the multiple_operation_message().

Some requests are order dependant, such as the various Supplemental requests. The Supplemental request modifies the characteristics of a Normal request, so they must be carried following the associated Normal request. In this way, multiple Normal requests with Supplemental requests can be carried without confusing which Supplemental request is associated with which Normal request.

Each instance of data() shall begin with a Normal or a Control request. A Normal request may be followed by zero or more Supplemental requests which modify or augment it. Unless otherwise specified, Supplemental request operations may occur in any order, except that they must follow the Normal operation to which they apply. It may then be followed by additional Normal requests for which the AS requests time deferral. The placement of a new Normal request shall indicate that the definition of the preceding Normal request is complete and that the resulting SCTE 35 [SCTE35] splice_info_section can be formatted and output at the time indicated by timestamp().

As used here, the term "processed" refers to whatever operations the Injector must accomplish to emit an SCTE 35 [SCTE35] section or sections or change a CW database. Processing begins when the timestamp() time has expired and ends when the section or sections are placed in the TS or the database is updated.

AMERICAN NATIONAL STANDARD

©2023 SCTE

ANSI/SCTE 104 2023

## 8.2.3.2. Format of the multiple_operation_message() structure

Table 8-2: multiple operation message

|  Syntax | Bytes | Type  |
| --- | --- | --- |
|  multiple_operation_message() { |  |   |
|  Reserved | 2 | uimsbf  |
|  messageSize | 2 | uimsbf  |
|  protocol_version | 1 | uimsbf  |
|  AS_index | 1 | uimsbf  |
|  message_number | 1 | uimsbf  |
|  DPI_PID_index | 2 | uimsbf  |
|  SCTE35 protocol version | 1 | uimsbf  |
|  timestamp() | * | Varies  |
|  num_ops | 1 | uimsbf  |
|  for (i=0; i < num_ops; i++) { |  |   |
|  opID | 2 |   |
|  data_length | 2 |   |
|  data() | * | Varies  |
|  } |  |   |
|  } |  |   |

## 8.2.3.3. Semantics of fields in multiple_operation_message()

Reserved – This field shall be set to all ones (0xFFFF).

messageSize – The size of the entire multiple_operation_message() structure in bytes.

protocol_version – An 8-bit unsigned integer field whose function is to allow, in the future, this message type to carry parameters that may be structured differently than those defined in the current protocol. It shall be zero (0x00). Non-zero values of protocol_version may be used by a future version of this standard to indicate structurally different messages.

AS_index – Defined in Section 8.2.1 above.

message_number – An integer value that is used to identify an individual message. The message_number variable must be unique for the life of a message. When multiple copies of the same message are sent, they can be identified because they have the same message_number. This means that

AMERICAN NATIONAL STANDARD

©2023 SCTE

ANSI/SCTE 104 2023

for messages that are to be processed in the future, the **message_number** may not be reused until the message has been processed. If not in current use, the **message_number** may freely vary over the range of 0 to 255.

In a uni-directional system, the message number can be assumed to be available for reuse after the associated processing timestamp() time has passed.

**DPI_PID_index** – Defined in Section 8.2.1 above.

**SCTE35_protocol_version** – This 8-bit unsigned integer field indicates the version of SCTE 35 protocol that the section which results from this message conforms to. Its function is to allow, in the future, this section type to carry parameters that may be structured differently than those defined in the current protocol. At present, the only valid value defined by SCTE 35 [SCTE35] is zero (0x00). Non-zero values of SCTE35_protocol_version may be used by a future version of this standard to indicate structurally different sections.

timestamp() – This field delivers the exact time to process all of the requests in this message (See Section 12.5). The **time_type** field of timestamp() may be zero, indicating the messages are processed immediately. The timestamp() may contain either the UTC time or the VITC time specifying when to process the requests. The timestamp() may alternatively contain the number of the GPI to use for triggering the messages to be processed. Once the GPI is triggered, all requests associated with that edge of the GPI will be processed.

**num_ops** – An integer value that indicates the number of requests contained within the data() loop.

**opID** – An integer value that indicates what request is being sent. See Table 8-4.

**data_length** – The size of the data() field being sent in bytes.

data() – Specific data structure for the request being sent. Details on each of the requests containing data are described in Sections 9.3.1, 9.4, 9.5, 9.7, and 9.8 of this document. The size of this field is equal to data_length and is determined by the size of the data being added to the multiple_operation_message() structure.

## 8.2.3.4. Detailed Discussion of Message Syntax and Semantics

Note that each opID in Table 8-4 has an associated “Usage” column, which indicates the class of each request. Normal requests have no associations with other requests and (once the time value specified in the timestamp() structure is reached) are immediately formatted into the appropriate SCTE 35 [SCTE35] message and dispatched. Each Normal request may be followed by zero or more “Supplemental” requests. The Supplemental requests must follow immediately after the Normal request that they are modifying. Some Supplemental requests are specific to a certain type of Normal request. Others are a general Supplemental request that can be associated with any Normal request, when appropriate. The Injector must ensure in processing any Normal requests that it checks for the existence of associated Supplemental requests before inserting the transport packet into the multiplex.

For the Control requests, only one request per Control Word index is permitted within a single multiple_operation_message(). It is permitted to send several requests in the same message, each operating on different Control Words. For example, update CW_index 1 and delete CW_index 2 in the same message is permitted. It would not be permitted to update CW_index 1 and then delete CW_index 1 within the same message.

AMERICAN NATIONAL STANDARD

©2023 SCTE

ANSI/SCTE 104 2023

DPI Schedules are potentially very large. The system is downloading a playlist of future ad avail periods, one splice point at a time. There is a single start message, and a single stop message, to frame the downloading of the data. Like other messages in this API, the schedules have Normal and Supplemental features. If Supplemental features are required, they must be included in the same message as the basic schedule request, and immediately following the associated basic request.

If multiple Normal requests are present in a message, then the requests are processed in the same order that they appear in the message. If the **time_type** field of timestamp() is zero, all Normal request timing is relative to the arrival time of the last byte of the message. Please see Section 8.2.3.1 for additional information.

## 8.3. Operation Types (Normative)

Table 8-3 and Table 8-4 contain the assigned values for each type of operation (request or response) supported by this API. Other columns in the tables list information identifying the normal originator and recipient, and other useful information.

Those operations required for the Simple Profile appear in the column labeled "In Simple Profile," with an indication of "Y." An "N" indicates that support of the Request is not required for compliance. "n/a" indicates "not applicable." With the sole exception of the "general_response_data") message, compliant implementations may also omit support for those messages in Table 8-3 which show PAMS as either the "Sent By" or "Sent To" when the PAMS is not a constituent portion of the overall system. Systems should with PAMS as constituent portion of the overall system should indicate this as "Simple Profile with PAMS," or, if applicable (and as an example), "Simple Profile plus encryption with PAMS."

AMERICAN NATIONAL STANDARD

©2023 SCTE

ANSI/SCTE 104 2023

Table 8-3: opID Assigned Values and Meanings for single_operation_messages

|  opID assigned value | Operation Name | Sent By | Sent To | In Simple Profile | Description | Usage  |
| --- | --- | --- | --- | --- | --- | --- |
|  0x0000 | general_response_data() | PAMS, Automation or Injector | PAMS, Automation or Injector | Y | Used to convey asynchronous information between the devices. There is no data() associated with this message. | basic response  |
|  0x0001 | init_request_data() | Automation | Injector | Y | Initial Message to Injector on predefined port | basic request  |
|  0x0002 | init_response_data() | Injector | Automation | Y | Initial Response to Automation on the established connection | basic response  |
|  0x0003 | alive_request_data() | Automation | Injector | Y | Sends an alive message to acquire current status. | basic request  |
|  0x0004 | alive_response_data() | Injector | Automation | Y | Response to the alive message indicating current status. | basic response  |
|  0x0005 - 0x0006 | User Defined |  |  | n/a | Receiving devices shall ignore these values. Used in legacy systems. |   |
|  0x0007 | inject_response_data() | Injector | Automation | Y | Response to indicate that the request was received and that Injector is preparing to send SCTE 35 [SCTE35] message or messages. | basic response  |
|  0x0008 | injectcomplete_response_data() | Injector | Automation | Y | Response from Injector when all resultant SCTE 35 [SCTE35] splice messages are sent. | basic response  |
|  0x0009 | config_request_data() | Automation | PAMS | n/a | Automation sends PAMS its IP configuration | basic request  |
|  0x000A | config_response_data() | PAMS | Automation | n/a | Responds to Config_Request | basic response  |
|  0x000B | provisioning_request_data() | PAMS | Automation | n/a | PAMS notification of the Injectors provisioned for DPI service | basic request  |
|  0x000C | provisioning_response_data() | Automation | PAMS | n/a | Response from Automation that the message is received and DPI is starting | basic response  |
|  0x000D -0x000E | Reserved |  |  | n/a | Range Reserved Used in legacy systems. |   |

AMERICAN NATIONAL STANDARD

©2023 SCTE

ANSI/SCTE 104 2023

|  opID assigned value | Operation Name | Sent By | Sent To | In Simple Profile | Description | Usage  |
| --- | --- | --- | --- | --- | --- | --- |
|  0x000F | fault_request_data() | Automation | PAMS | n/a | Automation discovered communication problem with an Injector | basic request  |
|  0x0010 | fault_response_data() | PAMS | Automation | n/a | Response from PAMS | basic response  |
|  0x0011 | AS_alive_request_data() | PAMS | Automation | n/a | Maintain PAMS to AS communications | basic response  |
|  0x0012 | AS_alive_response_data() | Automation | PAMS | n/a | Maintain AS to PAMS communications | basic response  |
|  0x0013 -0x00FF | Reserved for future basic requests or responses |  |  | n/a | Range Reserved for future standardization. |   |
|  0x0100 -0x7FFF | Reserved |  |  | n/a | Range Reserved for Table 8-4uses |   |
|  0x8000 -0xBFFF | User Defined | Automation or PAMS | Injector or PAMS | n/a | Range available for user defined functions. |   |
|  0xC000 - 0xFFFE | Reserved |  |  |  | Range Reserved for user defined Table 8-4 uses. |   |
|  0xFFFF | Reserved |  |  |  | Reserved value |   |

Table 8-4: opID Assigned Values and Meanings for multiple_operation_messages

|  opID assigned value | Operation Name | Sent By | Sent To | In Simple Profile | Description | Usage  |
| --- | --- | --- | --- | --- | --- | --- |
|  0x0000 - 0x00FF | Reserved |  |  | n/a | Range Reserved (see Table 8-3). |   |
|  0x0100 | inject_section_data_request() | Automation | Injector | Y | Generates an SCTE 35 [SCTE35] section directly | Normal  |
|  0x0101 | splice_request_data() | Automation | Injector | Y | Normally used request to send SCTE 35 [SCTE35] message or messages. | Normal  |
|  0x0102 | splice_null_request_data() | Automation | Injector | Y | Generates an SCTE 35 [SCTE35] splice_null operation | Normal  |
|  0x0103 | start_schedule_download_request_data() | Automation | Injector | N | Initiates schedule download | Normal  |

AMERICAN NATIONAL STANDARD

©2023 SCTE

ANSI/SCTE 104 2023

|  opID assigned value | Operation Name | Sent By | Sent To | In Simple Profile | Description | Usage  |
| --- | --- | --- | --- | --- | --- | --- |
|  0x0104 | time_signal_request_data() | Automation | Injector | Y | Generates an SCTE 35 [SCTE35] time_signal operation | Normal  |
|  0x0105 | transmit_schedule_request_data() | Automation | Injector | N | Initiates schedule transmission | Normal  |
|  0x0106 | component_mode_DPI_request_data() | Automation | Injector | N | Adds component mode to a DPI request | Supplemental  |
|  0x0107 | encrypted_DPI_request_data() | Automation | Injector | N | Adds encryption to a DPI request | Supplemental  |
|  0x0108 | insert_descriptor_request_data() | Automation | Injector | Y | Adds a descriptor to another operation | Supplemental  |
|  0x0109 | insert_DTMF_descriptor_request_data() | Automation | Injector | Y | Adds a DTMF descriptor to another operation | Supplemental  |
|  0x010A | insert_avail_descriptor_request_data() | Automation | Injector | Y | Adds an avail_descriptor to the SCTE 35 [SCTE35] section | Supplemental  |
|  0x010B | insert_segmentation_descriptor_request_data() | Automation | Injector | Y | Adds a segmentation descriptor to another operation | Supplemental  |
|  0x010C | proprietary_command_request_data() | Automation | Injector | Y | Adds a proprietary descriptor to another operation | Normal  |
|  0x010D | schedule_component_mode_request_data() | Automation | Injector | N | Adds component mode to an avail definition | Supplemental  |
|  0x010E | schedule_definition_data() request | Automation | Injector | N | Single avail definition | Supplemental  |
|  0x010F | insert_tier_data() | Automation | Injector | Y | Specifies tier data | Supplemental  |
|  0x0110 | insert_time_descriptor() | Automation | Injector | Y | Specifies insertion of time descriptors | Supplemental  |
|  0x0111 | insert_audio_descriptor request | Automation | Injector | Y | Specifies insertion of audio descriptors | Supplemental  |
|  0x0112 | insert_audio_provisioning request | Automation | Injector | Y | Specifies channel mode for audio service | Control  |
|  0x0113 | insert_alternate_break_duration() request | Automation | Injector | Y | Specifies substitution of break duration | Supplemental  |
|  0x0114 - 0x02FF | Reserved |  |  | n/a | Range Reserved for future standardization (additional Normal or Supplemental operations). |   |
|  0x0300 | delete_ControlWord_data()request | Automation | Injector | N | Maintains CW database | Control  |
|  0x0301 | update_ControlWord_data() request | Automation | Injector | N | Maintains CW database | Control  |

AMERICAN NATIONAL STANDARD

©2023 SCTE

ANSI/SCTE 104 2023

|  opID assigned value | Operation Name | Sent By | Sent To | In Simple Profile | Description | Usage  |
| --- | --- | --- | --- | --- | --- | --- |
|  0x0302 - 0x7FFF | Reserved |  |  | n/a | Range Reserved for future standardization (additional Control operations). |   |
|  0x8000 - 0xBFFF | Reserved |  |  | n/a | Range Reserved (see Table 8-3). |   |
|  0xC000 - 0xFFFE | User Defined | Automation or PAMS | Injector or PAMS | n/a | Range available for user defined functions for multiple operation messages. |   |
|  0xFFFF | Reserved |  |  |  | Reserved value. |   |

AMERICAN NATIONAL STANDARD

©2023 SCTE

35

ANSI/SCTE 104 2023

## 8.3.1. Meaning of the Usage Field in Table 8-3 and Table 8-4

The Usage field indicates the class of each request or response and the messages with which they may be used:

- Basic requests or responses **shall** always use the single_operation_message() structure (See Section 8.2.2).
- Normal requests **shall** have no linkage with other Normal requests and are normally formatted into the appropriate SCTE 35 [SCTE35] splice_info_section and dispatched. Normal requests **shall** use the multiple_operation_message() structure (See Section 8.2.3.2). While multiple Normal requests **may** be grouped together into a single instance of multiple_operation_message(), they **may** not have any dependencies beyond execution order (See Section 8.2.3.1).
- Supplemental requests are also carried only by the multiple_operation_message() structure (See Section 8.2.3). Each Supplemental request follows immediately after the Normal request that they are modifying. Some Supplemental requests are specific to a certain request. Others are a general request that can be associated with any Normal request, when appropriate.
- Control requests are also carried only by the multiple_operation_message() structure (See Section 8.2.3). Each Control request **shall** be independent of any other contained within the same data() structure and **shall** be executed at the time specified in the timestamp(). Multiple Control requests **may** be present within the data() structure. Supplemental requests do not modify Control requests.

## 8.4. Conventions and Requirements

1. Each message that contains data is outlined with its data fields and types below. Additional structures are indicated as functions and are described in Section 12 of this document.
2. The Injector **shall** retain the following data values while messages are being processed:

- message_number
- splice_event_id

These are retained until the inject_complete_response message is sent to the AS. In addition, each Injector which supports splice schedule messages must retain any descriptors defined via this API during the output of the individual SCTE 35 [SCTE35] splice_schedule() sections which result from a single schedule_definition request (See Section 9.7).

1. All string lengths have space reserved for a null terminator character (0x00) and **shall** use null terminated strings. The size defined for the string is constant and will not vary depending on the actual length of the string. As an example a string that is defined as 16 characters can have at most 15 characters of data followed by a null character. Once a null is encountered in scanning a string, the rest of the characters in the string are undefined and ignored. This specification uses 8 bit ASCII characters for strings.
2. Response messages **shall** be sent out without unnecessary delay. The device expecting a response should consider no response within 5 seconds to indicate a timeout. When the Automation System suspects a timeout, it **shall** send an alive_request message. If the Injector does not answer as specified in this document, the connection for this channel **shall** be dropped and re-established.
3. Initialization (or re-initialization) of the communications between the AS and the Injector **shall** not cause interruption of any of the audio, video, or DPI message insertions currently being processed by either the AS or the Injector. Initialization can be safely conducted at any point in time. This includes changes to Injector services or Injectors themselves. These events **may** be expected to occur at random intervals.

AMERICAN NATIONAL STANDARD ©2022 SCTE

ANSI/SCTE 104 2023

4. When a device is polling to start or restart communications, a suitable interval (30 to 60 seconds) may be left between attempts. Such an interval might be randomly determined, with exponential backoff, as is commonly used in Ethernet-based protocols.

# 9. Automation System to Injector Communication

# 9.1. Initialization

The methods of initializing the TCP/IP communications parameters are discussed in Section 10.4

For TCP/IP, the initial communication begins with Injector listening on predefined port 5167 and the Automation System opening an API Connection to the Injector via that socket. If another socket number has been furnished in the provisioning_request message (via the injector_socket_number field), that socket should be used instead of the default socket 5167. The Automation System sends an init_request message to the Injector. The Automation System then listens for the response from the Injector on the established API Connection. All further communication is done on this API Connection. Either the Automation System or Injector may terminate communications by closing this API Connection. Each device is responsible for detecting and properly handling a closed API Connection.

The Injector should support multiple Automation System connections simultaneously if provisioned to do so. When the Injector initializes the TCP listener on port 5167 it should allow for the number of API Connections it is provisioned for (see Section 11.4). No two Automation Systems may have an active connection to any given Injector Instance at any one time. The Injector Instance shall return a response of "Injector already in use" (see Table 14-1) if this occurs.

The protocol_version fields in single_operation_message() and multiple_operation_message() permit the Automation System and the Digital Compression System to "negotiate" at which level of the protocol the system will function. The lesser value shall be taken as the operating point for the system as initialized. Please note that this value may have implications upon the possible values for the SCTE35_protocol_version field (see Sections 8.2.2 and 8.2.3).

In a uni-directional system, the AS and Injector must both be configured to operate at a compatible protocol version.

# 9.1.1. init_request AS  $= = &gt;$  IJ

This basic usage request is sent by the Automation System to the Injector to initialize a TCP/IP connection. The appropriate value for desired protocol_version shall be furnished to the Injector in this message.

Table 9-1: init_request_data

|  Syntax | Bytes | Type  |
| --- | --- | --- |
|  init_request_data(){ |  |   |
|  } |  |   |

AMERICAN NATIONAL STANDARD ©2022 SCTE

ANSI/SCTE 104 2023

## 9.1.2. init_response IJ ==&gt; AS

This basic usage response is sent by the Injector to the Automation System to indicate the receipt of the init_request. The appropriate value for desired protocol_version shall be furnished to the AS in this message. All devices supporting this API shall operate from this point forward at the lesser of the furnished protocol_version values.

Table 9-2: init_response_data

|  Syntax | Bytes | Type  |
| --- | --- | --- |
|  init_response_data() { } |  |   |

A Proxy Device may respond to this message with a “Proxy Response” result code (see Table 14-1). This permits the Automation System, should it desire to do so, to track whether or not a given Injector is served by a Proxy Device or a direct connection.

## 9.2. Alive (“Heartbeat”) Communications

For bi-directional communications, once initialization is complete, then the Automation System shall send alive_request messages to ensure that the Injector and the communications path remain up and running. Each alive_response message (wrapped in the single_operation_message()) contains a result field that may be used to signal if DPI support has been stopped on the recipient’s end. If there has been no activity on the connection in the preceding 60 seconds, then an alive_request message shall be sent.

If TCP/IP is being used and the user de-provisions DPI support in the Injector, the Injector will close the socket connection to the Automation System without waiting for the next alive_request.

For uni-directional communications this message also serves to provide a mechanism that the receiving device shall use to verify a working connection to the automation computer. This message shall be sent at least once every 60 seconds. If the messages fail to arrive, then the receiving Injector shall notify its PAMS or a human operator that communications may be lost.

The second function is to provide clock synchronization for UTC or VITC time-stamped splice messages. The time () structure provides the time for the start of the associated video frame. This requires the sender and the receiver to both be examining synchronous video of the same frame rate. In multi-standard systems, this requirement is very important.

The receiving device can synchronize to the vertical interval of its incoming video and the received time () value and thus maintain a local UTC or VITC time base to use with time-stamped messages.

For TCP/IP-based systems, implementers may choose to use an external time standard to keep the internal clocks of the Automation System and the Injector in sync. This is not strictly necessary for the simplest implementation that meets the requirements of SCTE 35 [SCTE35].

If the Automation System has access to a facility master clock, and it makes sense to both parties, then the current value of facility time-of-day timecode can be transmitted in the “alive_request” messages from the Automation System to the Injector and conversely in the Injector to the Automation System “alive_response” responses. Alternatively, facility time-of-day time samples may be conveyed to the Injector in the video signal proper as VITC.

AMERICAN NATIONAL STANDARD ©2022 SCTE

ANSI/SCTE 104 2023

## 9.2.1. alive_request AS ==&gt; IJ

This basic request serves to ensure that the AS to Injector communications path remains open and reliable. In addition it may be used to ensure the internal time within each is synchronized. If deferred requests are to be used with a time-value trigger, then it is vital that synchronization be maintained.

Table 9-3: alive_request_data

|  Syntax | Bytes | Type  |
| --- | --- | --- |
|  alive_request_data() { time() } |  |   |

## 9.2.1.1. Semantics of fields in alive_request_data()

time() – This is an optional structure, unless the time_type field of the timestamp() structure carried in multiple operation messages is non-zero. The current UTC time clock of the sending device checked as close as possible to the sending of the message. This is designed to be used by the Injector and the Automation System to check on how well the two systems are time synchronized. See Section 12.4 for a definition of time(). If this time synchronization is not being used in a given system, the value of time() may be set to zero.

## 9.2.2. alive_response IJ ==&gt; AS

This basic response serves to ensure that the AS to Injector communications path remains open and reliable. In addition it may be used to ensure the internal time within each is synchronized. If deferred requests are to be used with a time-value trigger, then it is vital that synchronization be maintained.

Table 9-4: alive_response_data

|  Syntax | Bytes | Type  |
| --- | --- | --- |
|  alive_response_data() { time() } |  |   |

A Proxy Device should respond to this message with a “Successful Response” result code (see Table 14-1) as if it were an Injector.

## 9.2.2.1. Semantics of fields in alive_response_data()

time() – This is an optional structure, unless the time_type field of the timestamp() structure carried in multiple operation messages is non-zero. The current UTC time clock of the sending device checked as close as possible to the sending of the message. This is designed to be used by the Injector and the Automation System to check on how well the two systems are time synchronized. See Section 12.4 for a

AMERICAN NATIONAL STANDARD ©2022 SCTE

ANSI/SCTE 104 2023

definition of time(). If this time synchronization is not being used in a given system, the value of time() may be set to zero.

## 9.3. Splice Requests

After initializing communications with the Injector, the Automation System can issue (via a multiple operation message), one of the Normal requests listed in the Usage column of Table 8-4. Issuing typically a splice_request to initiate placement of one or more SCTE 35 [SCTE35] splice_info_sections into the outgoing TS. The Automation System may choose to send any of the messages multiple times before the designated in-point (especially if return path communications is unavailable). The Injector can detect that these are duplicates of one another by comparison of the message_number fields.

The two messages that are returned (in a bi-directional system) from the splice request messages are the inject_response message and the inject_complete_response message. A inject_response message is returned upon receipt of the splice request. A inject_complete_response message is returned once the SCTE 35 [SCTE35] section has been generated.

## 9.3.1. splice request  $AS ==&gt; IJ$

This Normal request is the usual carrier of splicing requests. It may be further elaborated upon by various Supplemental type requests which may follow it within the data() structure of a multiple_operation_message.

Table 9-5: splice_request_data

|  Syntax | Bytes | Type  |
| --- | --- | --- |
|  splice_request_data() { |  |   |
|  splice_insert_type | 1 | uimsbf  |
|  splice_event_id | 4 | uimsbf  |
|  unique_program_id | 2 | uimsbf  |
|  pre_roll_time | 2 | uimsbf  |
|  break_duration | 2 | uimsbf  |
|  avail_num | 1 | uimsbf  |
|  avails_expected | 1 | uimsbf  |
|  auto_return_flag | 1 | uimsbf  |
|  not_an_entry_flag | 1 | uimsbf  |

AMERICAN NATIONAL STANDARD ©2022 SCTE

---

ANSI/SCTE 104 2023

# 9.3.1.1. Semantics of fields in splice_request_data()

splice_insert_type – An 8-bit unsigned integer defining the type of insertion operation desired. These will result in the generation of one or more SCTE 35 [SCTE35] splice_info() sections with a splice_command_type field value of splice_insert with other inferred field values also being set within the resulting splice_info() section. The other inferred field values are noted with the discussion of each assigned value. Please refer to Section 9.3.2 below for additional clarification of the inferred values.

Table 9-6: splice_insert_type Assigned Values

|  splice_insert_type | Value assigned  |
| --- | --- |
|  reserved | 0  |
|  spliceStart_normal | 1  |
|  spliceStart_immediate | 2  |
|  spliceEnd_normal | 3  |
|  spliceEnd_immediate | 4  |
|  splice_cancel | 5  |

spliceStart_normal section(s) occur at least once before a splice point. This interval should match the requirements of SCTE 35 [SCTE35] (Section 7.1) and serve to set up the actual insertion. It is recommended that if sufficient pre-roll time is given by the AS, the Injector send several succeeding SCTE 35 [SCTE35] splice_info_section() sections (per SCTE 35 [SCTE35] and SCTE 67 [SCTE67]) in response to a single splice_request message with a spliceStart_normal splice_insert_type value. The minimum non-zero pre_roll_time is defined in Section 12.3 of this document.

spliceStart_immediate sections may come once at the splice point’s exact location. The Injector shall set the splice_immediate_flag to 1 and the out_of_network_indicator to 1 in the resulting SCTE 35 [SCTE35] splice_info_section() section. Usage of “immediate mode” signaling is not recommended by SCTE 35 [SCTE35] and may result in inaccurate splices.

spliceEnd_normal sections come to terminate a splice done without a duration specified. They may also be sent to ensure a splice has terminated on schedule. The Injector sets the out_of_network_indicator to 0. If they are to terminate a spliceStart_normal with no duration specified, they should be sent prior to the minimum interval before the return point and carry a value for pre_roll_time, especially if terminating a long form insertion. The minimum non-zero pre_roll_time is defined in Section 12.3 of this document.

spliceEnd_immediate sections come to terminate a current splice before the splice point, or a splice in process earlier than expected. The Injector sets the out_of_network_indicator to 0 and the splice_immediate_flag to 1. The value of pre_roll_time is ignored.

splice_cancel sections come to cancel a recently sent spliceStart_normal section. The AS must supply the correct value of splice_event_id for the section to be cancelled. The Injector shall set the splice_event_cancel_indicator to 1.

splice_event_id – As specified in SCTE 35 [SCTE35]. See the discussion in Section 12.1 of this document for further details. The Injector retains this value until the time indicated by the timestamp() is reached.

unique_program_id – As specified in SCTE 35 [SCTE35]. See the discussion in Section 12.2 of this document for further details.

AMERICAN NATIONAL STANDARD ©2022 SCTE

ANSI/SCTE 104 2023

pre_roll_time – An 16-bit field giving the time to the insertion point in milliseconds. This field is ignored for splice_insert_type values other than spliceStart_normal and spliceEnd_normal. If zero (and Component Mode is not in use) the Injector should set the splice_immediate_flag to 1 in the resulting SCTE 35 [SCTE35] splice_info_section. The minimum non-zero pre_roll_time is defined in Section 12.3 of this document.

break_duration – A 16-bit field giving the duration of the insertion in tenths of seconds. If zero the Injector will not set a duration. This field is ignored for splice_insert_type values other than spliceStart_normal and spliceStart_immediate.

avail_num – An 8-bit field giving an identification for a specific avail within the current unique_program_id. The value follows the semantics specified in SCTE 35 [SCTE35] for this field. It may be zero to indicate its non-usage.

avails_expected – An 8-bit field giving a count of the expected number of individual avails within the current viewing event. If zero, it indicates that avail_num has no meaning.

auto_return_flag – If this field is non-zero and a non-zero value of break_duration is present, then the auto_return field in the resulting SCTE 35 [SCTE35] section will be set to one. This field is ignored for splice_insert_type values other than spliceStart_normal and spliceStart_immediate.

not_an_entry_flag – When non-zero, this 8-bit optional field indicates to the compression system that this request shall not be inferred as an entry point in any transport. This flag is not passed into the resulting SCTE 35 [SCTE35] results from this request. When this field is not present, or the value is zero, then this request may be inferred as an entry point.

## 9.3.1.2. Detailed Discussion of Message Syntax and Semantics

The Automation System will only need to send a single splice_request message per splice unless there is a compelling reason to do so otherwise (such as video conveyance or cancellation). The Injector, on the other hand, may generate several SCTE 35 [SCTE35] splice_info_sections per splice on a normal basis. This is in keeping with the recommendations of SCTE 67 [SCTE67]. To permit such action, the AS must send the single splice_request message well in advance of the minimum pre_roll_time (for example, 10 seconds instead of the minimum 4).

If a spliceStart_normal request with a non-zero value of pre_roll_time which is less than the minimum allowed value is received, the Injector shall issue the resultant SCTE 35 [SCTE35] splice_info_section and return an error code of "pre-roll too small".

If the AS has issued a splice_cancel splice_insert_type request to the Injector, and the indicated request was issued with a time delay, then the Injector can use the splice_event_id field to determine if it should simply not issue the resulting SCTE 35 [SCTE35] section related to that message_number, or if it needs to issue a splice_insert() section with the splice_event_cancel_indicator set to '1'.

If a splice is to be canceled, then the splice_insert_type value would be splice_cancel, the AS supplies the correct value for splice_event_id and the Injector will set the splice_event_cancel_indicator to 1 in the resulting splice_info_section. If a splice is to be cancelled, then the AS is responsible for ensuring that a cancellation is sent before the indicated insertion point is reached.

If an early return is to be signaled, the splice_insert_type value would be spliceEnd_immediate. The splice_info_section the Injector will send as a result has out_of_network_indicator set to 0 and splice_immediate_flag set to 1.

AMERICAN NATIONAL STANDARD ©2022 SCTE

ANSI/SCTE 104 2023

For long-form insertions where a duration is either not known or the return is to be explicitly signaled, the break_duration field is set to 0 and a non-zero pre_roll_time value is given. At the return point, a spliceEnd_normal request is sent, again with a non-zero value in the pre_roll_time field. In this case, the Injector may also choose to send several return splice_infoSections in a manner analogous to spliceStart_normal.

# 9.3.2. Mapping of splice_request fields into SCTE 35 [SCTE35] splice_insert() fields (Informative)

The following table summarizes the settings resulting from the combination of the splice_insert_type and the other parameters in the splice_request_data(). Duration_flag is set to one if a non-zero break_duration is given.

Table 9-7: splice_insert_type corresponding splice_insert() field settings (Informative)

|  This API |   | Resulting SCTE 35 splice_insert() structure  |   |   |   |   |
| --- | --- | --- | --- | --- | --- | --- |
|  splice_insert_type | Value | splice_event_cancel_indicator | out_of_network_indicator | duration_flag | splice_immediate_flag | auto_return_flag*  |
|  reserved | 0 | n/a | n/a | n/a | n/a | n/a  |
|  spliceStart_normal | 1 | 0 | 1 | 0 or 1 | 0 | 0 or 1  |
|  spliceStart_immediate | 2 | 0 | 1 | 0 or 1 | 1 | 0 or 1  |
|  spliceEnd_normal | 3 | 0 | 0 | 0 | 0 | n/a (0)  |
|  spliceEnd_immediate | 4 | 0 | 0 | 0 | 1 | n/a (0)  |
|  splice_cancel | 5 | 1 | n/a (0) | 0 | n/a (0) | n/a (0)  |

* Note: The auto_return_flag is within the SCTE 35 [SCTE35] break_duration() structure, not the splice_insert() structure, in which all of the other parameters are defined.

A more detailed drawing is shown below, illustrating the mapping between the fields contained in a single_operation_message() (with opID of splice_request and the resulting SCTE 35 [SCTE35] splice_info_section()).

Please note that one or more descriptors are built in response to a splice_request, to which the user may add by use of an insert_avail_descriptor request (See Section 9.8.4), insert_descriptor request (See Section 9.8.5), an insert_DTMF_descriptor request (See Section 9.8.6), or an insert_segmentation_descriptor request (See Section 9.8.7).

Note: Advanced (with respect to MPEG-2) video codecs have added a structural concept called a "Stream Access Point" (or SAP). Refer to SCTE 172 Section 5.1 [[SCTE172]. "Abbreviations" as well as Section 6. "Digital Program Insertion System Overview (Informative)" for additional details. Signaling for the SAP values also exists in SCTE 35 [SCTE35]. The reader is also reminded about the not_an_entry_flag within the splice_request_data() structure, which also may be useful.

AMERICAN NATIONAL STANDARD ©2022 SCTE

ANSI/SCTE 104 2023

![img-0.jpeg](img-0.jpeg)
Figure 9-1: multiple_operation_message() to SCTE 35 section field mapping (Informative)

AMERICAN NATIONAL STANDARD ©2022 SCTE

ANSI/SCTE 104 2023

## 9.4. Encryption Support (Normative)

The method provided by this API for the support of encrypted SCTE 35 [SCTE35] splice_info_sections assumes that the encryption will be done by the Injector. As a result, the PAMS will need to supply a number of additional items of provisioning data related to the encryption method to be used, such as the key information (which may also be provided by the AS using this API) and so forth. Please refer to Section 9 of SCTE 35 [SCTE35] for additional information.

The Injector which supports encryption **shall** contain 256 Control Word “slots”. If a slot has been filled with a Control Word set (three 64-bit numbers) then encryption can take place. If the AS references a slot without a Control Word defined, then the entire generation of the associated splice_info_section **shall** be aborted and an error returned to the AS or an alarm raised by the Injector.

## 9.4.1. Encryption Control Word Support

The API specifies the basic messages to define and maintain the current (and next) control words. Compliant implementations which support encryption may choose not to support these messages (defined in Section 9.4.3 and Section 9.4.4) and instead have the PAMS manage all Control Words.

These AS requests carry sensitive security information. If these requests are used, then normal security precautions should be implemented (such as password protection on login screens and physical access restrictions to control areas). The assumption in using these messages is that the link used to carry the messages is secure and is not easily compromised. Further protection for these requests, such as encrypting the requests, is outside the scope of this document.

## 9.4.2. The encrypted DPI request

The encrypted DPI message is used for applications that wish to use the built in security capabilities of SCTE 35 [SCTE35] under the direction of the Automation System. This message is sent in the clear, and the resulting SCTE 35 [SCTE35] section will be encrypted by the Injector before being formatted and placed in the output multiplex.

The actual control words to use by the Injector must have been previously provisioned by the PAMS or by the AS (via the update_ControlWord request in Section 9.4.3) for the particular control word index, or the resulting SCTE 35 [SCTE35] splice_info_section() will not be placed in the outgoing TS, the data() discarded, and an error code returned by the Injector. In a uni-directional communication system, the error return path **shall** be notification of the PAMS operator.

This is a Supplemental usage request and must follow the associated splice_request_data() in the data() structure of the multiple_operation_message() (See Section 8.2.3) for which it applies. If component_mode_DPI_data() structures are also present in the multiple_operation_message data() structure, then the encrypted_DPI_data() follows the final occurrence of component_mode_DPI_data(). When this request is present, the encrypted_packet bit **shall** be set in the resulting splice_info_section().

AMERICAN NATIONAL STANDARD ©2022 SCTE

ANSI/SCTE 104 2023

Table 9-8: encrypted_DPI_request_data

|  Syntax | Bytes | Type  |
| --- | --- | --- |
|  encrypted_DPI_request_data() { |  |   |
|  encryption_algorithm | 1 | uimsbf  |
|  CW_index | 1 | uimsbf  |

## 9.4.2.1. Semantics of fields in encrypted_DPI_request_data()

encryption_algorithm – This field carries the value of the 6-bit field defined in SCTE 35 [SCTE35].

CW_index – An 8 bit unsigned integer which conveys which Control Word (key) is to be used to encrypt and decrypt the message.

## 9.4.3. update_ControlWord request AS ==&gt; IJ

This is a Control usage request, and serves to setup an authorization group. Changing the Control Words for a service is expected to be a relatively rare occurrence. This request allows the encryption group to be downloaded and then used by subsequent encrypted_DPI requests. This message will replace any existing Control Words in the specified index position.

In some architectures, the control of encryption services may be done by the PAMS rather than the AS. In these cases, this message would not be used, since it would overwrite the Control Words downloaded by the system controller. The automation system may still need to know which messages are to be encrypted and which CW_index to assign to specific messages. The mechanism for doing so is not defined in this document.

Table 9-9: update_ControlWord_data

|  Syntax | Bytes | Type  |
| --- | --- | --- |
|  update_ControlWord_data() { |  |   |
|  CW_index | 1 | uimsbf  |
|  CW_A | 8 | uimsbf  |
|  CW_B | 8 | uimsbf  |
|  CW_C | 8 | uimsbf  |

AMERICAN NATIONAL STANDARD ©2022 SCTE

ANSI/SCTE 104 2023

## 9.4.3.1. Semantics of fields in update_ControlWord_data()

CW Index – This field specifies the control word index used to reference the control word database. This field *may* range from 0 to 255. The index sent indicates which of the 256 Control Word set *should* be replaced in the Injector’s Control Word database.

Each Control Word set is 3 64-bit numbers. The two Single DES encryption modes only use CW_A, while Triple-DES requires all 3 64-bit Control Words. All 3 fields are always sent, but if Triple-DES is not used, CW_B and CW_C *shall* be zeros.

CW_A – ControlWord_A, a 64-bit value which is always used. In the case of the two Single DES encryption modes, CW_A is used alone (CW_B and CW_C are zero filled), while Triple-DES requires all 3 64-bit control words.

CW_B – The second 64-bit number sent as a Control Word. This value is normally zero unless Triple-DES encryption is utilized, in which case it carries the second of the three control word values.

CW_C – The third 64-bit number sent as a Control Word. This value is normally zero unless Triple-DES encryption is utilized, in which case it carries the third of the three control word values.

## 9.4.4. delete_ControlWord request AS ==&gt; IJ

This is a Control usage request. If an Encryption Group is no longer required, then this request can be sent to remove the Control Words from the Injector’s database. This is only really necessary if one wishes to prevent messages from being sent with this Control Word, since empty Control Word index slots results in an alarm if an attempt is made to use it.

The Injector *shall not* produce an alarm if an undefined Control Word is deleted. This allows the AS to delete all control words without actually knowing what Control Words are present, so the Control Word database can be reinitialized.

In some architectures, the control of encryption services *may* be done by the PAMS rather than an automation controller. In these cases, this message would not be used, since it would delete the control words downloaded by the PAMS.

Table 9-10: delete_ControlWord_data

|  Syntax | Bytes | Type  |
| --- | --- | --- |
|  delete_ControlWord_data() { CW_index} | 1 | uimsbf  |

## 9.4.4.1. Semantics of fields in delete_ControlWord_data()

CW Index – This field specifies the control word index used to reference the control word database. This field ranges from 0 to 255.

AMERICAN NATIONAL STANDARD ©2022 SCTE

ANSI/SCTE 104 2023

## 9.5. Component Mode Support

### 9.5.1. component mode DPI request

The component mode DPI request is used for applications that wish to splice into some of the elementary streams of a program, and not others. This is an advanced method of DPI control that requires detailed knowledge of the structure of the program elements that exists in the same program as this DPI splice_info_section.

It is a Supplemental type request (See Section 8.3.1) and must follow the splice_request_data() for which it applies within the data() structure of the multiple_operation_message (See Section 8.2.3).

The presence of this request changes fundamental syntactic elements in the resulting SCTE 35 [SCTE35] splice_info_section() as the request will force component mode rather than program mode operation in the splicer.

Table 9-11: component_mode_DPI_request_data

|  Syntax | Bytes | Type  |
| --- | --- | --- |
|  component_mode_DPI_request_data() { for(i=0; i<N; i++) { component_tag component_preroll } | 1 | uimsbf  |
|   | 2 | uimsbf  |

### 9.5.1.1. Semantics of fields in component_mode_DPI_request_data()

component_tag – This field contains the associated component tag for one of the elementary streams to be spliced. The loop provides a complete list of spliced elementary streams and the time at which the splice should occur.

component_preroll – The overall request timestamp provides the exact time to process the message. In component mode, each component (i.e. Elementary stream PID) has a unique time at which its splice is to occur. The actual SCTE 35 [SCTE35] timestamp can be calculated by adding the pre-roll time to the timestamp() reference point.

When operating in component mode splicing, the value of pre_roll_time given in the corresponding splice_request message is not used.

This field is expressed in milliseconds.

AMERICAN NATIONAL STANDARD ©2022 SCTE

ANSI/SCTE 104 2023

## 9.6. Response Messages

### 9.6.1. general_response message IJ ==&gt; AS

The general_response message conveys back a result code. This is a basic message.

**Table 9-12: general_response_data**

|  Syntax | Bytes | Type  |
| --- | --- | --- |
|  general_response_data() { |  |   |

This response message is sent following the receipt of the following messages:

**Table 9-13: general responses**

|  Request | Description  |
| --- | --- |
|  update_ControlWord | This allows the AS to download a new CW for use in encrypted messages.  |
|  delete_Control_Word | This allows the AS to delete an active CW. Once deleted, an Injector can flag an error if any attempt is made to use it.  |

### 9.6.2. inject_response message IJ ==&gt; AS

The inject_response message conveys back the message_number from the multiple_operation_message() structure (Section 8.2.3) to which it is responding. This message can contain a result code if appropriate. This is a basic message.

A Proxy Device may respond with a “Proxy Response” result code (see Table 14-1). This permits the Automation System, should it desire to do so, to track whether or not a given Injector is served by a Proxy Device or a direct connection.

**Table 9-14: inject_response data**

|  Syntax | Bytes | Type  |
| --- | --- | --- |
|  inject_response_data() { message_number } | 1 | uimsbf  |

### 9.6.2.1. Semantics of fields in inject_response_data()

message_number – The message_number of the multiple_operation_message() that is being acknowledged.

The inject_response message is sent following the receipt of the following messages:

AMERICAN NATIONAL STANDARD ©2022 SCTE

ANSI/SCTE 104 2023

Table 9-15: inject_responses

|  Request | Description  |
| --- | --- |
|  splice_request | Acknowledgement for splice_request – returned to the AS immediately to acknowledge receipt of the command  |
|  time_signal_request | Acknowledgement for time signal request – returned to the AS immediately to acknowledge receipt of the command  |
|  splice_null_request | Acknowledgement for splice null request – returned to the AS immediately to acknowledge receipt of the command  |
|  proprietary_command_request | Acknowledgement for proprietary command request – returned to the AS immediately to acknowledge receipt of the command  |
|  start_schedule_download_request | Indicates to an Injector that it should start collecting schedule information.  |
|  schedule_definition_request | Used to download a single schedule entry into the Injector’s database.  |
|  schedule_component_mode request | Used as a supplemental command for Schedule Definition to indicate that a component splice is being scheduled.  |
|  transmit_schedule_request | The Automation System uses this command to tell an Injector to send the accumulated schedule information.  |

## 9.6.3. inject_complete_response  IJ ==&gt; AS

The inject_complete_response message is sent once when the Injector finishes issuing all SCTE 35 [SCTE35] splice_info_sections for a given Normal request operation and conveys back the message_number from the multiple_operation_message() structure (Section 8.2.3) to which it is responding. If a Normal request does not result in the issuing of any SCTE 35 splice_info_sections, then this response is not sent. The value of the message_number variable is now free to be re-used.

A single inject_complete_response message is sent regardless of the number of operations contained within a given multiple_operation_message() structure. The inject_complete_response message contains a count which indicates the number of SCTE 35 splice_info_sections issued by Injector in response to the previous splice_request. A result value of “Successful Response” will normally be expected for this message. See Table 14-1 for the various result codes.

AMERICAN NATIONAL STANDARD ©2022 SCTE

ANSI/SCTE 104 2023

Table 9-16: injectcomplete response data

|  Syntax | Bytes | Type  |
| --- | --- | --- |
|  injectcomplete_response_data() { message_number cue_message_count} | 1 | uimsbf  |
|   | 1 | uimsbf  |

A Proxy Device may respond with a "Proxy Response" result code (see Table 14-1). This permits the Automation System, should it desire to do so, to track whether or not a given Injector is served by a Proxy Device or a direct connection.

# 9.6.3.1. Semantics of fields in injectcomplete_response_data()

message_number - message number of the multiple_operation_message() that has completed processing.

cue_message_count - this an integer value that specifies the count of SCTE 35 [SCTE35] splice_infoSections sent by Injector. This value may be logged by the Automation System if desired. The Injector will clear the cue_message_count after each injectcomplete_response is sent to the Automation System.

The injectcomplete_response message is sent following the injection the SCTE 35 [SCTE35] section in response to the following messages:

Table 9-17: injectcomplete Responses

|  Request | Description  |
| --- | --- |
|  splice_request | Acknowledgement for splice request – returned after the DPI message has been injected into the transport. May be returned immediately after the Splice Response if immediate mode timing is used. May be delayed if time stamped processing is used.  |
|  time_signal_request | Acknowledgement for Time Signal request – returned after the DPI message has been injected into the transport. May be returned immediately after the Splice Response if immediate mode timing is used. May be delayed if time stamped processing is used.  |
|  splice_null_request | Acknowledgement for Splice Null request – returned after the DPI message has been injected into the transport. May be returned immediately after the Splice Response if immediate mode timing is used. May be delayed if time stamped processing is used.  |
|  proprietary_command_request | Acknowledgement for Proprietary Command request – returned after the DPI message has been injected into the transport. May be returned immediately after the Splice  |

AMERICAN NATIONAL STANDARD ©2022 SCTE

ANSI/SCTE 104 2023

|   | Response if immediate mode timing is used. May be delayed if time stamped processing is used.  |
| --- | --- |
|  transmit_schedule_request | Indicates the schedule data has been has been injected into the transport  |

## 9.7. SCTE 35 splice_schedule() Support Requests

The DPI schedule requests may exist in multiple sections within a transport. Each section contains a descriptor loop. All sections of a given schedule will contain the exact same descriptors.

If the avail descriptor is to be present, then it is filled from the data provided in the start schedule download request. This allows each section to be built as the data is being downloaded.

If other descriptors are to be present, those requests follow in data(), and the insert_descriptor requests must be present in the same message that carries this request. Those descriptors will then be duplicated in each real section generated. The Injector must have enough memory to hold the descriptors as well as the schedule data.

## 9.7.1. start schedule download request  $AS ==&gt; IJ$

The SCTE 35 [SCTE35] standard allows for a schedule of avail times to be broadcast. This request readies the Injector to accept one or more schedule_definition_data() requests prior to transmission. Since a schedule can potentially have a large amount of data, provision has been made to download the data in smaller pieces.

The start schedule download message permits generation of a SCTE 35 [SCTE35] avail_descriptor. It is a Normal type message. The Injector must allocate sufficient memory to permit the accumulation of the maximum amount of section data specified by SCTE 35 [SCTE35]. A splice_request is not required in conjunction with splice_schedule.

If the schedule request is intended to be encrypted before being sent, then the Encrypted_DPI_data() structure must be included in the same multiple_operation_message data() structure (See Section 8.2.3) as this start_schedule_download_data() structure. In this case ONLY, it must be placed in the data()structure before the start_schedule_download_data() structure is placed in the data() structure. By setting up the encryption before downloading the data, any intermediate sections that might be created can also be encrypted.

The SCTE 35 [SCTE35] splice_info_section() structure only allows one descriptor loop for an entire schedule splice_info_section. Therefore, any Supplemental requests that generate descriptors must be attached to the Start Schedule Request. These descriptors will then be inserted in all splice_info_section generated as a result of the schedule download.

AMERICAN NATIONAL STANDARD ©2022 SCTE

ANSI/SCTE 104 2023

Table 9-18: start_schedule_download_request_data

|  Syntax | Bytes | Type  |
| --- | --- | --- |
|  start_schedule_download_request_data() { |  |   |
|  num_provider_avails | 1 | uimsbf  |
|  for (i=0 ;i< num_provider_avails; i++) { |  |   |
|  provider_avail_id | 4 | uimsbf  |
|  } |  |   |

## 9.7.1.1. Semantics of fields in start_schedule_download_request_data()

num_provider_avails – If this field is zero, then the provider avail id field is not being used and the value should be ignored and no avail_descriptor will be created.

If this field is non-zero, then the provider avail id field(s) must contain valid data.

provider_avail_id – This is an optional 32-bit number which will be inserted into the SCTE 35 [SCTE35] splice_info_section() avail_descriptor.

Please refer to Section 8.3.1 of SCTE 35 [SCTE35] for more information.

## 9.7.2. schedule definition request  $AS ==&gt; IJ$

This request allows a single avail definition to be collected by the Injector. Using the overall message structure, it is possible to deliver multiple splice point definitions in the same resultant splice_info_section(). This request will be issued once per splice event to be included in that splice_info_section().

A splice definition being transmitted must be contained within a SCTE 35 [SCTE35] splice_info_section() structure. This section has a limited size of 4096 bytes, although some implementations may have lower maximum sizes. If a schedule being transmitted exceeds the local maximum memory allocated, it is possible that the first resultant section could be formatted, packetized, and placed in the Transport Stream before the transmit_schedule request is sent to force transmission and thus make space for more schedule data in the local memory of the Injector.

This is a Supplemental request and must follow a start_schedule_download_data() in the data() structure of a multiple_operation_message().

AMERICAN NATIONAL STANDARD ©2022 SCTE

ANSI/SCTE 104 2023

Table 9-19: schedule_definition_data

|  Syntax | Bytes | Type  |
| --- | --- | --- |
|  schedule_definition_data() { |  |   |
|  splice_schedule command | 1 | uimsbf  |
|  splice_event_id | 4 | uimsbf  |
|  time() | 4 |   |
|  unique_program_id | 2 | uimsbf  |
|  auto_return | 1 | uimsbf  |
|  break_duration | 2 | uimsbf  |
|  avail_num | 1 | uimsbf  |
|  avails_expected | 1 | uimsbf  |

## 9.7.2.1. Semantics of fields in schedule_definition_data()

splice_schedule command – This field indicates if the associated SCTE 35 [SCTE35] splice_schedule() section generated will be a splice insert (away from the network) or a splice return to the network. A cancellation may also be signaled.

Table 9-20: splice_schedule command type Assigned Values

|  splice_schedule_command_type | Value assigned  |
| --- | --- |
|  reserved | 0  |
|  splice_insert | 1  |
|  reserved | 2  |
|  splice_return | 3  |
|  reserved | 4  |
|  splice_cancel | 5  |

splice_event_id – This is a 32-bit number that will be coded into the splice_event_id in the final SCTE 35 [SCTE35] splice_info_section.

time() – See Section 12.4. A 32-bit unsigned integer quantity representing the time of the signaled splice event as the number of seconds since 00 hours UTC, January 6, 1980, with the count of intervening leap seconds included. See RFC 1305 [RFC1305] for further information.

unique_program_id – This is a 16-bit field as defined by SCTE 35 [SCTE35].

AMERICAN NATIONAL STANDARD ©2022 SCTE

ANSI/SCTE 104 2023

auto_return – If this field is non-zero, then the auto_return field in the resulting break_duration() of the SCTE 35 [SCTE35] section will be set to one.

break_duration – A 16-bit field giving the duration of the insertion in tenths of seconds. If break_duration is set to zero, then the resulting SCTE 35 [SCTE35] splice_schedule() section will not include the break_duration() and the flags auto_return and duration_flag will be set to zero.

avail_num – This is an 8-bit number indicating which avail within the program is currently being described (see SCTE 35 [SCTE35]). It will be coded as a decimal number from 1 to 255. A value of zero indicates that the avail fields are not being used. If this field is coded as zero, so should the avails_expected field.

avails_expected – This is an 8-bit number indicating how many avails to expect within the program currently being described (see SCTE 35 [SCTE35]). It will be coded as a decimal number from 1 to 255. A value of 0 indicates that the avail fields are not being used. If this field is coded as zero, so should the avail_num field.

## 9.7.3. The schedule component mode request  $AS ==&gt; IJ$

The schedule_component_mode request is used for applications that wish to splice into some of the elementary streams of a program, and not others. This is an advanced method of DPI control that requires detailed knowledge of the structure of the program elements that exists in the same program as this DPI message. If component mode is used for a specific avail, then this structure may be delivered along with the associated schedule_definition_data() structure to define the components that will be spliced in that avail.

Table 9-21: schedule_component_request_mode

|  Syntax | Bytes | Type  |
| --- | --- | --- |
|  schedule_component_mode_request_data() { for(i=0; i<N; i++) { component_tag time() } | 1 * | uimsbf  |

## 9.7.3.1. Semantics of fields in schedule_component_mode_request_data()

component_tag – This field contains the associated component tag for one of the elementary streams to be spliced. The loop provides a complete list of spliced elementary streams and the time at which the splice should occur.

time() – See Section 12.4. A 32-bit unsigned integer quantity representing the time of the signaled splice event as the number of seconds since 00 hours UTC, January 6, 1980, with the count of intervening leap seconds included. See RFC 1305 [RFC1305] for further information.

AMERICAN NATIONAL STANDARD ©2022 SCTE

---

ANSI/SCTE 104 2023

## 9.7.4. transmit_schedule request

This is a Normal usage request. When this request is processed, any schedule data saved in local memory is packetized and transmitted at the time indicated.

A downloaded schedule is not remembered after it has been transmitted, and the Injector may immediately free-up allocated local memory. The automation device is responsible for retransmitting up-to-date schedule information when required.

Table 9-22: transmit_schedule_request_data

|  Syntax | Bytes | Type  |
| --- | --- | --- |
|  transmit_schedule_request_data() { cancel } | 1 | uimsbf  |

## 9.7.4.1. Semantics of fields in transmit_schedule_request_data()

cancel – This flag is used to cancel any downloaded data and abort the transmission of the schedule in progress. A value of zero is normal, and indicates that the downloaded data can be transmitted at the time that the timestamp indicates. Any non-zero value indicates that the download should be cancelled.

If this request is cancelled before being processed, then the entire schedule downloaded is also discarded. The effect is the same as if this request was sent with the cancel bit set.

## 9.8. Miscellaneous Requests

### 9.8.1. time_signal request  $AS ==&gt; IJ$

This is a Normal request which will be generated and transmitted at the time indicated by the timestamp() field of the multiple_operation_message() structure. This request will normally be accompanied by one or more insert_descriptor requests.

Table 9-23: time_signal_request_data

|  Syntax | Bytes | Type  |
| --- | --- | --- |
|  time_signal_request_data() { pre-roll_time } | 2 | uimsbf  |

### 9.8.1.1. Semantics of fields in time_signal_request_data()

pre-roll_time – The splice splice_info_section may be sent by the automation system well in advance of when it is required. In order to support repeated sending of the same splice_info_section and to support multiple sections being outstanding simultaneously, this request supports the preloading of its parameters. The timestamp() indicates the time to process the splice_info_section. The pre-roll field indicates the

AMERICAN NATIONAL STANDARD ©2022 SCTE

ANSI/SCTE 104 2023

amount of time, in milliseconds, after being processed that the action will occur. For the time_signal_request() this is the pre-roll for the associated descriptors. If this request arrives after the indicated time, the splice_info_section is sent as soon as possible.

The timestamp field can indicate immediate processing (and therefore uses relative timing) or Deferred processing (which uses exact timing). In all cases, the signaling point is calculated relative to the time the Request is processed. The pre-roll field determines the exact delay period for the splice point relative to the Request being processed.

If this Request is processed immediately on arrival, then the physical insertion of the time signal request is as soon as it is received.

In the case of an exact timestamp using a UTC, VITC1 or GPI triggering2, the Request is processed at the indicated time.

In the case when a component mode request is used to modify this basic request, the overall pre-roll time is not used. That is, this field is only used when the DPI splice_info_section produced is for a program mode splice. For component mode splicing, each component will have its own time stamp.

## 9.8.2. splice null request

This is a Normal usage request. When this request is processed, an SCTE 35 [SCTE35] splice_null() splice_info_section will be generated and transmitted at the time indicated by the timestamp field. This request will normally be accompanied by one or more insert_descriptor requests.

Table 9-24: splice_null_request_data

|  Syntax | Bytes | Type  |
| --- | --- | --- |
|  splice_null_request_data() { |  |   |

## 9.8.3. inject section data request  $AS ==&gt; IJ$

This is a Normal usage request. When this request is processed, the image will be copied into the command structure of the associated SCTE 35 [SCTE35] splice_info_section being created. Some Supplemental requests, such as an insert descriptor request or encrypted_DPI request may be used with this request.

AMERICAN NATIONAL STANDARD ©2022 SCTE

ANSI/SCTE 104 2023

Table 9-25: inject_section_data_request

|  Syntax | Bytes | Type  |
| --- | --- | --- |
|  inject_section_data_request() { |  |   |
|  SCTE35_command_length | 2 | uimsbf  |
|  SCTE35_protocol_version | 1 | uimsbf  |
|  SCTE35_command_type | 1 | uimsbf  |
|  SCTE35_command_contents() | * |   |

## 9.8.3.1. Semantics of fields in inject_section_data_request()

SCTE35_command_length – This field encodes the number of bytes in the SCTE35_command_contents() structure.

SCTE35_protocol_version – When the SCTE 35 [SCTE35] splice_info_section() is created, the protocol version field in the Splice Info Section will be filled in with this value. This could allow a compatible method of delivering commands defined in future revisions of SCTE 35 [SCTE35] using older versions of this protocol.

SCTE35_command_type – This field will fill in the value of the splice_command_type field in the SCTE 35 [SCTE35] splice_info_section() being created.

SCTE35_command_contents() – This is a complete binary image of the SCTE 35 [SCTE35] splice_info_section() being created, following the splice_command_type field up to, but not including, the descriptor_loop_length field.

## 9.8.4. insert_avail_descriptor request AS ==&gt; IJ

This is a Supplemental usage request. When this request is processed, an avail_descriptor() shall be added to the descriptor loop of the associated SCTE 35 [SCTE35] splice_info_section being created. The Normal request to which it applies must exist earlier in the same data() buffer.

AMERICAN NATIONAL STANDARD ©2022 SCTE

ANSI/SCTE 104 2023

Table 9-26: insert_avail_descriptor_request_data

|  Syntax | Bytes | Type  |
| --- | --- | --- |
|  insert_avail_descriptor_request_data() { |  |   |
|  num_provider_avails | 1 | uimsbf  |
|  for (i=0 ;i< num_provider_avails; i++) { |  |   |
|  provider_avail_id | 4 | uimsbf  |
|  } |  |   |

## 9.8.4.1. Semantics of fields in insert_avail_descriptor_request_data()

num_provider_avails – If this field is zero, then the provider_avail_id field is not being used and the value shall be ignored.

If this field is non-zero, then the num_provider_avails field is the repetition count for the provider_avail_id field. Also, the Injector must include an avail_descriptor() in the DPI splice_info_section created.

provider_avail_id – This is an optional 32-bit field which may be inserted into the resulting SCTE 35 [SCTE35] splice_info_section. If the value of num_provider_avails is zero, this field shall be ignored and no avail_descriptor() shall be created.

## 9.8.5. insert_descriptor request  $AS ==&gt; IJ$

This is a Supplemental usage request. When this request is processed, the descriptor image will be copied into the descriptor loop of the associated SCTE 35 [SCTE35] splice_info_section being created. One of the Normal requests must exist earlier in the same data() buffer and these descriptors will be added to any SCTE 35 [SCTE35] section generated by that Normal request.

AMERICAN NATIONAL STANDARD ©2022 SCTE

ANSI/SCTE 104 2023

Table 9-27: insert_descriptor_request_data

|  Syntax | Bytes | Type  |
| --- | --- | --- |
|  insert_descriptor_request_data() { |  |   |
|  descriptor_count | 1 | uimsbf  |
|  for(i=0; i< descriptor_count ; i++) { |  |   |
|  descriptor_image() | * |   |
|  } |  |   |
|  } |  |   |

## 9.8.5.1. Semantics of fields in insert_descriptor_request_data()

descriptor_count – This field encodes the number of descriptors following.

descriptor_image – This field carries a complete image of a standard SCTE 35 [SCTE35] descriptor, which follows MPEG-2 rules and has its length as the second byte of the descriptor. This request is used to inject proprietary, or future standard descriptors into a request without need for specific knowledge of the contents of the descriptor to be injected. For standard descriptors, the recommended method is to update this protocol to include a request for the new descriptor.

## 9.8.6. insert_DTMF_descriptor request AS ==&gt; IJ

This is a Supplemental usage request. This request creates an image of the DTMF descriptor defined in SCTE 35 [SCTE35]. Refer to SCTE 35 [SCTE35] for details of each field in the descriptor.

One specific note about this descriptor. The pre-roll field found in this descriptor is intended to be the same value as that used for the associated splice_request. The DTMF descriptor allows for tenths of a second resolution, and the splice_request allows millisecond resolution. One should ensure that both requests use the same pre-roll value to provide a consistent program insertion on both analog and digital systems.

AMERICAN NATIONAL STANDARD ©2022 SCTE

ANSI/SCTE 104 2023

Table 9-28: insert_DTMF_descriptor_request_data

|  Syntax | Bytes | Type  |
| --- | --- | --- |
|  insert_DTMF_descriptor_request_data() { |  |   |
|  pre-roll | 1 | uimsbf  |
|  dtmf_length | 1 | uimsbf  |
|  for(i=0; i<dtmf_length; i++) { |  |   |
|  DTMF_char | 1 | uimsbf  |
|  } |  |   |

## 9.8.6.1. Semantics of fields in insert_DTMF_descriptor_request_data()

pre-roll – Refer to SCTE 35 [SCTE35] for detail usage of this field.

The pre-roll time encodes the number of tenths of seconds before the splice_point signaled in the resulting SCTE 35 [SCTE35] section that a DTMF tone sequence should finish being emitted. To allow for processing time, the pre-roll signaled in the SCTE 35 [SCTE35] message should be greater than this value.

dtmf_length – This indicates the length of the following loop in bytes.

DTMF_char – This field carries one character of a DTMF sequence to be output by an IRD. This field should contain one of the ASCII characters ‘0’ through ‘9’, ‘*’, ‘#’, and ‘A’ through ‘D’. Refer to SCTE 35 [SCTE35] for detailed usage of this field.

## 9.8.7. insert_segmentation_descriptor request  $AS ==&gt; IJ$

This is a Supplemental usage request, and creates a Segmentation descriptor defined in SCTE 35 [SCTE35]. Refer to SCTE 35 [SCTE35] for details of each field in the descriptor. The program_segmentation_flag shall be set to one in the resulting SCTE 35 [SCTE35] splice_info_section(). If the user needs to support component mode segmentation, then an insert_descriptor request should be used to directly format this descriptor.

AMERICAN NATIONAL STANDARD ©2022 SCTE

ANSI/SCTE 104 2023

Table 9-29: insert_segmentation_descriptor_request_data

|  Syntax | Bytes | Type  |
| --- | --- | --- |
|  insert_segmentation_descriptor_request_data() { |  |   |
|  segmentation_event_id | 4 | uimsbf  |
|  segmentation_event_cancel_indicator | 1 | uimsbf  |
|  duration | 2 | uimsbf  |
|  segmentation_upid_type | 1 | uimsbf  |
|  segmentation_upid_length | 1 | uimsbf  |
|  segmentation_upid | varies | uimsbf  |
|  segmentation_type_id | 1 | uimsbf  |
|  segment_num | 1 | uimsbf  |
|  segments_expected | 1 | uimsbf  |
|  duration_extension Frames | 1 | uimsbf  |
|  delivery_not_restricted_flag | 1 | uimsbf  |
|  webdelivery_allowed_flag | 1 | uimsbf  |
|  no_regional/blackout_flag | 1 | uimsbf  |
|  archive_allowed_flag | 1 | uimsbf  |
|  device_restrictions | 1 | uimsbf  |
|  insert_sub_segment_info | 1 | uimsbf  |
|  sub_segment_num | 1 | uimsbf  |
|  sub_segments_expected | 1 | uimsbf  |

# 9.8.7.1. Semantics of fields in insert_segmentation_descriptor_request_data()

segmentation_event_id - A four byte (32-bit) unique segmentation event identifier.

segmentation_event_cancel_indicator - A one byte flag that when set to '1' indicates that a previously sent segmentation event, identified by segmentation_event_id, has been cancelled.

AMERICAN NATIONAL STANDARD ©2022 SCTE

ANSI/SCTE 104 2023

duration - A two byte (16-bit) field giving the duration of the program segment in whole seconds. A zero value is legal and results in the segmentation_duration_flag in the resulting SCTE 35 [SCTE35] section being set to '0'. See duration_extension_frames.

segmentation_upid_type - A one byte field that specifies the type of "UPID" utilized in this program. There are multiple types allowed to insure that programmers will be able to use an id that their systems support. Refer to SCTE 35 [SCTE35] for full details.

segmentation_upid_length - A one byte field that specifies the length in bytes of the segmentation_upid. If there is no segmentation_upid data, segmentation_upid_length shall be set to 0.

segmentation_upid - A variable-length field that specifies the "UPID" value for this segment. Refer to SCTE 35 [SCTE35] for details.

segmentation_type_id - A one byte field which designates type of segmentation and takes values specified in SCTE 35 [SCTE35].

segment_num - A one byte field that provides identification for a specific segment within a collection of segments. Refer to SCTE 35 [SCTE35] for full details.

segments_expected - A one byte field that provides a count of the expected number of individual segments within a collection of segments.

duration_extension_frames - A one byte field that shall carry a value in the range from 0 to the value of the greatest integer less than frame rate, which shall be the number of frames in the fractional second not included in duration plus one. The total duration of the program segment is duration seconds plus duration_extension_frames frame times. If duration is 0 this field carries no meaning.

Note: In SCTE 35 [SCTE35], content length is described in terms of the number of ticks of a 90 kHz MPEG counter. A value in these units is calculated from duration and duration_extension_frames by converting duration using the Section titled "Conversion of SMPTE ST 12-1 Time-Address Value to Local Wall Clock Time" of SMPTE EG40 [SMPTE_EG40], converting duration_extension_frames using Section titled "Conversion of Local Wall Clock Time to MPEG-2 PCRtb Value" of SMPTE EG40 [SMPTE_EG40], and adding the resulting values. It is vital that implementers reference the latest published edition of SMPTE EG40 [SMPTE_EG40].

delivery_not_restricted_flag - A one byte flag that when set to 1 indicates there is no need for external checks prior to delivery. A value of 0 indicates the content requires external checks. Refer to SCTE 35 [SCTE35] for full details.

web_delivery_allowed_flag - A one byte flag that when set to 1 indicates web delivery is allowed. Refer to SCTE 35 [SCTE35] for full details.

no_regional_blackout_flag - A one byte flag that when set to 1 indicates there is not a regional blackout. Refer to SCTE 35 [SCTE35] for full details.

archive_allowed_flag - A one byte flag that when set to 1 indicates the content is archiveable. Refer to SCTE 35 [SCTE35] for full details.

device_restrictions - A one byte field which designates type of segmentation and takes values specified in SCTE 35 [SCTE35].

AMERICAN NATIONAL STANDARD ©2022 SCTE

ANSI/SCTE 104 2023

insert_sub_segment_info - A one byte flag that indicates whether sub_segment_num and sub_segments_expected are included in the resultant SCTE 35 [SCTE35] segmentation descriptor. If the value is 1, the values shall be inserted. If the value is 0 or null, the values shall not be inserted. insert_sub_segment_info is not passed in the resultant SCTE 35 [SCTE35] segmentation descriptor.

sub_segment_num - A one byte field that provides identification for a specific sub-segment within a collection of segments. Refer to SCTE 35 [SCTE35] for full details.

sub_segments_expected - A one byte field that provides a count of the expected number of individual sub-segments within a collection of sub-segments.

Note: insert_sub_segment_info, sub_segment_num and sub_segments_expected can form an optional appendix to the segmentation descriptor. The presence or absence of this optional data block is determined by the descriptor loop's data_length.

# 9.8.8. proprietary_command request AS ==&gt; IJ

This is a Normal usage request, and allows for proprietary extension to the protocol. The data_length field functions in the normal manner for the data() loop within the context of multiple_operation_message().

The opID variable for the proprietary_command_data() is one of the values defined in Table 8-3 for user defined requests. In addition to using this opID value, each company that wishes to define proprietary SCTE 35 [SCTE35] commands should register with SMPTE-RA [SMPTE_RA] for a proprietary id value (see SCTE 35 [SCTE35] Section 9.3.6). This permits the company to create one or more proprietary commands that are uniquely theirs, each identified by their respective proprietary_command_data() structure.

The data_length field in multiple_operation_message() (See Section 8.2.3) the must be correctly set to reflect the number of bytes utilized by the remainder of the request which follows the data_length field itself. Failure to do so will result in the commands not being processed correctly.

Table 9-30: proprietary_command_request_data

|  Syntax | Bytes | Type  |
| --- | --- | --- |
|  proprietary_command_request_data() { |  |   |
|  proprietary_id | 4 | uimsbf  |
|  proprietary_command | 1 | uimsbf  |
|  for (i=0; i<data_length-5; i++) { |  |   |
|  proprietary_data() | * |   |
|  } |  |   |
|  } |  |   |

AMERICAN NATIONAL STANDARD ©2022 SCTE

ANSI/SCTE 104 2023

## 9.8.8.1. Semantics of fields in proprietary_command_request_data()

proprietary_id – This number is a 32-bit identifier that has been registered with SMPTE-RA [12] for a specific company. The contents of the command and the definition of how to process the command are proprietary. All definitions are beyond the scope of this document.

proprietary_command – This is a field, similar to the opID tag, which identifies individual proprietary commands for each proprietary id. The meaning of this field is not defined, but must follow the basic rules for the protocol.

proprietary_data() – This is a variable length field that contains the data for the specific proprietary command. The amount of data contained in the command can be determined from the overall length field for this command.

The definition for this data is not specified, but it must follow the basic rules for the protocol.

## 9.8.9. insert_tier_data request AS ==&gt; IJ

This is a Supplemental usage request. When this request is processed, the tier value shall be copied into the associated SCTE 35 [SCTE35] splice_info_section being created. One of the Normal requests shall be placed earlier in the same data() buffer and this value will be added to the SCTE 35 [SCTE35] section generated by that Normal request. If this request is missing, the Injector shall insert the value of 0xFFF into the tier field in the associated SCTE 35 [SCTE35] splice_info_section being created.

Table 9-31: insert_tier_data

|  Syntax | Bytes | Type  |
| --- | --- | --- |
|  insert_tier_data() { tier_data} | 2 | uimsbf  |

## 9.8.9.1. Semantics of fields in insert_tier_data()

tier_data – A field with the most significant nibble set to 0x0 and containing, in the lower 12-bits, a value with semantics as specified in SCTE 35 [SCTE35] for “tier.”

## 9.8.10. insert_time_descriptor request AS ==&gt; IJ

This is a Supplemental usage request. When this request is processed, the time_descriptor() shall be associated with the Normal request that shall have been placed earlier in the same data() buffer and this structure will be added the SCTE 35 [SCTE35] section generated by that Normal request.

The request requires the AS to supply an exact PTP [TAI] sample to be inserted in the resulting message (see IEEE 1588 [IEEE1588]).

Per SCTE 35 [SCTE35], this request may be associated with splice_insert(), splice_null() and time_signal() requests. The injector will make no effort to verify that no other Normal request is being used.

AMERICAN NATIONAL STANDARD ©2022 SCTE

ANSI/SCTE 104 2023

Table 9-32: insert_time_descriptor

|  Syntax | Bytes | Type  |
| --- | --- | --- |
|  insert_time_descriptor() { |  |   |
|  TAI_seconds | 6 | uimsbf  |
|  TAI_ns | 4 | uimsbf  |
|  UTC_offset | 2 | uimsbf  |
|  } |  |   |

## 9.8.10.1. Semantics of fields in insert_time_descriptor()

TAI_seconds – Per SCTE 35 [SCTE35] Table 27, time_descriptor().

TAI_ns – Per SCTE 35 [SCTE35] Table 27, time_descriptor().

UTC_offset – Per SCTE 35 [SCTE35] Table 27, time_descriptor().

## 9.8.11. insert_audio_descriptor request AS ==&gt; IJ

This is a Supplemental usage request. When this request is processed, the audio_descriptor() shall be associated with the Normal request that shall have been placed earlier in the same data() buffer and this structure will be added the SCTE 35 [SCTE35] section generated by that Normal request.

Per SCTE 35 [SCTE35], this request may be associated with splice_insert(), splice_null() and time_signal() requests. The injector will make no effort to verify that no other Normal request is being used.

AMERICAN NATIONAL STANDARD ©2022 SCTE