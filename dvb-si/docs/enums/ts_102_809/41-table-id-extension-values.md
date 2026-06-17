## Table 41 — table_id_extension values
_§9.3.5.2, PDF pp. 66-66_

| table_id_extension | Description |
|---|---|
| 0x0000 to 0x00FF | Authentication message sections as defined in clause 9.4.3. Sections with each value are considered independently for the purposes of maintaining sets of verified hashes. For the number of different sub_tables that a receiver is required to process when implementing a specific profile see clause 9.4.6. In the context of an authentication message the table_id_extension is authentication_group_id of the authentication message. |
| 0x0100 | Certificate collection message as defined in clause 9.5.4.8.5. |
| 0x0101 to 0xFFFF | Reserved for future use |

