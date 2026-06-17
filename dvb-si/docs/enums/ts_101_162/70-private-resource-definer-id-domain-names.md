# Table 70: Private_Resource_Definer_ID domain names

_Source: specs/etsi_ts_101_162_v01.09.01_dvb_identifiers.pdf (PDF pp.12-44). NB: this is a registration-PROCESS spec (per-identifier registration/allocation templates + domain-name registries); live value allocations are maintained at the DVB online registry, not fixed here._


|  Name | Domain | Description  |
| --- | --- | --- |
|  Private_Resource_Definer_ID | Registration Domain | Constituted by the present document  |
|  private_resource_definer | DVB-CI | ETSI TS 101 699 [i.30]  |



# 13 DVB Extensions to CI Plus™ (DVB-CI-Plus) identifiers

# 13.1 Scope

Clause 13 covers the identifiers defined in ETSI TS 103 205 [i.37].

# 13.2 CC_System_ID

# 13.2.1 Introduction

The CC_System_ID identifies the Content Control system used for content control for a particular instance of the interface. It is defined in CI Plus [i.36] and its usage is clarified in ETSI TS 103 205 [i.37].

Content Control systems often use public-key infrastructures (PKI) to manage authorization, authentication, data integrity, and certificate revocations. If and when this is the case, a registered CC_System_ID value is also implicitly associated with the root-of-trust of the respective PKI.

# 13.2.2 CC_System_ID registration principles

CC_System_ID values shall be allocated only to bona fide organizations. Applicants need to demonstrate that the vendor is proposing a registration for a legitimate content control product.

# 13.2.3 CC_System_ID registration template

To register a CC_System_ID, applicants shall supply at least the information labelled as "required" in the registration template given in table 71.
