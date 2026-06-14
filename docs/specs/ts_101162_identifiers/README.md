# ETSI TS 101 162 v1.9.1 — Allocation of DVB SI identifiers

Source: specs/etsi_ts_101_162_v01.09.01_dvb_identifiers.pdf (PDF pp.12-44). NB: this is a registration-PROCESS spec (per-identifier registration/allocation templates + domain-name registries); live value allocations are maintained at the DVB online registry, not fixed here.

> Wire-structure reference, table-per-file for deep-linking. Each linked file
> carries one syntax/enum table **plus its field semantics** — enough to drive a
> spec-accurate Rust parser (symmetric Parse/Serialize; coded enums get TOML
> drift-guards when implemented). Transcribed via BlazeDocs (table oracle; not
> pdftotext), spot-checked vs the PDF render. No parser implemented yet.

## Carriage / overview

URI Uniform Resource Identifier
XML Extensible Markup Language

# 4 Principles of registration

## 4.0 Requirements and process of registration

The present document defines the allocation of identifiers pertaining to different DVB specifications (e.g. MHP, SI, Data Broadcasting, etc.). It does not describe the detail or the template as to how this should be done. The aim of the present document is to provide assistance to those soliciting and allocating identifiers.

Each identifier has the following attributes:

1) It is defined in a DVB specification (e.g. DVB Service Information (ETSI EN 300 468 [i.1])).
2) It is either:

a) a binary number represented by either its hexadecimal equivalent denoted by the prefix "0x", or its decimal equivalent;
b) a string constant represented by its Unicode equivalent; or
c) a combination of a binary number and a string constant.

3) It has a text description. It is the table of values and descriptions which is published on www.dvb.org.
4) It is allocated to an organization operating in the digital television space (e.g. ACME Digital Broadcasting, Inc.), or a grouping of such companies (e.g. an ACME - Association of Cable/MMDS Enterprises) or an institution acting in digital television, e.g. IEEE (Institute of Electrical and Electronic Engineers).
5) It may be allocated for a given region. For terrestrial broadcasting, this is typically a sovereign country; for satellite operations, this is typically a geographical region spanning many countries, but consistent with the footprint of the satellites owned by the operators.

The present document describes where to find definitions of each identifier, who to refer to when there are questions, templates for the allocations and rules governing them. In addition, and where appropriate, there are descriptions of best practice and some historical notes.

The DVB Project Office shall be the only Registrar entitled to accept applications and perform registrations under the regime of the present document, and within the application areas of the specifications listed in clause 2. The DVB Project Office shall maintain a public, on-line register of assigned identifiers to ease quick look-up of the current assignments.

NOTE: For practical reasons, the DVB Project Office may choose to delegate the operation and maintenance of the public, on-line register and the authority of receiving applications and performing registrations to one or more third parties.

## 4.1 Registration domain and application domains

The scope of the present document shall constitute a registration domain namespace. Referred to as the registration domain for short. All identifiers defined in the present document are assigned a name in the registration domain.

Other specification documents - as referenced by the present document - constitute their own application domain namespaces. Each of them referred to as an application domain for short. Different names may be used for referring to the identifiers defined in the present document, in these application domains.

For each of the identifiers defined in the present document, a clause is provided, which lists the registration domain name, and application domain names used to refer to the respective identifier. This means that all the names listed for each identifier, refer to one and the same identifier. Consequently, all provisions made in the present document for the respective identifier, shall also apply to the application domains listed.



# 5 Service Information (DVB-SI) identifiers

# 5.0 Scope

Clause 5 covers the identifiers defined in ETSI EN 300 468 [i.1].

# 5.1 Bouquet_ID

# 5.1.0 Bouquet_ID registration principles

Bouquet_ID values shall be allocated to broadcasters and network operators to identify bouquets within the application area of ETSI EN 300 468 [i.1], by insertion in the bouquet_id field.

# 5.1.1 Bouquet_ID registration template

To register a Bouquet_ID, applicants shall supply at least the information labelled as "required" in the registration template below.

## Tables

- [Table 1 — Bouquet_ID registration template](tables/1-bouquet-id-registration-template.md)
- [Table 2 — Bouquet_id allocation template](tables/2-bouquet-id-allocation-template.md)
- [Table 3 — Bouquet_ID domain names](tables/3-bouquet-id-domain-names.md)
- [Table 4 — CA_System_ID registration template](tables/4-ca-system-id-registration-template.md)
- [Table 5 — CA_System_ID allocation template](tables/5-ca-system-id-allocation-template.md)
- [Table 6 — CA_System_ID domain names](tables/6-ca-system-id-domain-names.md)
- [Table 7 — CP_System_ID registration template](tables/7-cp-system-id-registration-template.md)
- [Table 8 — CP_System_ID allocation template](tables/8-cp-system-id-allocation-template.md)
- [Table 9 — CP_system_id domain names](tables/9-cp-system-id-domain-names.md)
- [Table 10 — Country Code registration template](tables/10-country-code-registration-template.md)
- [Table 11 — Country Code allocation template](tables/11-country-code-allocation-template.md)
- [Table 12 — Country Code domain names](tables/12-country-code-domain-names.md)
- [Table 13 — Encoding_Type_ID registration template](tables/13-encoding-type-id-registration-template.md)
- [Table 14 — Encoding_Type_ID allocation template](tables/14-encoding-type-id-allocation-template.md)
- [Table 15 — Encoding_type_id domain names](tables/15-encoding-type-id-domain-names.md)
- [Table 16 — Network_ID registration template](tables/16-network-id-registration-template.md)
- [Table 17 — Network_ID allocation template](tables/17-network-id-allocation-template.md)
- [Table 18 — Network_ID domain names](tables/18-network-id-domain-names.md)
- [Table 19 — Original_Network_ID registration template](tables/19-original-network-id-registration-template.md)
- [Table 20 — Original_Network_ID allocation template](tables/20-original-network-id-allocation-template.md)
- [Table 21 — Original_Network_ID domain names](tables/21-original-network-id-domain-names.md)
- [Table 22 — Private_Data_Specifier_ID registration template](tables/22-private-data-specifier-id-registration-template.md)
- [Table 23 — Private_Data_Specifier_ID allocation template](tables/23-private-data-specifier-id-allocation-template.md)
- [Table 24 — Private_data_specifier domain names](tables/24-private-data-specifier-domain-names.md)
- [Table 25 — URI_Linkage_Type registration template](tables/25-uri-linkage-type-registration-template.md)
- [Table 26 — URI_Linkage_Type allocation template](tables/26-uri-linkage-type-allocation-template.md)
- [Table 27 — URI_Linkage_Type domain names](tables/27-uri-linkage-type-domain-names.md)
- [Table 28 — Data_Broadcast_ID registration template](tables/28-data-broadcast-id-registration-template.md)
- [Table 29 — Data_Broadcast_ID allocation template](tables/29-data-broadcast-id-allocation-template.md)
- [Table 30 — Data_Broadcast_ID domain names](tables/30-data-broadcast-id-domain-names.md)
- [Table 31 — Platform_ID registration template](tables/31-platform-id-registration-template.md)
- [Table 32 — Platform_ID allocation template](tables/32-platform-id-allocation-template.md)
- [Table 33 — Platform_ID domain names](tables/33-platform-id-domain-names.md)
- [Table 34 — Protocol_Type_ID registration template](tables/34-protocol-type-id-registration-template.md)
- [Table 35 — Protocol_Type_ID allocation template](tables/35-protocol-type-id-allocation-template.md)
- [Table 36 — Protocol_Type_ID domain names](tables/36-protocol-type-id-domain-names.md)
- [Table 37 — Application_System_ID registration template](tables/37-application-system-id-registration-template.md)
- [Table 38 — Application_System_ID allocation template](tables/38-application-system-id-allocation-template.md)
- [Table 39 — Application_System_ID domain names](tables/39-application-system-id-domain-names.md)
- [Table 40 — MHP_AIT_Descriptor registration template](tables/40-mhp-ait-descriptor-registration-template.md)
- [Table 41 — MHP_AIT_Descriptor allocation template](tables/41-mhp-ait-descriptor-allocation-template.md)
- [Table 42 — MHP_AIT_Descriptor domain names](tables/42-mhp-ait-descriptor-domain-names.md)
- [Table 43 — MHP_Application_Type_ID registration template](tables/43-mhp-application-type-id-registration-template.md)
- [Table 44 — MHP_Application_Type_ID allocation template](tables/44-mhp-application-type-id-allocation-template.md)
- [Table 45 — MHP_Application_Type_ID domain names](tables/45-mhp-application-type-id-domain-names.md)
- [Table 46 — MHP_Organisation_ID registration template](tables/46-mhp-organisation-id-registration-template.md)
- [Table 47 — MHP_Organisation_ID allocation template](tables/47-mhp-organisation-id-allocation-template.md)
- [Table 48 — MHP_Organisation_ID domain names](tables/48-mhp-organisation-id-domain-names.md)
- [Table 49 — MHP_Protocol_ID registration template](tables/49-mhp-protocol-id-registration-template.md)
- [Table 50 — MHP_Protocol_ID allocation template](tables/50-mhp-protocol-id-allocation-template.md)
- [Table 51 — MHP_Protocol_ID domain names](tables/51-mhp-protocol-id-domain-names.md)
- [Table 52 — Payload_ID allocation template](tables/52-payload-id-allocation-template.md)
- [Table 53 — Platform_id domain names](tables/53-platform-id-domain-names.md)
- [Table 54 — IPDC_Operator_ID registration template](tables/54-ipdc-operator-id-registration-template.md)
- [Table 55 — Numerical IPDC_Operator_ID allocation template](tables/55-numerical-ipdc-operator-id-allocation-template.md)
- [Table 56 — IPDC_Operator_ID domain names](tables/56-ipdc-operator-id-domain-names.md)
- [Table 57 — IPDC_Notification_Type registration template](tables/57-ipdc-notification-type-registration-template.md)
- [Table 58 — IPDC_Notification_Type allocation template](tables/58-ipdc-notification-type-allocation-template.md)
- [Table 59 — IPDC_Notification_Type domain names](tables/59-ipdc-notification-type-domain-names.md)
- [Table 60 — Root_of_Trust_ID registration template](tables/60-root-of-trust-id-registration-template.md)
- [Table 61 — Root_of_Trust_ID allocation template](tables/61-root-of-trust-id-allocation-template.md)
- [Table 62 — Root_of_Trust_ID domain names](tables/62-root-of-trust-id-domain-names.md)
- [Table 63 — Metadata Application Format registration template](tables/63-metadata-application-format-registration-template.md)
- [Table 64 — Metadata Application Format allocation template](tables/64-metadata-application-format-allocation-template.md)
- [Table 65 — Metadata Application Format domain names](tables/65-metadata-application-format-domain-names.md)
- [Table 66 — Registration_Authority_ID allocation template](tables/66-registration-authority-id-allocation-template.md)
- [Table 67 — Registration_Authority_ID domain names](tables/67-registration-authority-id-domain-names.md)
- [Table 68 — Private_Resource_Definer_ID registration template](tables/68-private-resource-definer-id-registration-template.md)
- [Table 69 — Private_Resource_Definer_ID allocation template](tables/69-private-resource-definer-id-allocation-template.md)
- [Table 70 — Private_Resource_Definer_ID domain names](tables/70-private-resource-definer-id-domain-names.md)
- [Table 71 — CC_System_ID registration template](tables/71-cc-system-id-registration-template.md)
- [Table 72 — CC_System_ID allocation template](tables/72-cc-system-id-allocation-template.md)
- [Table 73 — CC_System_ID domain names](tables/73-cc-system-id-domain-names.md)
