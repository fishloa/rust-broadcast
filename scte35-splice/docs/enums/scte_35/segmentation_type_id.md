# segmentation type id

_ANSI/SCTE 35 2023r1 §10.3.3.1 Table 23 — segmentation_type_id values_

> Values rendered from the co-located drift-guard [`segmentation_type_id.toml`](./segmentation_type_id.toml) — the spec defines these inline in a larger syntax table, so there is no standalone table to transcribe. The drift test keeps this list in lockstep with the Rust enum.

| value | variant | spec meaning |
|---|---|---|
| 0x00 | `NotIndicated` | Not Indicated |
| 0x01 | `ContentIdentification` | Content Identification |
| 0x02 | `Private` | Private |
| 0x10 | `ProgramStart` | Program Start |
| 0x11 | `ProgramEnd` | Program End |
| 0x12 | `ProgramEarlyTermination` | Program Early Termination |
| 0x13 | `ProgramBreakaway` | Program Breakaway |
| 0x14 | `ProgramResumption` | Program Resumption |
| 0x15 | `ProgramRunoverPlanned` | Program Runover Planned |
| 0x16 | `ProgramRunoverUnplanned` | Program Runover Unplanned |
| 0x17 | `ProgramOverlapStart` | Program Overlap Start |
| 0x18 | `ProgramBlackoutOverride` | Program Blackout Override |
| 0x19 | `ProgramJoin` | Program Join |
| 0x1A | `ProgramImmediateResumption` | Program Immediate Resumption |
| 0x20 | `ChapterStart` | Chapter Start |
| 0x21 | `ChapterEnd` | Chapter End |
| 0x22 | `BreakStart` | Break Start |
| 0x23 | `BreakEnd` | Break End |
| 0x24 | `OpeningCreditStart` | Opening Credit Start |
| 0x25 | `OpeningCreditEnd` | Opening Credit End |
| 0x26 | `ClosingCreditStart` | Closing Credit Start |
| 0x27 | `ClosingCreditEnd` | Closing Credit End |
| 0x30 | `ProviderAdvertisementStart` | Provider Advertisement Start |
| 0x31 | `ProviderAdvertisementEnd` | Provider Advertisement End |
| 0x32 | `DistributorAdvertisementStart` | Distributor Advertisement Start |
| 0x33 | `DistributorAdvertisementEnd` | Distributor Advertisement End |
| 0x34 | `ProviderPlacementOpportunityStart` | Provider Placement Opportunity Start |
| 0x35 | `ProviderPlacementOpportunityEnd` | Provider Placement Opportunity End |
| 0x36 | `DistributorPlacementOpportunityStart` | Distributor Placement Opportunity Start |
| 0x37 | `DistributorPlacementOpportunityEnd` | Distributor Placement Opportunity End |
| 0x38 | `ProviderOverlayPlacementOpportunityStart` | Provider Overlay Placement Opportunity Start |
| 0x39 | `ProviderOverlayPlacementOpportunityEnd` | Provider Overlay Placement Opportunity End |
| 0x3A | `DistributorOverlayPlacementOpportunityStart` | Distributor Overlay Placement Opportunity Start |
| 0x3B | `DistributorOverlayPlacementOpportunityEnd` | Distributor Overlay Placement Opportunity End |
| 0x3C | `ProviderPromoStart` | Provider Promo Start |
| 0x3D | `ProviderPromoEnd` | Provider Promo End |
| 0x3E | `DistributorPromoStart` | Distributor Promo Start |
| 0x3F | `DistributorPromoEnd` | Distributor Promo End |
| 0x40 | `UnscheduledEventStart` | Unscheduled Event Start |
| 0x41 | `UnscheduledEventEnd` | Unscheduled Event End |
| 0x42 | `AlternateContentOpportunityStart` | Alternate Content Opportunity Start |
| 0x43 | `AlternateContentOpportunityEnd` | Alternate Content Opportunity End |
| 0x44 | `ProviderAdBlockStart` | Provider Ad Block Start |
| 0x45 | `ProviderAdBlockEnd` | Provider Ad Block End |
| 0x46 | `DistributorAdBlockStart` | Distributor Ad Block Start |
| 0x47 | `DistributorAdBlockEnd` | Distributor Ad Block End |
| 0x50 | `NetworkStart` | Network Start |
| 0x51 | `NetworkEnd` | Network End |
