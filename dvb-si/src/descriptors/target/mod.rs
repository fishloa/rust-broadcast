//! Target descriptors shared by the INT and the UNT — tags `0x06..=0x0A`.
//!
//! ETSI EN 301 192 V1.7.1 §8.4.5.4-§8.4.5.11 (Tables 22-24, 26, 29) and
//! ETSI TS 102 006 §9.5.2.1-§9.5.2.5 (Tables 20-24) define the same five
//! target descriptors at the same tag values in the shared `0x00..=0x3F`
//! name space ("descriptor tags from 0x00 to 0x3F share a common descriptor
//! name space with UNT descriptors", EN 301 192 §8.4.5.1). The wire layout is
//! identical; only the mask/match field names differ between the two specs
//! (`IPv4_addr` vs `IP_addr_match`), so one Rust type serves both namespaces.

pub mod ip_address;
pub mod ipv6_address;
pub mod mac_address;
pub mod serial_number;
pub mod smartcard;

pub use ip_address::TargetIpAddressDescriptor;
pub use ipv6_address::TargetIpv6AddressDescriptor;
pub use mac_address::TargetMacAddressDescriptor;
pub use serial_number::TargetSerialNumberDescriptor;
pub use smartcard::TargetSmartcardDescriptor;
