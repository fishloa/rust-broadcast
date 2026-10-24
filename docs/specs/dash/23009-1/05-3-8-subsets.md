---
source_pdf: private/specs/iso_iec_23009-1_dash_2022_ed5.pdf
edition: ISO/IEC 23009-1:2022(E), 5th edition
clauses: "Clause 5.3.8 - Subsets"
pdf_pages: "78-79"
printed_pages: "70-71"
verified: true
verification_date: 2026-10-24
verification: see VERIFICATION.md
---

<a id="c5-3-8"></a>

### 5.3.8 Subsets

<a id="c5-3-8-1"></a>

#### 5.3.8.1 Overview

Subsets are described by the Subset element contained in the Period element.

Subsets provide a mechanism to restrict the combination of active Adaptation Sets where an active Adaptation Set is one for which the DASH Client is presenting at least one of the contained Representations.

A Subset defines a set of one or more Adaptation Sets. The presence of a Subset element within a Period element expresses the intention of the creator of the Media Presentation that a client should act as follows: at any time, the set of active Adaptation Sets shall be a subset of the Adaptation Sets of one of the specified Subsets. Any Adaptation Set not explicitly contained in any Subset element is implicitly contained in all specified Subsets.

<!--PB printed 70-->

This implies that

- empty Subsets are not allowed.
- no Subset should contain all the Adaptation Sets.

Each Adaptation Set for which the value of the @id is provided in the @contains attribute is contained in this Subset.

The semantics of the attributes and elements within a Subset are provided in Table 15 of subclause [5.3.8.2.](#c5-3-8-2) The XML syntax of the Subset type is provided in subclause [5.3.8.3.](#c5-3-8-3)

<a id="c5-3-8-2"></a>

#### 5.3.8.2 Semantics

Table 15 — Subset element semantics

| Element or Attribute Name | Use | Description |
|---|---|---|
| **Subset** |  | specifies a Subset |
| `@contains` | M | specifies the Adaptation Sets contained in a Subset by providing a white space separated list of the `@id` values of the contained Adaptation Sets. |
| `@id` | O | specifies a unique identifier for the Subset. |

Key

For attributes: M=mandatory, O=optional, OD=optional with default value, CM=conditionally mandatory

For elements: <minOccurs>..<maxOccurs> (N=unbounded)

Elements are bold; attributes are non-bold and preceded with an @.

<a id="c5-3-8-3"></a>

#### 5.3.8.3 XML syntax

```xml
<xs:complexType name="SubsetType">
    <xs:annotation>
        <xs:documentation xml:lang="en">Subset</xs:documentation>
    </xs:annotation>
    <xs:attribute name="contains" type="UIntVectorType" use="required"/>
    <xs:attribute name="id" type="xs:string"/>
    <xs:anyAttribute namespace="##other" processContents="lax"/>
</xs:complexType>
```
