# Table 54: IPDC_Operator_ID registration template

_Source: specs/etsi_ts_101_162_v01.09.01_dvb_identifiers.pdf (PDF pp.12-44). NB: this is a registration-PROCESS spec (per-identifier registration/allocation templates + domain-name registries); live value allocations are maintained at the DVB online registry, not fixed here._


|  Registration field | Required | Description  |
| --- | --- | --- |
|  IPDC Operator ID Type | required | Type of the IPDC Operator ID to be registered, i.e. string or numerical  |
|  IPDC Operator CA System ID | required | The CA_System_id (see clause 5.2) which has already been registered to "IPDC Operator Name", and under which the "IPDC Operator ID" will be used  |
|  IPDC Operator Name | required | Name of the organization supplying Conditional Access services (e.g. "ACME Mobile Services, Inc.")  |
|  IPDC Operator Legal Contact | required | Name and e-mail of authorized legal signatory of "IPDC Operator Name"  |
|  IPDC Operator Technical Contact | required | Name and e-mail of technical contact of "IPDC Operator Name"  |
|  IPDC Operator Notes | optional | Notes on the application, e.g. last revised and what revisions were made  |
|  NOTE: For historical reasons, the IPDCOperatorId value actually used in IPDC signalling can either be a numerical value or a string value, depending on the CA system with which it is associated (e.g. IPDC SPP Open Security Framework is traditionally associated with IPDCOperatorId numerical values, whereas IPDC SPP 18Crypt is traditionally associated with IPDCOperatorId string values).  |   |   |

When a string ID is to be registered, it shall be a unique text string compliant with one of the two XML built-in data types "string" or "anyURI".

# 10.1.2 IPDC_Operator_ID allocation template

The scheme and values given in table 55 shall be used for the allocation of IPDC_Operator_ID values.
