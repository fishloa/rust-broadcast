# Resource identifier coding + Resource identifier values

_Source: EN 50221 §8.2.2 (Table 15, PDF p. 24) + §8.8.1 (Table 57, PDF p. 54), render-verified_

A resource identifier consists of **4 octets**. The two most significant bits of the
first octet (`resource_id_type`) indicate whether the resource is public or private,
and hence the structure of the rest of the field. Values 0, 1, 2 indicate a public
resource; value 3 indicates a private resource.

## Table 15 — resource_identifier coding (p. 24)

| Syntax | No. of bits | Mnemonic |
|--------|-------------|----------|
| `resource_identifier() {` | | |
| &nbsp;&nbsp;resource_id_type | 2 | uimsbf |
| &nbsp;&nbsp;`if (resource_id_type != 3) {` | | |
| &nbsp;&nbsp;&nbsp;&nbsp;resource_class | 14 | uimsbf |
| &nbsp;&nbsp;&nbsp;&nbsp;resource_type | 10 | uimsbf |
| &nbsp;&nbsp;&nbsp;&nbsp;resource_version | 6 | uimsbf |
| &nbsp;&nbsp;`}` | | |
| &nbsp;&nbsp;`else {` | | |
| &nbsp;&nbsp;&nbsp;&nbsp;private_resource_definer | 10 | uimsbf |
| &nbsp;&nbsp;&nbsp;&nbsp;private_resource_identity | 20 | uimsbf |
| &nbsp;&nbsp;`}` | | |
| `}` | | |

Notes:
- Public resource layout (resource_id_type != 3): 2 + 14 + 10 + 6 = 32 bits.
- Private resource layout (resource_id_type == 3): 2 + 10 + 20 = 32 bits.
- Public resource classes are allocated in the range 1 to 49150, treating
  `resource_id_type` as the most significant part of `resource_class`. Value 0 is
  reserved. The maximum (all-ones) value of all fields is reserved.
- For private resources, each private resource definer can define the structure and
  content of `private_resource_identity` in any way it chooses, except that the
  maximum (all-ones) value is reserved.

## Table 57 — Resource Identifier values (public resources, p. 54)

| Resource | class | type | version | resource identifier (hex) |
|----------|-------|------|---------|---------------------------|
| Resource Manager           | 1  | 1 | 1 | `00010041` |
| Application Information     | 2  | 1 | 1 | `00020041` |
| Conditional Access Support | 3  | 1 | 1 | `00030041` |
| Host Control               | 32 | 1 | 1 | `00200041` |
| Date-Time                  | 36 | 1 | 1 | `00240041` |
| MMI                        | 64 | 1 | 1 | `00400041` |
| Low-Speed Communications   | 96 | see §8.8.1.1 | 1 | `0060xxx1` |
| reserved | other values | other values | other values | other values |

The 32-bit resource identifier is the packed (resource_id_type=0, resource_class,
resource_type, resource_version) per Table 15. E.g. Resource Manager class=1, type=1,
version=1 packs to `0x00010041`.

## Low-Speed Communications resource types (§8.8.1.1, p. 54)

The low-speed communications resource type value encodes two fields. Bits 0 & 1 are a
device number (more than one instance of a particular device may exist). Bits 2-9 are
the device type proper.

Device type field coding:

| Description | Value |
|-------------|-------|
| Modems - see below | `00`-`3F` |
| Serial Ports | `40`-`4F` |
| Cable return channel | `50` |
| reserved | `51`-`FF` |
