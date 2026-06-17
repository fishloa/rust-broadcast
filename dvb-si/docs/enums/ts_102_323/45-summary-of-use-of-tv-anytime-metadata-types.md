## Table 45 — Summary of use of TV-Anytime metadata types
_§8.2, PDF pp. 53-53_

| Fragment | DVB Profile |
|---|---|
| ProgramInformation | Required for support of metadata searching. In addition, a ProgramInformation fragment (with the same CRID) shall be present for each ScheduleEvent element that does not contain an InstanceDescription element. Otherwise optional. |
| GroupInformation | A GroupInformation fragment shall be present for each group CRID that is referenced by other fragments. Otherwise optional. |
| BroadcastEvent | Optional. If provided then there shall be a corresponding PIT entry for the content item. |
| Schedule | Optional. |
| ServiceInformation | A ServiceInformation fragment shall be present for each serviceID referenced by other fragments. Otherwise optional. |
| PersonName (from CreditsInformationTable) | A PersonName fragment shall be present for each person that is referenced from other fragments. Otherwise optional. |
| OrganizationName (from CreditsInformationTable) | An OrganizationName fragment shall be present for each organization that is referenced from other fragments. Otherwise optional. |
| SegmentInformation | Optional. |
| SegmentGroupInformation | Optional. |
| Review (from ProgramReviewTable) | Optional. |
| OnDemandProgram | Optional. |
| OnDemandService | Optional. |
| PushDownloadProgram | Optional. |
| CSAlias | A CSAlias fragment shall be present for each CSAlias that is referenced from other fragments. |
| ClassificationScheme | Mandatory if any classification scheme other than those defined in TS 102 822-3-1 [4] is referenced by any other fragment. Otherwise optional. |
| PurchaseInformation | Optional. |

